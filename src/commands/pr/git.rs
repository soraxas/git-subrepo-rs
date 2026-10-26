use crate::git_utils::{run_git, run_git_env, try_run_git};
use anyhow::{Context as _, Result, ensure};
use std::path::Path;
use std::process::{Command, Stdio};

/// Run Git with byte-preserving input/output for patches, including binary changes.
pub(super) fn bytes(
    cwd: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    index: Option<&Path>,
) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().context("Git stdin unavailable")?;
    // Drain stdout/stderr concurrently with writing large binary patches.
    let output = std::thread::scope(|scope| {
        let writer = scope.spawn(move || -> std::io::Result<()> {
            use std::io::Write;
            if let Some(input) = input {
                stdin.write_all(input)?;
            }
            Ok(())
        });
        let output = child.wait_with_output()?;
        let wrote = writer.join().expect("Git input writer panicked");
        if output.status.success() {
            wrote?;
        }
        Ok::<_, std::io::Error>(output)
    })?;
    ensure!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr).trim(),
        String::from_utf8_lossy(&output.stdout).trim()
    );
    Ok(output.stdout)
}

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
