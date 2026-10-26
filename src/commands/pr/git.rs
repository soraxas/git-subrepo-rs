use crate::git_utils::{run_git, run_git_env, try_run_git};
use anyhow::{Context as _, Result, ensure};
use std::path::Path;

pub(super) use crate::git_utils::run_git_bytes as bytes;

/// Manipulate a temporary index without altering the checkout's real index.
pub(super) fn index_git(cwd: &Path, index: &Path, args: &[&str]) -> Result<String> {
    run_git_env(args, cwd, &[("GIT_INDEX_FILE", &index.to_string_lossy())])
}

/// Snapshot committed source while excluding the owner's root subrepo metadata.
pub(super) fn subtree(cwd: &Path, subdir: &str, index: &Path) -> Result<String> {
    index_git(cwd, index, &["read-tree", &format!("HEAD:{subdir}")])?;
    index_git(
        cwd,
        index,
        &["update-index", "--force-remove", "--", ".gitrepo"],
    )?;
    index_git(cwd, index, &["write-tree"])
}

/// Construct a public-tree commit whose only parent is the chosen upstream baseline.
pub(super) fn commit(cwd: &Path, tree: &str, parent: &str, message: &str) -> Result<String> {
    run_git(&["commit-tree", tree, "-p", parent, "-m", message], cwd)
}

pub(super) fn ancestor(cwd: &Path, older: &str, newer: &str) -> bool {
    try_run_git(&["merge-base", "--is-ancestor", older, newer], cwd).0
}

pub(super) fn head(cwd: &Path) -> Result<String> {
    run_git(&["rev-parse", "HEAD"], cwd)
}

pub(super) fn tree(cwd: &Path, rev: &str) -> Result<String> {
    run_git(&["rev-parse", &format!("{rev}^{{tree}}")], cwd)
}

/// Require committed input; include untracked files instead of silently exporting a stale snapshot.
pub(super) fn clean(cwd: &Path, subdir: Option<&str>) -> Result<()> {
    let mut args = vec!["status", "--porcelain", "--untracked-files=all"];
    let literal = subdir.map(|path| format!(":(literal){path}"));
    if let Some(ref path) = literal {
        args.extend(["--", path]);
    }
    ensure!(
        run_git(&args, cwd)?.is_empty(),
        "Uncommitted changes; commit or reconcile them before this PR operation"
    );
    Ok(())
}

/// Require a literal relative path, avoiding traversal and pathspec interpretation.
pub(super) fn relative(value: &str) -> Result<()> {
    use std::path::Component;
    ensure!(
        !value.is_empty()
            && Path::new(value)
                .components()
                .all(|part| matches!(part, Component::Normal(name) if name != ".git")),
        "Expected a literal relative path: {value}"
    );
    Ok(())
}

/// Fetch the accepted branch independently from the destination topic branch.
pub(super) fn fetch(cwd: &Path, url: &str, branch: &str) -> Result<String> {
    run_git(
        &["fetch", "--no-tags", url, &format!("refs/heads/{branch}")],
        cwd,
    )?;
    run_git(&["rev-parse", "FETCH_HEAD^{commit}"], cwd)
}

/// Copy the owning repository's effective identity into its isolated export repository.
pub(super) fn identity(owner: &Path, export: &Path) -> Result<()> {
    let ident = run_git(&["var", "GIT_COMMITTER_IDENT"], owner)?;
    let (name, tail) = ident
        .rsplit_once(" <")
        .context("Invalid Git committer identity")?;
    let (email, _) = tail
        .split_once('>')
        .context("Invalid Git committer identity")?;
    run_git(&["config", "user.name", name], export)?;
    run_git(&["config", "user.email", email], export)?;
    Ok(())
}
