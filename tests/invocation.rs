use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn invoke(dir: &Path, program: &str, args: &[&str]) -> Output {
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
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}
fn git(dir: &Path, args: &[&str]) -> String {
    success(invoke(dir, "git", args))
}
fn cli(dir: &Path, args: &[&str]) -> Output {
    invoke(dir, env!("CARGO_BIN_EXE_git-subrepo"), args)
}

struct Fixture {
    root: PathBuf,
    owner: PathBuf,
    upstream: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "subrepo-invocation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let owner = root.join("owner");
        let upstream = root.join("upstream");
        for dir in [&owner, &upstream] {
            std::fs::create_dir_all(dir).unwrap();
            git(dir, &["init", "-q", "-b", "main"]);
            std::fs::write(dir.join("file"), "original\n").unwrap();
            git(dir, &["add", "."]);
            git(dir, &["commit", "-qm", "Baseline"]);
        }
        success(cli(
            &owner,
            &["clone", upstream.to_str().unwrap(), "lib/unit", "--no-edit"],
        ));
        Self {
            root,
            owner,
            upstream,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn config_resolves_dot_parent_absolute_and_cwd_relative_subrepos() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.owner.join("lib/unit/src")).unwrap();
    for (cwd, target) in [
        (f.owner.join("lib"), "unit".to_owned()),
        (f.owner.join("lib/unit"), ".".to_owned()),
        (f.owner.join("lib/unit/src"), "..".to_owned()),
        (
            f.owner.join("lib"),
            f.owner.join("lib/unit").to_string_lossy().into_owned(),
        ),
    ] {
        let output = cli(&cwd, &["config", &target, "exclude", "LOCAL_PATCHES.md"]);
        assert!(String::from_utf8_lossy(&output.stderr).contains("Checkout:"));
        success(output);
        assert_eq!(
            git(
                &f.owner,
                &["config", "--file", "lib/unit/.gitrepo", "subrepo.exclude"]
            ),
            "LOCAL_PATCHES.md"
        );
    }
    assert!(!f.owner.join("lib/.gitrepo").exists());
}

#[test]
fn root_relative_fallback_requires_confirmation_in_scripts() {
    let f = Fixture::new();
    let cwd = f.owner.join("lib/unit");
    let before = std::fs::read(cwd.join(".gitrepo")).unwrap();
    let rejected = cli(&cwd, &["config", "lib/unit", "exclude", "LOCAL_PATCHES.md"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("--yes"));
    assert_eq!(std::fs::read(cwd.join(".gitrepo")).unwrap(), before);
    success(cli(
        &cwd,
        &["config", "lib/unit", "exclude", "LOCAL_PATCHES.md", "--yes"],
    ));
    assert_eq!(
        git(
            &f.owner,
            &["config", "--file", "lib/unit/.gitrepo", "subrepo.exclude"]
        ),
        "LOCAL_PATCHES.md"
    );
}

#[test]
fn cwd_relative_target_wins_when_the_same_name_also_exists_at_root() {
    let f = Fixture::new();
    success(cli(
        &f.owner,
        &["clone", f.upstream.to_str().unwrap(), "unit", "--no-edit"],
    ));
    success(cli(
        &f.owner.join("lib"),
        &["config", "unit", "exclude", "selected"],
    ));
    assert_eq!(
        git(
            &f.owner,
            &["config", "--file", "lib/unit/.gitrepo", "subrepo.exclude"]
        ),
        "selected"
    );
    assert!(
        !invoke(
            &f.owner,
            "git",
            &["config", "--file", "unit/.gitrepo", "subrepo.exclude"]
        )
        .status
        .success()
    );
}

#[test]
fn paths_cannot_escape_the_selected_checkout_or_follow_external_symlinks() {
    let f = Fixture::new();
    for target in ["../../upstream", f.upstream.to_str().unwrap()] {
        let output = cli(
            &f.owner.join("lib"),
            &["config", target, "exclude", "selected", "--yes"],
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("outside the selected checkout"));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&f.upstream, f.owner.join("outside")).unwrap();
        let output = cli(
            &f.owner.join("lib"),
            &["config", "../outside", "exclude", "selected", "--yes"],
        );
        assert!(!output.status.success());
    }
    assert!(!f.upstream.join(".gitrepo").exists());
}

#[test]
fn omitted_targets_infer_the_nearest_subrepo_for_fetch_status_and_pr() {
    let f = Fixture::new();
    let cwd = f.owner.join("lib/unit/src");
    std::fs::create_dir(&cwd).unwrap();
    success(cli(&cwd, &["fetch"]));
    let status = success(cli(&cwd, &["status", "--no-fetch", "--no-dirty"]));
    assert!(status.contains("lib/unit"));
    std::fs::write(f.owner.join("lib/unit/file"), "public edit\n").unwrap();
    git(&f.owner, &["add", "lib/unit/file"]);
    git(&f.owner, &["commit", "-qm", "Edit"]);
    let export = PathBuf::from(success(cli(
        &cwd,
        &[
            "pr",
            "prepare",
            "--branch",
            "contribute/test",
            "--message",
            "Public change",
            "--quiet",
        ],
    )));
    assert_eq!(
        std::fs::read_to_string(export.join("file")).unwrap(),
        "public edit\n"
    );
    assert!(success(cli(&cwd, &["pr", "status"])).contains("contribute/test"));
}

#[test]
fn pull_from_inside_subrepo_preserves_the_callers_directory() {
    let f = Fixture::new();
    let cwd = f.owner.join("lib/unit");
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&cwd).unwrap().ino()
    };
    std::fs::write(f.upstream.join("file"), "upstream edit\n").unwrap();
    git(&f.upstream, &["add", "file"]);
    git(&f.upstream, &["commit", "-qm", "Update"]);
    success(cli(&cwd, &["pull", "--no-edit"]));
    assert_eq!(
        std::fs::read_to_string(cwd.join("file")).unwrap(),
        "upstream edit\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(std::fs::metadata(&cwd).unwrap().ino(), inode);
    }
}

#[test]
fn clone_destinations_and_local_remotes_resolve_from_cwd() {
    let f = Fixture::new();
    let cwd = f.owner.join("lib");
    success(cli(
        &cwd,
        &["clone", "../../upstream", "second", "--no-edit"],
    ));
    assert!(f.owner.join("lib/second/.gitrepo").is_file());
    assert!(!f.owner.join("second").exists());
    success(cli(&cwd, &["clone", "../../upstream", "--no-edit"]));
    assert!(f.owner.join("lib/upstream/.gitrepo").is_file());
}

#[test]
fn init_resolves_new_subrepo_from_the_callers_directory() {
    let f = Fixture::new();
    let target = f.owner.join("lib/new-library");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("file"), "new\n").unwrap();
    git(&f.owner, &["add", "lib/new-library"]);
    git(&f.owner, &["commit", "-qm", "New library"]);
    success(cli(&f.owner.join("lib"), &["init", "new-library"]));
    assert!(target.join(".gitrepo").exists());
}

#[test]
fn config_does_not_create_metadata_for_a_non_subrepo_target() {
    let f = Fixture::new();
    std::fs::create_dir(f.owner.join("lib/plain")).unwrap();
    let output = cli(
        &f.owner.join("lib"),
        &["config", "plain", "exclude", "local"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Not a subrepo"));
    assert!(!f.owner.join("lib/plain/.gitrepo").exists());
}

#[test]
fn pull_refuses_to_replace_the_callers_directory_with_a_file() {
    let f = Fixture::new();
    std::fs::create_dir(f.upstream.join("src")).unwrap();
    std::fs::write(f.upstream.join("src/marker"), "directory\n").unwrap();
    git(&f.upstream, &["add", "src"]);
    git(&f.upstream, &["commit", "-qm", "Add directory"]);
    success(cli(&f.owner, &["pull", "lib/unit", "--no-edit"]));
    let cwd = f.owner.join("lib/unit/src");
    let before = git(&f.owner, &["rev-parse", "HEAD"]);
    git(&f.upstream, &["rm", "-r", "src"]);
    std::fs::write(f.upstream.join("src"), "replacement file\n").unwrap();
    git(&f.upstream, &["add", "src"]);
    git(&f.upstream, &["commit", "-qm", "Replace directory"]);
    let output = cli(&cwd, &["pull", "--no-edit"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("replace your current directory"));
    assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        std::fs::read_to_string(cwd.join("marker")).unwrap(),
        "directory\n"
    );
    assert!(git(&f.owner, &["status", "--porcelain"]).is_empty());
}

#[test]
fn inference_uses_nearest_nested_subrepo_and_all_keeps_checkout_scope() {
    let f = Fixture::new();
    success(cli(
        &f.owner,
        &[
            "clone",
            f.upstream.to_str().unwrap(),
            "lib/unit/nested",
            "--no-edit",
        ],
    ));
    let cwd = f.owner.join("lib/unit/nested/src");
    std::fs::create_dir(&cwd).unwrap();
    let status = success(cli(&cwd, &["status", "--no-fetch", "--no-dirty"]));
    assert!(status.contains("lib/unit/nested"));
    let all = success(cli(&cwd, &["status", "--all", "--no-fetch", "--no-dirty"]));
    assert!(all.contains("lib/unit"));
}
