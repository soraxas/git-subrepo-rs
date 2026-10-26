use crate::git_utils::run_git;
use anyhow::Result;
use std::path::Path;

const GITREPO_HEADER: &str = "; DO NOT EDIT (unless you know what you are doing)\n;\n; This subdirectory is a git \"subrepo\", and this file is maintained by the\n; git-subrepo command. See https://github.com/soraxas/git-subrepo-rs\n;\n";

#[derive(Debug, Default, Clone)]
pub struct GitrepoConfig {
    pub remote: String,
    pub branch: String,
    pub commit: String,
    pub parent: String,
    pub method: String,
    #[allow(dead_code)]
    pub cmdver: String,
    /// If true, run git hooks on commit (default: false = --no-verify)
    pub verify: bool,
    /// Optional URL of the original project, used by `workon` as the fetch/pull source
    /// (while `remote` stays the push destination, as it always has been).
    pub upstream: String,
    /// Literal subtree-relative paths whose local changes must stay in the owner.
    pub exclude: Vec<String>,
}

/// Read a .gitrepo file using git config.
pub fn read_gitrepo(path: &Path, repo_root: &Path) -> Result<GitrepoConfig> {
    let path_str = path.to_string_lossy();

    let get = |key: &str| -> String {
        let (_, out) = crate::git_utils::try_run_git(
            &["config", "--file", &path_str, &format!("subrepo.{key}")],
            repo_root,
        );
        out
    };

    let output = std::process::Command::new("git")
        .args([
            "config",
            "--null",
            "--file",
            &path_str,
            "--get-all",
            "subrepo.exclude",
        ])
        .current_dir(repo_root)
        .output()?;
    anyhow::ensure!(
        output.status.success() || output.status.code() == Some(1),
        "Cannot read exclusions: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let raw = String::from_utf8(output.stdout)?;
    let exclude = raw
        .split_terminator('\0')
        .map(|path| {
            crate::exclusions::validate(path)?;
            Ok(path.to_owned())
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(GitrepoConfig {
        remote: get("remote"),
        branch: get("branch"),
        commit: get("commit"),
        parent: get("parent"),
        method: {
            let m = get("method");
            if m.is_empty() { "merge".to_string() } else { m }
        },
        cmdver: get("cmdver"),
        verify: get("verify") == "true",
        upstream: get("upstream"),
        exclude,
    })
}

/// Write a new .gitrepo file with the standard header, then set values via git config.
#[allow(clippy::too_many_arguments)]
pub fn write_new_gitrepo(
    path: &Path,
    remote: &str,
    branch: &str,
    commit: &str,
    parent: Option<&str>, // None = don't write
    method: &str,
    cmdver: &str,
    repo_root: &Path,
) -> Result<()> {
    std::fs::write(path, GITREPO_HEADER)?;
    set_gitrepo_values(
        path, remote, branch, commit, parent, method, cmdver, repo_root,
    )
}

/// Update an existing .gitrepo file (preserves header, updates values).
#[allow(clippy::too_many_arguments)]
pub fn update_gitrepo(
    path: &Path,
    remote: Option<&str>, // None = don't update
    branch: Option<&str>, // None = don't update
    commit: &str,
    parent: Option<&str>, // None = don't write
    method: &str,
    cmdver: &str,
    repo_root: &Path,
) -> Result<()> {
    // Refresh the header comment (migrate from old git-subrepo to git-subrepo-rs).
    // Read current file, strip leading `;` comment lines, prepend our canonical header.
    if let Ok(current) = std::fs::read_to_string(path) {
        let body: String = current
            .lines()
            .skip_while(|l| l.starts_with(';') || l.trim().is_empty())
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(path, format!("{GITREPO_HEADER}{body}"))?;
    }

    let path_str = path.to_string_lossy().into_owned();

    if let Some(r) = remote {
        run_git(
            &["config", "--file", &path_str, "subrepo.remote", r],
            repo_root,
        )?;
    }
    if let Some(b) = branch {
        run_git(
            &["config", "--file", &path_str, "subrepo.branch", b],
            repo_root,
        )?;
    }
    run_git(
        &["config", "--file", &path_str, "subrepo.commit", commit],
        repo_root,
    )?;
    if let Some(p) = parent {
        run_git(
            &["config", "--file", &path_str, "subrepo.parent", p],
            repo_root,
        )?;
    }
    run_git(
        &["config", "--file", &path_str, "subrepo.method", method],
        repo_root,
    )?;
    run_git(
        &["config", "--file", &path_str, "subrepo.cmdver", cmdver],
        repo_root,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn set_gitrepo_values(
    path: &Path,
    remote: &str,
    branch: &str,
    commit: &str,
    parent: Option<&str>,
    method: &str,
    cmdver: &str,
    repo_root: &Path,
) -> Result<()> {
    let path_str = path.to_string_lossy().into_owned();

    run_git(
        &["config", "--file", &path_str, "subrepo.remote", remote],
        repo_root,
    )?;
    run_git(
        &["config", "--file", &path_str, "subrepo.branch", branch],
        repo_root,
    )?;
    // Write commit even if empty
    run_git(
        &["config", "--file", &path_str, "subrepo.commit", commit],
        repo_root,
    )?;
    if let Some(p) = parent {
        run_git(
            &["config", "--file", &path_str, "subrepo.parent", p],
            repo_root,
        )?;
    }
    run_git(
        &["config", "--file", &path_str, "subrepo.method", method],
        repo_root,
    )?;
    run_git(
        &["config", "--file", &path_str, "subrepo.cmdver", cmdver],
        repo_root,
    )?;
    Ok(())
}
