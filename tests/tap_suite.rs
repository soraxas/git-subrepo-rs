/// Auto-generated wrappers that run each TAP bash test (.t file) as a Rust test.
///
/// Each test:
///   1. Builds the binary (via the test binary depending on the package).
///   2. Runs `bash test/<name>.t` in the repo root with `prove`.
///   3. Fails if prove exits non-zero.
///
/// Run with:  cargo nextest run
///            cargo test
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    // Integration tests run from the repo root (workspace root).
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_tap(test_file: &str) {
    let root = repo_root();
    // Ensure binary is built (nextest builds before running, so this is usually a no-op).
    let status = std::process::Command::new("prove")
        .arg(format!("test/{test_file}"))
        .current_dir(&root)
        .status()
        .expect("failed to run prove");
    assert!(
        status.success(),
        "TAP test 'test/{test_file}' failed (exit {})",
        status.code().unwrap_or(-1)
    );
}

macro_rules! tap_test {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            run_tap($file);
        }
    };
}

tap_test!(branch_all, "branch-all.t");
tap_test!(branch_rev_list_one_path, "branch-rev-list-one-path.t");
tap_test!(branch_rev_list, "branch-rev-list.t");
tap_test!(branch, "branch.t");
tap_test!(clean, "clean.t");
tap_test!(clone_annotated_tag, "clone-annotated-tag.t");
tap_test!(clone_message, "clone-message.t");
tap_test!(clone, "clone.t");
tap_test!(compile, "compile.t");
tap_test!(config, "config.t");
tap_test!(encode, "encode.t");
tap_test!(error, "error.t");
tap_test!(fetch, "fetch.t");
tap_test!(gitignore, "gitignore.t");
tap_test!(init, "init.t");
tap_test!(issue29, "issue29.t");
tap_test!(issue95, "issue95.t");
tap_test!(issue96, "issue96.t");
tap_test!(pull_all, "pull-all.t");
tap_test!(pull_merge, "pull-merge.t");
tap_test!(pull_message, "pull-message.t");
tap_test!(pull_new_branch, "pull-new-branch.t");
tap_test!(pull_ours, "pull-ours.t");
tap_test!(pull_theirs, "pull-theirs.t");
tap_test!(pull_twice, "pull-twice.t");
tap_test!(pull_worktree, "pull-worktree.t");
tap_test!(pull, "pull.t");
tap_test!(push_after_init, "push-after-init.t");
tap_test!(push_after_push_no_changes, "push-after-push-no-changes.t");
tap_test!(push_force, "push-force.t");
tap_test!(push_new_branch, "push-new-branch.t");
tap_test!(push_no_changes, "push-no-changes.t");
tap_test!(push_squash, "push-squash.t");
tap_test!(push, "push.t");
tap_test!(rebase, "rebase.t");
tap_test!(reclone, "reclone.t");
tap_test!(status, "status.t");
tap_test!(submodule, "submodule.t");
tap_test!(zsh, "zsh.t");
