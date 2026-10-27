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
    baseline: String,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "subrepo-exclusions-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let owner = root.join("owner");
        let upstream = root.join("upstream");
        for dir in [&owner, &upstream] {
            std::fs::create_dir_all(dir).unwrap();
            git(dir, &["init", "-q", "-b", "main"]);
            git(dir, &["config", "receive.denyCurrentBranch", "ignore"]);
        }
        std::fs::write(upstream.join("file"), "original\n").unwrap();
        std::fs::write(upstream.join("LOCAL_PATCHES.md"), "upstream ledger\n").unwrap();
        git(&upstream, &["add", "."]);
        git(&upstream, &["commit", "-qm", "Baseline"]);
        let baseline = git(&upstream, &["rev-parse", "HEAD"]);
        std::fs::write(owner.join("owner-file"), "owner\n").unwrap();
        git(&owner, &["add", "."]);
        git(&owner, &["commit", "-qm", "Owner"]);
        success(cli(
            &owner,
            &["clone", upstream.to_str().unwrap(), "lib/unit", "--no-edit"],
        ));
        Self {
            root,
            owner,
            upstream,
            baseline,
        }
    }

    fn exclude(&self, path: &str) {
        git(
            &self.owner,
            &[
                "config",
                "--file",
                "lib/unit/.gitrepo",
                "--add",
                "subrepo.exclude",
                path,
            ],
        );
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.owner.join("lib/unit").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn commit(&self) {
        git(&self.owner, &["add", "lib/unit"]);
        git(&self.owner, &["commit", "-qm", "Library changes"]);
    }

    fn prepare(&self, extra: &[&str]) -> PathBuf {
        let mut args = vec![
            "pr",
            "prepare",
            "lib/unit",
            "--branch",
            "contribute/test",
            "--message",
            "Public changes",
            "--quiet",
        ];
        args.extend(extra);
        PathBuf::from(success(cli(&self.owner, &args)))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn prepare_uses_uncommitted_exclusions_without_committing_or_changing_the_checkout() {
    for staged in [false, true] {
        let f = Fixture::new();
        f.write("file", "public improvement\n");
        f.write("local-only.txt", "owner-only notes\n");
        f.commit();
        f.exclude("local-only.txt");
        if staged {
            git(&f.owner, &["add", "lib/unit/.gitrepo"]);
        }
        let before = git(&f.owner, &["rev-parse", "HEAD"]);
        let status = git(&f.owner, &["status", "--porcelain"]);
        let index = git(&f.owner, &["ls-files", "--stage"]);
        let metadata = std::fs::read(f.owner.join("lib/unit/.gitrepo")).unwrap();
        let work = f.prepare(&[]);
        assert!(!work.join("local-only.txt").exists());
        assert!(!work.join(".gitrepo").exists());
        assert_eq!(
            std::fs::read_to_string(work.join("file")).unwrap(),
            "public improvement\n"
        );
        let session = success(cli(&f.owner, &["pr", "status", "lib/unit"]));
        assert!(session.contains("exclude-0: local-only.txt"), "{session}");
        assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
        assert_eq!(git(&f.owner, &["status", "--porcelain"]), status);
        assert_eq!(git(&f.owner, &["ls-files", "--stage"]), index);
        assert_eq!(
            std::fs::read(f.owner.join("lib/unit/.gitrepo")).unwrap(),
            metadata
        );
    }
}

#[test]
fn uncommitted_nested_metadata_and_similarly_named_files_still_block_preparation() {
    for path in ["vendor/.gitrepo", ".gitrepo.notes"] {
        let f = Fixture::new();
        f.write("file", "public improvement\n");
        f.commit();
        f.exclude("LOCAL_PATCHES.md");
        f.write(path, "uncommitted source\n");
        let output = cli(
            &f.owner,
            &[
                "pr",
                "prepare",
                "lib/unit",
                "--branch",
                "topic",
                "--message",
                "Public",
            ],
        );
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Uncommitted") && error.contains(path),
            "{error}"
        );
    }
}

#[test]
fn configured_exclusions_preserve_upstream_baseline_and_literal_directory_scope() {
    let f = Fixture::new();
    for path in ["LOCAL_PATCHES.md", "local notes/", "literal[1]"] {
        f.exclude(path);
    }
    f.write("LOCAL_PATCHES.md", "private ledger\n");
    f.write("local notes/nested/file", "private notes\n");
    f.write("literal[1]", "private literal\n");
    f.write("literal1", "public literal\n");
    f.write("vendor/LOCAL_PATCHES.md", "public vendor provenance\n");
    f.write("vendor/.gitrepo", "nested metadata\n");
    f.write("file", "public edit\n");
    f.commit();
    let before = git(&f.owner, &["rev-parse", "HEAD"]);
    let work = f.prepare(&[]);
    assert_eq!(
        std::fs::read_to_string(work.join("LOCAL_PATCHES.md")).unwrap(),
        "upstream ledger\n"
    );
    assert!(!work.join("local notes").exists());
    assert!(!work.join("literal[1]").exists());
    assert!(work.join("literal1").exists());
    assert!(work.join("vendor/LOCAL_PATCHES.md").exists());
    assert!(work.join("vendor/.gitrepo").exists());
    assert!(!work.join(".gitrepo").exists());
    assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
    assert!(git(&f.owner, &["status", "--porcelain"]).is_empty());
    let status = success(cli(&f.owner, &["pr", "status", "lib/unit"]));
    assert!(status.contains("LOCAL_PATCHES.md"));
}

#[test]
fn exclusions_win_over_explicit_path_selection() {
    let f = Fixture::new();
    f.exclude("LOCAL_PATCHES.md");
    f.write("LOCAL_PATCHES.md", "private\n");
    f.write("file", "public\n");
    f.commit();
    let work = f.prepare(&["--path", "LOCAL_PATCHES.md", "--path", "file"]);
    assert_eq!(
        std::fs::read_to_string(work.join("LOCAL_PATCHES.md")).unwrap(),
        "upstream ledger\n"
    );
    assert_eq!(
        std::fs::read_to_string(work.join("file")).unwrap(),
        "public\n"
    );
}

#[test]
fn publication_checks_excluded_paths_in_every_outgoing_commit() {
    let f = Fixture::new();
    f.exclude("local.txt");
    f.write("file", "public\n");
    f.commit();
    let work = f.prepare(&[]);
    std::fs::write(work.join("local.txt"), "accidental private data\n").unwrap();
    git(&work, &["add", "local.txt"]);
    git(&work, &["commit", "-qm", "Accidental addition"]);
    git(&work, &["rm", "local.txt"]);
    git(&work, &["commit", "-qm", "Remove addition"]);
    let head = git(&work, &["rev-parse", "HEAD"]);
    let output = cli(
        &f.owner,
        &["pr", "publish", "lib/unit", "--expected-head", &head],
    );
    assert!(
        !output.status.success(),
        "publication accepted excluded intermediate content"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("excluded"));
    assert!(git(&f.upstream, &["branch", "--list", "contribute/test"]).is_empty());
}

#[test]
fn direct_push_filters_every_commit_and_keeps_local_files() {
    let f = Fixture::new();
    f.exclude("LOCAL_PATCHES.md");
    f.exclude("local.txt");
    f.write("LOCAL_PATCHES.md", "private\n");
    f.write("local.txt", "private\n");
    f.write("file", "public one\n");
    f.commit();
    f.write("local.txt", "more private\n");
    f.write("file", "public two\n");
    f.commit();
    success(cli(&f.owner, &["push", "lib/unit"]));
    let range = format!("{}..main", f.baseline);
    for commit in git(&f.upstream, &["rev-list", &range]).lines() {
        assert_eq!(
            git(
                &f.upstream,
                &["show", &format!("{commit}:LOCAL_PATCHES.md")]
            ),
            "upstream ledger"
        );
        assert!(
            !invoke(
                &f.upstream,
                "git",
                &["cat-file", "-e", &format!("{commit}:local.txt")]
            )
            .status
            .success()
        );
        assert!(
            !invoke(
                &f.upstream,
                "git",
                &["cat-file", "-e", &format!("{commit}:.gitrepo")]
            )
            .status
            .success()
        );
    }
    assert_eq!(git(&f.upstream, &["show", "main:file"]), "public two");
    assert_eq!(
        std::fs::read_to_string(f.owner.join("lib/unit/local.txt")).unwrap(),
        "more private\n"
    );
    assert!(git(&f.owner, &["status", "--porcelain"]).is_empty());
    f.write("file", "public three\n");
    f.commit();
    success(cli(&f.owner, &["push", "lib/unit"]));
    assert!(
        !invoke(&f.upstream, "git", &["cat-file", "-e", "main:local.txt"])
            .status
            .success()
    );
}

#[test]
fn pull_preserves_excluded_local_content_and_deletions() {
    for stage in [false, true] {
        let f = Fixture::new();
        f.exclude("LOCAL_PATCHES.md");
        f.exclude("local.txt");
        f.write("local.txt", "retained\n");
        std::fs::remove_file(f.owner.join("lib/unit/LOCAL_PATCHES.md")).unwrap();
        f.commit();
        std::fs::write(f.upstream.join("LOCAL_PATCHES.md"), "upstream changed\n").unwrap();
        std::fs::write(f.upstream.join("local.txt"), "upstream added\n").unwrap();
        std::fs::write(f.upstream.join("file"), "upstream public\n").unwrap();
        git(&f.upstream, &["add", "."]);
        git(&f.upstream, &["commit", "-qm", "Upstream changes"]);
        let mut args = vec!["pull", "lib/unit", "--no-edit"];
        if stage {
            args.push("--stage-only");
        }
        success(cli(&f.owner, &args));
        assert!(!f.owner.join("lib/unit/LOCAL_PATCHES.md").exists());
        assert_eq!(
            std::fs::read_to_string(f.owner.join("lib/unit/local.txt")).unwrap(),
            "retained\n"
        );
        assert_eq!(
            std::fs::read_to_string(f.owner.join("lib/unit/file")).unwrap(),
            "upstream public\n"
        );
        assert_eq!(
            git(
                &f.owner,
                &[
                    "config",
                    "--file",
                    "lib/unit/.gitrepo",
                    "--get-all",
                    "subrepo.exclude"
                ]
            ),
            "LOCAL_PATCHES.md\nlocal.txt"
        );
    }
}

#[test]
fn invalid_exclusions_fail_before_preparing_or_pushing() {
    for path in [
        "",
        "../owner-file",
        "/tmp/private",
        ".",
        "local/../file",
        ".git/config",
        ":(exclude)file",
    ] {
        let f = Fixture::new();
        f.exclude(path);
        f.write("file", "public\n");
        f.commit();
        let before = git(&f.owner, &["rev-parse", "HEAD"]);
        for args in [
            vec![
                "pr",
                "prepare",
                "lib/unit",
                "--branch",
                "contribute/test",
                "--message",
                "Public",
            ],
            vec!["push", "lib/unit"],
        ] {
            let output = cli(&f.owner, &args);
            assert!(
                !output.status.success(),
                "accepted invalid exclusion {path:?}"
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("exclu"));
        }
        assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
        assert_eq!(git(&f.upstream, &["rev-parse", "HEAD"]), f.baseline);
    }
}

#[test]
fn accepted_pr_retains_excluded_files_when_upstream_changes_them() {
    let f = Fixture::new();
    f.exclude("LOCAL_PATCHES.md");
    f.exclude("local.txt");
    f.write("LOCAL_PATCHES.md", "local ledger\n");
    f.write("local.txt", "retained locally\n");
    f.write("file", "public contribution\n");
    f.commit();
    let work = f.prepare(&[]);
    let head = git(&work, &["rev-parse", "HEAD"]);
    success(cli(
        &f.owner,
        &["pr", "publish", "lib/unit", "--expected-head", &head],
    ));
    std::fs::write(f.upstream.join("LOCAL_PATCHES.md"), "new upstream ledger\n").unwrap();
    std::fs::write(f.upstream.join("local.txt"), "new upstream file\n").unwrap();
    git(&f.upstream, &["add", "."]);
    git(&f.upstream, &["commit", "-qm", "Upstream ledger update"]);
    git(
        &f.upstream,
        &[
            "merge",
            "--no-ff",
            "-m",
            "Accept contribution",
            "contribute/test",
        ],
    );
    let accepted = git(&f.upstream, &["rev-parse", "HEAD"]);
    success(cli(
        &f.owner,
        &["pr", "reconcile", "lib/unit", "--merged", &accepted],
    ));
    assert_eq!(
        std::fs::read_to_string(f.owner.join("lib/unit/LOCAL_PATCHES.md")).unwrap(),
        "local ledger\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.owner.join("lib/unit/local.txt")).unwrap(),
        "retained locally\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.owner.join("lib/unit/file")).unwrap(),
        "public contribution\n"
    );
}

#[test]
fn first_push_after_init_filters_the_entire_outgoing_history() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.owner.join("new-library")).unwrap();
    std::fs::write(f.owner.join("new-library/file"), "public\n").unwrap();
    std::fs::write(f.owner.join("new-library/local.txt"), "local\n").unwrap();
    git(&f.owner, &["add", "new-library"]);
    git(&f.owner, &["commit", "-qm", "New library"]);
    let destination = f.root.join("new.git");
    git(
        &f.root,
        &["init", "--bare", "-q", destination.to_str().unwrap()],
    );
    success(cli(
        &f.owner,
        &[
            "init",
            "new-library",
            "--remote",
            destination.to_str().unwrap(),
            "--branch",
            "main",
        ],
    ));
    git(
        &f.owner,
        &[
            "config",
            "--file",
            "new-library/.gitrepo",
            "subrepo.exclude",
            "local.txt",
        ],
    );
    git(&f.owner, &["add", "new-library/.gitrepo"]);
    git(&f.owner, &["commit", "-qm", "Configure export"]);
    success(cli(&f.owner, &["push", "new-library"]));
    for commit in git(&destination, &["rev-list", "main"]).lines() {
        assert!(
            !invoke(
                &destination,
                "git",
                &["cat-file", "-e", &format!("{commit}:local.txt")]
            )
            .status
            .success()
        );
    }
    assert!(f.owner.join("new-library/local.txt").exists());
}

#[test]
fn config_reads_all_exclusions_and_can_replace_them_without_force() {
    let f = Fixture::new();
    f.exclude("one");
    f.exclude("two");
    let read = success(cli(&f.owner, &["config", "lib/unit", "exclude"]));
    assert!(read.contains("one") && read.contains("two"));
    success(cli(
        &f.owner,
        &["config", "lib/unit", "exclude", "replacement/"],
    ));
    assert_eq!(
        git(
            &f.owner,
            &[
                "config",
                "--file",
                "lib/unit/.gitrepo",
                "--get-all",
                "subrepo.exclude"
            ]
        ),
        "replacement/"
    );
    let invalid = cli(&f.owner, &["config", "lib/unit", "exclude", "../outside"]);
    assert!(!invalid.status.success());
    assert_eq!(
        git(
            &f.owner,
            &[
                "config",
                "--file",
                "lib/unit/.gitrepo",
                "--get-all",
                "subrepo.exclude"
            ]
        ),
        "replacement/"
    );
}

#[test]
fn exclusion_restores_binary_content_modes_and_deleted_directories() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.upstream.join("retained")).unwrap();
    std::fs::write(f.upstream.join("retained/blob"), [0, 255, 10, 128]).unwrap();
    git(&f.upstream, &["add", "."]);
    git(
        &f.upstream,
        &["update-index", "--chmod=+x", "retained/blob"],
    );
    git(&f.upstream, &["commit", "-qm", "Binary baseline"]);
    success(cli(&f.owner, &["pull", "lib/unit", "--no-edit"]));
    f.exclude("retained/");
    std::fs::remove_dir_all(f.owner.join("lib/unit/retained")).unwrap();
    f.write("file", "public\n");
    f.commit();
    let work = f.prepare(&[]);
    assert_eq!(
        std::fs::read(work.join("retained/blob")).unwrap(),
        [0, 255, 10, 128]
    );
    assert!(git(&work, &["ls-tree", "HEAD", "retained/blob"]).starts_with("100755"));
    assert!(!f.owner.join("lib/unit/retained").exists());
}
