use crate::git_utils::run_git;
use anyhow::Result;
use std::path::Path;

const GITREPO_HEADER: &str = "; DO NOT EDIT (unless you know what you are doing)\n;\n; This subdirectory is a git \"subrepo\", and this file is maintained by the\n; git-subrepo command. See https://github.com/ingydotnet/git-subrepo#readme\n;\n";

#[derive(Debug, Default, Clone)]
pub struct GitrepoConfig {
    pub remote: String,
    pub branch: String,
    pub commit: String,
    pub parent: String,
    pub method: String,
    #[allow(dead_code)]
    pub cmdver: String,
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
