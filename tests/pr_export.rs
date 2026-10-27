use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn command(program: &str, dir: &Path, args: &[&str]) -> Output {
    invocation(program, dir, args).output().unwrap()
}

fn invocation(program: &str, dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_EDITOR", "true")
        .env_remove("GIT_SUBREPO_WORKON");
    command
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
    success(command("git", dir, args))
}

fn cli(dir: &Path, args: &[&str]) -> Output {
    command(env!("CARGO_BIN_EXE_git-subrepo"), dir, args)
}

struct Fixture {
    temp: PathBuf,
    owner: PathBuf,
    upstream: PathBuf,
    remote: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let temp = std::env::temp_dir().join(format!(
            "subrepo-pr-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let owner = temp.join("owner");
        let upstream = temp.join("upstream");
        let remote = temp.join("remote.git");
        for dir in [&owner, &upstream] {
            std::fs::create_dir_all(dir).unwrap();
            git(dir, &["init", "-q", "-b", "main"]);
        }
        std::fs::write(upstream.join("file"), "original\n").unwrap();
        git(&upstream, &["add", "."]);
        git(&upstream, &["commit", "-qm", "Upstream baseline"]);
        git(
            &temp,
            &[
                "clone",
                "--bare",
                upstream.to_str().unwrap(),
                remote.to_str().unwrap(),
            ],
        );
        git(
            &upstream,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        std::fs::write(owner.join("private"), "private application\n").unwrap();
        git(&owner, &["add", "."]);
        git(&owner, &["commit", "-qm", "Private product setup"]);
        success(cli(
            &owner,
            &["clone", remote.to_str().unwrap(), "lib/unit", "--no-edit"],
        ));
        Self {
            temp,
            owner,
            upstream,
            remote,
        }
    }

    fn edit(&self, path: &str, text: &str) {
        let path = self.owner.join("lib/unit").join(path);
        std::fs::write(path, text).unwrap();
        git(&self.owner, &["add", "lib/unit"]);
        git(
            &self.owner,
            &["commit", "-qm", "Private product implementation"],
        );
    }

    fn prepare(&self, branch: &str, extra: &[&str]) -> PathBuf {
        let mut args = vec![
            "pr",
            "prepare",
            "lib/unit",
            "--branch",
            branch,
            "--message",
            "Improve shared library",
            "--quiet",
        ];
        args.extend(extra);
        PathBuf::from(success(cli(&self.owner, &args)))
    }

    fn publish(&self, work: &Path) -> String {
        let head = git(work, &["rev-parse", "HEAD"]);
        success(cli(
            &self.owner,
            &["pr", "publish", "lib/unit", "--expected-head", &head],
        ));
        head
    }

    fn merge(&self, branch: &str, squash: bool) -> String {
        git(&self.upstream, &["fetch", "origin", branch]);
        if squash {
            git(&self.upstream, &["merge", "--squash", "FETCH_HEAD"]);
            git(&self.upstream, &["commit", "-qm", "Accepted contribution"]);
        } else {
            git(
                &self.upstream,
                &["merge", "--no-ff", "-m", "Merge contribution", "FETCH_HEAD"],
            );
        }
        git(&self.upstream, &["push", "origin", "main"]);
        git(&self.upstream, &["rev-parse", "HEAD"])
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

#[test]
fn dirty_prepare_reports_files_and_existing_export_in_another_checkout() {
    let f = Fixture::new();
    f.edit("file", "export\n");
    let export = f.prepare("contribute/resume", &[]);
    let sibling = f.temp.join("dirty checkout");
    git(
        &f.owner,
        &["worktree", "add", "-b", "later", sibling.to_str().unwrap()],
    );
    std::fs::write(sibling.join("lib/unit/file"), "unfinished\n").unwrap();
    let before = git(&sibling, &["status", "--porcelain"]);
    let output = cli(&sibling, &["pr", "prepare", "lib/unit"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    for expected in [
        sibling.to_str().unwrap(),
        "lib/unit/file",
        "contribute/resume",
        export.to_str().unwrap(),
        f.owner.to_str().unwrap(),
    ] {
        assert!(error.contains(expected), "missing {expected:?}: {error}");
    }
    assert_eq!(git(&sibling, &["status", "--porcelain"]), before);
    assert!(git(&export, &["status", "--porcelain"]).is_empty());
}

#[test]
fn equivalent_subrepo_paths_share_the_session_and_lock() {
    let f = Fixture::new();
    f.edit("file", "export\n");
    f.prepare("contribute/canonical", &[]);
    let original = success(cli(&f.owner, &["pr", "status", "lib/unit"]));
    for alias in ["lib/unit/", "lib//unit", "lib/./unit"] {
        assert_eq!(success(cli(&f.owner, &["pr", "status", alias])), original);
        let output = cli(
            &f.owner,
            &[
                "pr",
                "prepare",
                alias,
                "--branch",
                "contribute/duplicate",
                "--message",
                "Duplicate",
            ],
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("A PR session is active"));
    }
}

#[test]
fn prepare_exports_only_public_history_without_changing_owner_or_sync_state() {
    let f = Fixture::new();
    f.edit("file", "shared improvement\n");
    let before = git(&f.owner, &["rev-parse", "HEAD"]);
    let metadata = std::fs::read(f.owner.join("lib/unit/.gitrepo")).unwrap();
    let work = f.prepare("contribute/viewer", &[]);
    let status = success(cli(&f.owner, &["pr", "status", "lib/unit"]));
    assert!(
        status.contains("created-at: ") && status.contains(" UTC"),
        "{status}"
    );
    assert!(status.contains("source-branch: main"), "{status}");
    assert_eq!(
        std::fs::read_to_string(work.join("file")).unwrap(),
        "shared improvement\n"
    );
    assert!(!work.join(".gitrepo").exists());
    assert!(!work.join("private").exists());
    let history = git(&work, &["log", "--format=%B"]);
    assert!(!history.contains("Private product"), "{history}");
    assert!(history.contains("Improve shared library"));
    assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
    assert_eq!(git(&f.owner, &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read(f.owner.join("lib/unit/.gitrepo")).unwrap(),
        metadata
    );
    assert!(
        !command(
            "git",
            &f.remote,
            &["show-ref", "--verify", "refs/heads/contribute/viewer"]
        )
        .status
        .success()
    );
}

#[test]
fn publish_requires_current_reviewed_head_and_appends_review_fixes_without_repinning() {
    let f = Fixture::new();
    f.edit("file", "initial\n");
    let work = f.prepare("contribute/review", &[]);
    let first = f.publish(&work);
    let status = success(cli(&f.owner, &["pr", "status", "lib/unit"]));
    assert!(
        status.contains("published-at: ") && status.contains(" UTC"),
        "{status}"
    );
    let record = work
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("session");
    git(
        &f.owner,
        &[
            "config",
            "--file",
            record.to_str().unwrap(),
            "export.published-at",
            "2026-10-14 08:45:00 UTC",
        ],
    );
    f.publish(&work);
    assert_eq!(
        git(
            &f.owner,
            &[
                "config",
                "--file",
                record.to_str().unwrap(),
                "export.published-at"
            ]
        ),
        "2026-10-14 08:45:00 UTC"
    );
    let before = git(&f.owner, &["rev-parse", "HEAD"]);
    std::fs::write(work.join("file"), "reviewed\n").unwrap();
    git(&work, &["commit", "-qam", "Fix review feedback"]);
    assert!(
        !cli(
            &f.owner,
            &["pr", "publish", "lib/unit", "--expected-head", &first]
        )
        .status
        .success()
    );
    assert_eq!(git(&f.remote, &["rev-parse", "contribute/review"]), first);
    let second = f.publish(&work);
    assert_ne!(
        git(
            &f.owner,
            &[
                "config",
                "--file",
                record.to_str().unwrap(),
                "export.published-at"
            ]
        ),
        "2026-10-14 08:45:00 UTC"
    );
    assert_eq!(git(&f.remote, &["rev-parse", "contribute/review"]), second);
    assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        git(
            &f.owner,
            &["config", "--file", "lib/unit/.gitrepo", "subrepo.branch"]
        ),
        "main"
    );
    let status = success(cli(&f.owner, &["pr", "status", "lib/unit"]));
    assert!(status.contains(&second));
}

#[test]
fn merge_and_squash_roundtrips_keep_review_fixes_and_unexported_local_work() {
    for squash in [false, true] {
        let f = Fixture::new();
        f.edit("file", "initial\n");
        f.edit("local-only", "not part of this PR\n");
        let work = f.prepare("contribute/first", &["--path", "file"]);
        assert!(!work.join("local-only").exists());
        f.publish(&work);
        std::fs::write(work.join("file"), "reviewed\n").unwrap();
        git(&work, &["commit", "-qam", "Address review"]);
        f.publish(&work);
        f.edit("later", "continued development\n");
        std::fs::write(f.upstream.join("upstream-only"), "upstream addition\n").unwrap();
        git(&f.upstream, &["add", "."]);
        git(&f.upstream, &["commit", "-qm", "Concurrent upstream work"]);
        git(&f.upstream, &["push", "origin", "main"]);
        let merged = f.merge("contribute/first", squash);
        let before = git(&f.owner, &["rev-parse", "HEAD"]);
        success(cli(
            &f.owner,
            &["pr", "reconcile", "lib/unit", "--merged", &merged],
        ));
        assert_eq!(
            git(&f.owner, &["rev-parse", "HEAD"]),
            before,
            "reconciliation stages, never commits implicitly"
        );
        for (name, contents) in [
            ("file", "reviewed\n"),
            ("local-only", "not part of this PR\n"),
            ("later", "continued development\n"),
            ("upstream-only", "upstream addition\n"),
        ] {
            assert_eq!(
                std::fs::read_to_string(f.owner.join("lib/unit").join(name)).unwrap(),
                contents
            );
        }
        assert_eq!(
            git(
                &f.owner,
                &["config", "--file", "lib/unit/.gitrepo", "subrepo.commit"]
            ),
            merged
        );
        git(&f.owner, &["commit", "-qm", "Import accepted contribution"]);
        let second = f.prepare("contribute/second", &[]);
        let diff = git(
            &second,
            &["diff", "--name-only", &format!("{merged}..HEAD")],
        );
        assert_eq!(diff.lines().collect::<Vec<_>>(), ["later", "local-only"]);
        f.publish(&second);
        let merged_second = f.merge("contribute/second", squash);
        success(cli(
            &f.owner,
            &["pr", "reconcile", "lib/unit", "--merged", &merged_second],
        ));
        git(&f.owner, &["commit", "-qm", "Import second contribution"]);
        assert_eq!(git(&f.owner, &["status", "--porcelain"]), "");
    }
}

#[test]
fn reconciliation_accepts_uncommitted_metadata_and_preserves_current_settings() {
    for staged in [false, true] {
        let f = Fixture::new();
        f.edit("file", "public change\n");
        let work = f.prepare("contribute/metadata", &[]);
        std::fs::write(work.join("file"), "reviewed upstream change\n").unwrap();
        git(&work, &["commit", "-qam", "Review fix"]);
        f.publish(&work);
        let merged = f.merge("contribute/metadata", false);
        git(
            &f.owner,
            &[
                "config",
                "--file",
                "lib/unit/.gitrepo",
                "subrepo.exclude",
                "LOCAL_PATCHES.md",
            ],
        );
        if staged {
            git(&f.owner, &["add", "lib/unit/.gitrepo"]);
            // The working policy, rather than the staged policy, must survive.
            git(
                &f.owner,
                &[
                    "config",
                    "--file",
                    "lib/unit/.gitrepo",
                    "--add",
                    "subrepo.exclude",
                    "local notes/",
                ],
            );
        }
        let before = git(&f.owner, &["rev-parse", "HEAD"]);
        success(cli(
            &f.owner,
            &["pr", "reconcile", "lib/unit", "--merged", &merged],
        ));
        assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
        assert_eq!(
            git(
                &f.owner,
                &["config", "--file", "lib/unit/.gitrepo", "subrepo.commit"]
            ),
            merged
        );
        let policy = git(
            &f.owner,
            &[
                "config",
                "--file",
                "lib/unit/.gitrepo",
                "--get-all",
                "subrepo.exclude",
            ],
        );
        assert_eq!(
            policy,
            if staged {
                "LOCAL_PATCHES.md\nlocal notes/"
            } else {
                "LOCAL_PATCHES.md"
            }
        );
        assert!(git(&f.owner, &["diff", "--", "lib/unit/.gitrepo"]).is_empty());
        assert_eq!(
            std::fs::read_to_string(f.owner.join("lib/unit/file")).unwrap(),
            "reviewed upstream change\n"
        );
        assert_eq!(
            git(&f.owner, &["diff", "--cached", "--name-only"]),
            "lib/unit/.gitrepo\nlib/unit/file"
        );
    }
}

#[test]
fn reconciliation_still_rejects_dirty_source_and_changed_sync_baselines() {
    for path in [
        "private",
        "private-index",
        "lib/unit/file",
        "lib/unit/vendor/.gitrepo",
        "lib/unit/.gitrepo",
    ] {
        let f = Fixture::new();
        f.edit("file", "public change\n");
        let work = f.prepare("contribute/guard", &[]);
        f.publish(&work);
        let merged = f.merge("contribute/guard", false);
        git(
            &f.owner,
            &[
                "config",
                "--file",
                "lib/unit/.gitrepo",
                "subrepo.exclude",
                "LOCAL_PATCHES.md",
            ],
        );
        if path == "lib/unit/.gitrepo" {
            git(
                &f.owner,
                &["config", "--file", path, "subrepo.commit", &merged],
            );
        } else {
            let file = f.owner.join(if path == "private-index" {
                "private"
            } else {
                path
            });
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "uncommitted work\n").unwrap();
            if path == "private-index" {
                git(&f.owner, &["add", "private"]);
                std::fs::write(file, "private application\n").unwrap();
            }
        }
        let before = git(&f.owner, &["status", "--porcelain"]);
        let metadata = std::fs::read(f.owner.join("lib/unit/.gitrepo")).unwrap();
        let output = cli(
            &f.owner,
            &["pr", "reconcile", "lib/unit", "--merged", &merged],
        );
        assert!(!output.status.success(), "accepted dirty path {path}");
        assert_eq!(git(&f.owner, &["status", "--porcelain"]), before);
        assert_eq!(
            std::fs::read(f.owner.join("lib/unit/.gitrepo")).unwrap(),
            metadata
        );
        if path == "lib/unit/.gitrepo" {
            assert!(String::from_utf8_lossy(&output.stderr).contains("sync baseline changed"));
        }
    }
}

#[test]
fn prepare_refuses_base_branch_and_dirty_subtree() {
    let f = Fixture::new();
    f.edit("file", "changed\n");
    let rejected = cli(
        &f.owner,
        &[
            "pr",
            "prepare",
            "lib/unit",
            "--branch",
            "main",
            "--message",
            "Public",
        ],
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("must differ"));
    std::fs::write(f.owner.join("lib/unit/file"), "uncommitted\n").unwrap();
    let rejected = cli(
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
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Uncommitted"));
}

#[test]
fn preparation_keeps_new_upstream_files_binary_content_and_literal_path_scope() {
    let f = Fixture::new();
    f.edit("space name", "selected\n");
    f.edit("excluded", "local\n");
    let binary = [0, 255, 128, 10, 0, 13];
    std::fs::write(f.owner.join("lib/unit/binary"), binary).unwrap();
    git(&f.owner, &["add", "lib/unit/binary"]);
    git(&f.owner, &["commit", "-qm", "Private binary change"]);
    std::fs::write(f.upstream.join("concurrent"), "new upstream content\n").unwrap();
    git(&f.upstream, &["add", "."]);
    git(
        &f.upstream,
        &["commit", "-qm", "Upstream changes before export"],
    );
    git(&f.upstream, &["push", "origin", "main"]);
    let work = f.prepare(
        "contribute/binary",
        &["--path", "space name", "--path", "binary"],
    );
    assert_eq!(std::fs::read(work.join("binary")).unwrap(), binary);
    assert!(work.join("concurrent").is_file());
    assert!(work.join("space name").is_file());
    assert!(!work.join("excluded").exists());
}

#[test]
fn reconciliation_conflicts_do_not_touch_owner_and_can_be_resolved_separately() {
    let f = Fixture::new();
    f.edit("file", "initial\n");
    let work = f.prepare("contribute/conflict", &[]);
    std::fs::write(work.join("file"), "upstream review fix\n").unwrap();
    git(&work, &["commit", "-qam", "Fix review"]);
    f.publish(&work);
    f.edit("file", "newer local change\n");
    let merged = f.merge("contribute/conflict", true);
    let before = git(&f.owner, &["rev-parse", "HEAD"]);
    let error = cli(
        &f.owner,
        &["pr", "reconcile", "lib/unit", "--merged", &merged],
    );
    assert!(!error.status.success());
    assert!(String::from_utf8_lossy(&error.stderr).contains("conflict resolution"));
    assert_eq!(git(&f.owner, &["status", "--porcelain"]), "");
    assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
    let status = success(cli(&f.owner, &["pr", "status", "lib/unit"]));
    let integration = PathBuf::from(
        status
            .lines()
            .find_map(|line| line.strip_prefix("integration: "))
            .unwrap(),
    );
    std::fs::write(integration.join("file"), "review fix plus local change\n").unwrap();
    git(&integration, &["add", "file"]);
    success(cli(
        &f.owner,
        &["pr", "reconcile", "lib/unit", "--merged", &merged],
    ));
    assert_eq!(
        std::fs::read_to_string(f.owner.join("lib/unit/file")).unwrap(),
        "review fix plus local change\n"
    );
    assert_eq!(git(&f.owner, &["rev-parse", "HEAD"]), before);
}

#[test]
fn reconciliation_rejects_an_unrelated_accepted_commit() {
    let f = Fixture::new();
    f.edit("file", "proposed\n");
    let work = f.prepare("contribute/unmerged", &[]);
    f.publish(&work);
    std::fs::write(f.upstream.join("unrelated"), "not this PR\n").unwrap();
    git(&f.upstream, &["add", "."]);
    git(&f.upstream, &["commit", "-qm", "Unrelated upstream commit"]);
    git(&f.upstream, &["push", "origin", "main"]);
    let unrelated = git(&f.upstream, &["rev-parse", "HEAD"]);
    let error = cli(
        &f.owner,
        &["pr", "reconcile", "lib/unit", "--merged", &unrelated],
    );
    assert!(!error.status.success());
    assert!(String::from_utf8_lossy(&error.stderr).contains("does not match"));
    assert_eq!(git(&f.owner, &["status", "--porcelain"]), "");
}

#[test]
fn publication_rejects_metadata_in_intermediate_commits_even_when_removed_at_tip() {
    let f = Fixture::new();
    f.edit("file", "proposed\n");
    let work = f.prepare("contribute/history", &[]);
    std::fs::write(work.join(".gitrepo"), "private metadata\n").unwrap();
    git(&work, &["add", ".gitrepo"]);
    git(&work, &["commit", "-qm", "Accidental metadata"]);
    git(&work, &["rm", ".gitrepo"]);
    git(&work, &["commit", "-qm", "Remove metadata"]);
    let head = git(&work, &["rev-parse", "HEAD"]);
    let error = cli(
        &f.owner,
        &["pr", "publish", "lib/unit", "--expected-head", &head],
    );
    assert!(!error.status.success());
    assert!(String::from_utf8_lossy(&error.stderr).contains("root .gitrepo"));
    assert!(
        !command(
            "git",
            &f.remote,
            &["show-ref", "--verify", "refs/heads/contribute/history"]
        )
        .status
        .success()
    );
}

#[test]
fn nested_subrepo_names_do_not_collide_with_parent_session_metadata() {
    let f = Fixture::new();
    f.edit("file", "parent change\n");
    let parent = f.prepare("contribute/parent", &[]);
    success(cli(
        &f.owner,
        &[
            "clone",
            f.remote.to_str().unwrap(),
            "lib/unit/session",
            "--no-edit",
        ],
    ));
    std::fs::write(f.owner.join("lib/unit/session/file"), "nested change\n").unwrap();
    git(&f.owner, &["add", "lib/unit/session/file"]);
    git(&f.owner, &["commit", "-qm", "Nested work"]);
    let nested = PathBuf::from(success(cli(
        &f.owner,
        &[
            "pr",
            "prepare",
            "lib/unit/session",
            "--branch",
            "contribute/nested",
            "--message",
            "Nested contribution",
            "--quiet",
        ],
    )));
    assert!(parent.join("file").is_file());
    assert_eq!(
        std::fs::read_to_string(nested.join("file")).unwrap(),
        "nested change\n"
    );
    assert!(
        success(cli(&f.owner, &["pr", "status", "lib/unit"])).contains(parent.to_str().unwrap())
    );
}

#[test]
fn closing_and_replacing_a_session_retains_its_snapshot_record() {
    let f = Fixture::new();
    f.edit("file", "first\n");
    let first = f.prepare("contribute/first", &[]);
    success(cli(&f.owner, &["pr", "close", "lib/unit"]));
    f.edit("file", "second\n");
    let second = f.prepare("contribute/second", &[]);
    assert_ne!(first, second);
    let record = first.parent().unwrap().join("session");
    assert_eq!(
        git(
            &f.owner,
            &["config", "--file", record.to_str().unwrap(), "export.phase"]
        ),
        "closed"
    );
    assert_eq!(
        git(
            &f.owner,
            &[
                "config",
                "--file",
                record.to_str().unwrap(),
                "export.branch"
            ]
        ),
        "contribute/first"
    );
}

#[test]
fn owner_garbage_collection_does_not_break_open_pr_reconciliation() {
    let f = Fixture::new();
    f.edit("file", "exported\n");
    std::fs::write(f.upstream.join("new-upstream"), "upstream\n").unwrap();
    git(&f.upstream, &["add", "."]);
    git(&f.upstream, &["commit", "-qm", "Advance upstream"]);
    git(&f.upstream, &["push", "origin", "main"]);
    let work = f.prepare("contribute/gc", &[]);
    f.publish(&work);
    let merged = f.merge("contribute/gc", true);
    git(&f.owner, &["reflog", "expire", "--expire=now", "--all"]);
    git(&f.owner, &["gc", "--prune=now"]);
    success(cli(
        &f.owner,
        &["pr", "reconcile", "lib/unit", "--merged", &merged],
    ));
    assert_eq!(
        std::fs::read_to_string(f.owner.join("lib/unit/file")).unwrap(),
        "exported\n"
    );
}

#[test]
fn reviewed_external_commits_can_be_followed_by_local_review_fixes() {
    let f = Fixture::new();
    f.edit("file", "exported\n");
    let work = f.prepare("contribute/external-review", &[]);
    f.publish(&work);
    git(
        &f.upstream,
        &["fetch", "origin", "contribute/external-review"],
    );
    git(&f.upstream, &["switch", "-c", "review", "FETCH_HEAD"]);
    std::fs::write(f.upstream.join("reviewer"), "external review fix\n").unwrap();
    git(&f.upstream, &["add", "."]);
    git(&f.upstream, &["commit", "-qm", "Reviewer fix"]);
    git(
        &f.upstream,
        &["push", "origin", "HEAD:contribute/external-review"],
    );
    git(&work, &["fetch", "publish", "contribute/external-review"]);
    git(&work, &["merge", "--ff-only", "FETCH_HEAD"]);
    std::fs::write(work.join("local-fix"), "local review fix\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "Local review fix"]);
    let tip = f.publish(&work);
    assert_eq!(
        git(&f.remote, &["rev-parse", "contribute/external-review"]),
        tip
    );
    assert_eq!(
        git(&f.remote, &["show", "contribute/external-review:reviewer"]),
        "external review fix"
    );
}

#[test]
fn literal_subrepo_path_does_not_remove_matching_sibling_files() {
    let f = Fixture::new();
    success(cli(
        &f.owner,
        &["clone", f.remote.to_str().unwrap(), "vendor/*", "--no-edit"],
    ));
    std::fs::create_dir_all(f.owner.join("vendor/sibling")).unwrap();
    std::fs::write(f.owner.join("vendor/sibling/precious"), "keep this\n").unwrap();
    std::fs::write(f.owner.join("vendor/*/file"), "exported\n").unwrap();
    git(&f.owner, &["add", "vendor"]);
    git(&f.owner, &["commit", "-qm", "Local work"]);
    let work = PathBuf::from(success(cli(
        &f.owner,
        &[
            "pr",
            "prepare",
            "vendor/*",
            "--branch",
            "contribute/literal",
            "--message",
            "Public fix",
            "--quiet",
        ],
    )));
    let tip = git(&work, &["rev-parse", "HEAD"]);
    success(cli(
        &f.owner,
        &["pr", "publish", "vendor/*", "--expected-head", &tip],
    ));
    let merged = f.merge("contribute/literal", false);
    success(cli(
        &f.owner,
        &["pr", "reconcile", "vendor/*", "--merged", &merged],
    ));
    assert_eq!(
        std::fs::read_to_string(f.owner.join("vendor/sibling/precious")).unwrap(),
        "keep this\n"
    );
    assert_eq!(
        git(
            &f.owner,
            &["diff", "--cached", "--name-only", "--", "vendor/sibling"]
        ),
        ""
    );
}

#[test]
#[cfg(unix)]
fn concurrent_metadata_settings_are_not_overwritten_during_reconciliation() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.edit("file", "exported\n");
    let work = f.prepare("contribute/metadata-race", &[]);
    f.publish(&work);
    let merged = f.merge("contribute/metadata-race", false);
    let before = git(
        &f.owner,
        &["config", "--file", "lib/unit/.gitrepo", "subrepo.commit"],
    );
    git(
        &f.owner,
        &[
            "config",
            "--file",
            "lib/unit/.gitrepo",
            "subrepo.exclude",
            "initial/",
        ],
    );
    let shim = f.temp.join("shim");
    std::fs::create_dir(&shim).unwrap();
    let script = shim.join("git");
    std::fs::write(
        &script,
        r#"#!/bin/sh
if [ "$1" = read-tree ] && [ "$2" = -m ] && [ "$3" = -u ]; then
    "$PR_TEST_GIT" -C "$PR_TEST_OWNER" config --file lib/unit/.gitrepo subrepo.exclude concurrent/
fi
exec "$PR_TEST_GIT" "$@"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let real_git = success(command("sh", &f.temp, &["-c", "command -v git"]));
    let old_path = std::env::var_os("PATH").unwrap();
    let path = std::env::join_paths(std::iter::once(shim).chain(std::env::split_paths(&old_path)))
        .unwrap();
    let output = invocation(
        env!("CARGO_BIN_EXE_git-subrepo"),
        &f.owner,
        &["pr", "reconcile", "lib/unit", "--merged", &merged],
    )
    .env("PATH", path)
    .env("PR_TEST_GIT", real_git)
    .env("PR_TEST_OWNER", &f.owner)
    .output()
    .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Root .gitrepo changed during checkout")
    );
    assert_eq!(
        git(
            &f.owner,
            &["config", "--file", "lib/unit/.gitrepo", "subrepo.exclude"]
        ),
        "concurrent/"
    );
    assert_eq!(
        git(
            &f.owner,
            &["config", "--file", "lib/unit/.gitrepo", "subrepo.commit"]
        ),
        before
    );
}

#[test]
#[cfg(unix)]
fn a_racing_owner_commit_cannot_be_reverted_outside_the_subtree() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.edit("file", "exported\n");
    let work = f.prepare("contribute/race", &[]);
    f.publish(&work);
    let merged = f.merge("contribute/race", false);
    let shim = f.temp.join("shim");
    std::fs::create_dir(&shim).unwrap();
    let script = shim.join("git");
    std::fs::write(
        &script,
        r#"#!/bin/sh
if [ "$1" = read-tree ] && [ "$2" = -m ] && [ "$3" = -u ]; then
    printf 'concurrent owner change\n' > "$PR_TEST_OWNER/private"
    "$PR_TEST_GIT" -C "$PR_TEST_OWNER" add private
    "$PR_TEST_GIT" -C "$PR_TEST_OWNER" commit -qm 'Concurrent owner commit'
fi
exec "$PR_TEST_GIT" "$@"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let real_git = success(command("sh", &f.temp, &["-c", "command -v git"]));
    let old_path = std::env::var_os("PATH").unwrap();
    let path = std::env::join_paths(std::iter::once(shim).chain(std::env::split_paths(&old_path)))
        .unwrap();
    let output = invocation(
        env!("CARGO_BIN_EXE_git-subrepo"),
        &f.owner,
        &["pr", "reconcile", "lib/unit", "--merged", &merged],
    )
    .env("PATH", path)
    .env("PR_TEST_GIT", real_git)
    .env("PR_TEST_OWNER", &f.owner)
    .output()
    .unwrap();
    // It may reject the race or safely stage only the subtree; it must not undo
    // another writer's already committed file.
    assert_eq!(
        std::fs::read_to_string(f.owner.join("private")).unwrap(),
        "concurrent owner change\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(git(&f.owner, &["diff", "--cached", "--", "private"]), "");
}
