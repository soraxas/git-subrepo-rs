use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn command(program: &str, dir: &Path, args: &[&str]) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_EDITOR", "true")
        .env_remove("GIT_SUBREPO_WORKON")
        .output()
        .unwrap()
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

fn git(dir: &Path, args: &[&str]) -> String {
    success(command("git", dir, args))
}

fn cli(dir: &Path, args: &[&str]) -> Output {
    command(env!("CARGO_BIN_EXE_git-subrepo"), dir, args)
}

struct Fixture {
    temp: PathBuf,
    main: PathBuf,
    work: PathBuf,
    subdir: String,
}

impl Fixture {
    fn new(subdir: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let temp = std::env::temp_dir().join(format!(
            "subrepo-context-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let main = temp.join("main");
        let upstream = temp.join("upstream");
        for dir in [&main, &upstream] {
            std::fs::create_dir_all(dir).unwrap();
            git(dir, &["init", "-q", "-b", "main"]);
            git(dir, &["config", "commit.gpgsign", "false"]);
        }
        std::fs::create_dir(upstream.join("src")).unwrap();
        std::fs::write(upstream.join("src/file"), "original\n").unwrap();
        git(&upstream, &["add", "."]);
        git(&upstream, &["commit", "-qm", "upstream"]);
        git(&main, &["commit", "--allow-empty", "-qm", "main"]);
        success(cli(
            &main,
            &["clone", upstream.to_str().unwrap(), subdir, "--no-edit"],
        ));
        success(cli(&main, &["workon", subdir, "--no-shell"]));
        let work = main.join(".git/tmp/subrepo").join(subdir);
        Self {
            temp,
            main,
            work,
            subdir: subdir.to_owned(),
        }
    }

    fn edit_work(&self, text: &str) {
        std::fs::write(self.work.join("src/file"), text).unwrap();
        git(&self.work, &["add", "."]);
        git(&self.work, &["commit", "-qm", "workon change"]);
    }

    fn imported(&self, text: &str) {
        assert_eq!(
            std::fs::read_to_string(self.main.join(&self.subdir).join("src/file")).unwrap(),
            text
        );
        assert_eq!(git(&self.main, &["status", "--porcelain"]), "");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

#[test]
fn commit_from_workon_root_and_descendant_imports_and_preserves_active_tree() {
    let f = Fixture::new("lib/space dir");
    for (cwd, text) in [(&f.work, "first\n"), (&f.work.join("src"), "second\n")] {
        f.edit_work(text);
        let before = git(&f.main, &["rev-parse", "HEAD"]);
        let source = git(&f.work, &["rev-parse", "HEAD"]);
        let output = success(cli(cwd, &["commit", "-m", "Import changes"]));
        f.imported(text);
        assert_eq!(git(&f.main, &["rev-parse", "HEAD^"]), before);
        assert_eq!(
            git(&f.main, &["log", "-1", "--format=%s"]),
            "Import changes"
        );
        assert_eq!(git(&f.work, &["rev-parse", "HEAD"]), source);
        assert!(cwd.is_dir());
        assert!(output.contains("lib/space dir"));
        assert!(output.contains(f.main.to_str().unwrap()));
    }
}

#[test]
fn commit_inside_main_subrepo_infers_nearest_subrepo_and_keeps_legacy_cleanup() {
    let f = Fixture::new("lib/unit");
    f.edit_work("imported\n");
    let cwd = f.main.join("lib/unit/src");
    #[cfg(unix)]
    let before = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&cwd).unwrap().ino()
    };
    success(cli(&cwd, &["commit"]));
    f.imported("imported\n");
    assert!(!f.work.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&cwd).unwrap().ino(),
            before,
            "the caller's working directory must not be deleted and recreated"
        );
    }
}

#[test]
fn workon_commit_uses_current_head_after_switching_branches() {
    let f = Fixture::new("lib/unit");
    git(&f.work, &["switch", "-qc", "feature"]);
    f.edit_work("feature\n");
    success(cli(&f.work, &["commit"]));
    f.imported("feature\n");
    assert_eq!(git(&f.work, &["branch", "--show-current"]), "feature");
}

#[test]
fn import_cannot_replace_the_callers_directory_with_a_file() {
    let f = Fixture::new("lib/unit");
    git(&f.work, &["rm", "-r", "src"]);
    std::fs::write(f.work.join("src"), "now a file\n").unwrap();
    git(&f.work, &["add", "src"]);
    git(&f.work, &["commit", "-qm", "replace directory"]);
    let before = git(&f.main, &["rev-parse", "HEAD"]);
    let output = cli(&f.main.join("lib/unit/src"), &["commit"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("current directory"));
    assert_eq!(git(&f.main, &["rev-parse", "HEAD"]), before);
    f.imported("original\n");
}

#[test]
fn workon_commit_refuses_dirty_source_and_destination_without_importing() {
    let f = Fixture::new("lib/unit");
    f.edit_work("committed\n");
    for dirty_dir in [&f.work, &f.main] {
        let before = git(&f.main, &["rev-parse", "HEAD"]);
        let dirty = dirty_dir.join("untracked.txt");
        std::fs::write(&dirty, "keep me").unwrap();
        let output = cli(&f.work, &["commit"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("uncommitted"));
        assert_eq!(git(&f.main, &["rev-parse", "HEAD"]), before);
        assert_eq!(std::fs::read_to_string(&dirty).unwrap(), "keep me");
        assert!(f.work.exists());
        std::fs::remove_file(dirty).unwrap();
    }
    f.imported("original\n");
}

#[test]
fn workon_commit_refuses_ambiguous_owner_checkouts() {
    let f = Fixture::new("lib/unit");
    f.edit_work("committed\n");
    let other = f.temp.join("other checkout");
    git(
        &f.main,
        &["worktree", "add", "-b", "other", other.to_str().unwrap()],
    );
    let before = git(&f.main, &["rev-parse", "HEAD"]);
    let output = cli(&f.work, &["commit"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Multiple"));
    assert_eq!(git(&f.main, &["rev-parse", "HEAD"]), before);
    assert_eq!(git(&other, &["rev-parse", "HEAD"]), before);
    assert!(f.work.exists());
    // A dirty deletion cannot remove a possible owner from consideration.
    std::fs::remove_file(f.main.join("lib/unit/.gitrepo")).unwrap();
    let output = cli(&f.work, &["commit"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Multiple"));
    assert_eq!(git(&other, &["rev-parse", "HEAD"]), before);
}

#[test]
fn inferred_import_from_main_subrepo_refuses_dirty_workon_before_cleanup() {
    let mut f = Fixture::new("lib/unit");
    f.edit_work("committed\n");
    for moved in [false, true] {
        if moved {
            let new_path = f.temp.join("moved source");
            git(
                &f.main,
                &[
                    "worktree",
                    "move",
                    f.work.to_str().unwrap(),
                    new_path.to_str().unwrap(),
                ],
            );
            f.work = new_path;
        }
        std::fs::write(f.work.join("untracked.txt"), "keep me").unwrap();
        let before = git(&f.main, &["rev-parse", "HEAD"]);
        let output = cli(&f.main.join("lib/unit/src"), &["commit"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("uncommitted"));
        assert_eq!(git(&f.main, &["rev-parse", "HEAD"]), before);
        assert_eq!(
            std::fs::read_to_string(f.work.join("untracked.txt")).unwrap(),
            "keep me"
        );
        std::fs::remove_file(f.work.join("untracked.txt")).unwrap();
    }
}

#[test]
fn bare_commit_at_main_root_still_requires_subdir() {
    let f = Fixture::new("lib/unit");
    let output = cli(&f.main, &["commit"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires arg 'subdir'"));
}

#[test]
fn workon_commit_resolves_explicit_head_in_the_source_tree() {
    let f = Fixture::new("lib/unit");
    f.edit_work("source head\n");
    success(cli(&f.work, &["commit", "lib/unit", "HEAD"]));
    f.imported("source head\n");
    assert!(f.work.is_dir());
}

#[test]
fn workon_commit_finds_a_linked_owning_checkout() {
    let f = Fixture::new("lib/unit");
    f.edit_work("linked owner\n");
    let owner = f.temp.join("linked owner");
    git(
        &f.main,
        &["worktree", "add", "-b", "owner", owner.to_str().unwrap()],
    );
    // The primary checkout is on a branch from before the subrepo existed.
    git(&f.main, &["checkout", "--detach", "HEAD^"]);
    let before = git(&f.main, &["rev-parse", "HEAD"]);
    success(cli(&f.work, &["commit"]));
    assert_eq!(git(&f.main, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        std::fs::read_to_string(owner.join("lib/unit/src/file")).unwrap(),
        "linked owner\n"
    );
    assert_eq!(git(&owner, &["status", "--porcelain"]), "");
    assert!(f.work.is_dir());
}

#[test]
fn moved_workon_tree_is_matched_by_its_encoded_branch() {
    let mut f = Fixture::new("lib/space dir");
    let moved = f.temp.join("moved workon");
    git(
        &f.main,
        &[
            "worktree",
            "move",
            f.work.to_str().unwrap(),
            moved.to_str().unwrap(),
        ],
    );
    f.work = moved;
    f.edit_work("moved\n");
    success(cli(&f.work, &["commit"]));
    f.imported("moved\n");
    assert!(f.work.is_dir());
}

#[test]
fn explicit_subdir_in_workon_cannot_target_another_subrepo() {
    let f = Fixture::new("lib/unit");
    f.edit_work("committed\n");
    let before = git(&f.main, &["rev-parse", "HEAD"]);
    let output = cli(&f.work, &["commit", "elsewhere"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("belongs to subrepo 'lib/unit'"));
    assert_eq!(git(&f.main, &["rev-parse", "HEAD"]), before);
    assert!(f.work.is_dir());
}

#[test]
fn workon_commit_rejects_detached_destination_and_preserves_both_heads() {
    let f = Fixture::new("lib/unit");
    f.edit_work("committed\n");
    git(&f.main, &["checkout", "--detach"]);
    let before = git(&f.main, &["rev-parse", "HEAD"]);
    let source = git(&f.work, &["rev-parse", "HEAD"]);
    let output = cli(&f.work, &["commit"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("detached HEAD"));
    assert_eq!(git(&f.main, &["rev-parse", "HEAD"]), before);
    assert_eq!(git(&f.work, &["rev-parse", "HEAD"]), source);
}
