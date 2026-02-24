use std::path::Path;
use std::process::Command;

#[allow(dead_code)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

fn make_cmd(args: &[&str], cwd: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// Run a git command; return stdout on success, Err on failure.
pub fn run_git(args: &[&str], cwd: &Path) -> anyhow::Result<String> {
    let output = make_cmd(args, cwd).output()?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout)
            .trim_end_matches('\n')
            .to_string();
        Ok(s)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let msg = if !stderr.trim().is_empty() {
            stderr.trim_end().to_string()
        } else {
            stdout.trim_end().to_string()
        };
        Err(anyhow::anyhow!("{}", msg))
    }
}

/// Run a git command with extra env vars; return stdout on success, Err on failure.
#[allow(dead_code)]
pub fn run_git_env(args: &[&str], cwd: &Path, envs: &[(&str, &str)]) -> anyhow::Result<String> {
    let mut cmd = make_cmd(args, cwd);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output()?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout)
            .trim_end_matches('\n')
            .to_string();
        Ok(s)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let msg = if !stderr.trim().is_empty() {
            stderr.trim_end().to_string()
        } else {
            stdout.trim_end().to_string()
        };
        Err(anyhow::anyhow!("{}", msg))
    }
}

/// Run a git command; returns (success, combined_output) without failing.
pub fn try_run_git(args: &[&str], cwd: &Path) -> (bool, String) {
    match make_cmd(args, cwd).output() {
        Ok(output) => {
            let success = output.status.success();
            let stdout = String::from_utf8_lossy(&output.stdout)
                .trim_end_matches('\n')
                .to_string();
            let stderr = String::from_utf8_lossy(&output.stderr)
                .trim_end_matches('\n')
                .to_string();
            let out = if !stdout.is_empty() { stdout } else { stderr };
            (success, out)
        }
        Err(e) => (false, e.to_string()),
    }
}

/// Run a git command with extra env vars; returns (success, combined_output).
pub fn try_run_git_env(args: &[&str], cwd: &Path, envs: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = make_cmd(args, cwd);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    match cmd.output() {
        Ok(output) => {
            let success = output.status.success();
            let stdout = String::from_utf8_lossy(&output.stdout)
                .trim_end_matches('\n')
                .to_string();
            let stderr = String::from_utf8_lossy(&output.stderr)
                .trim_end_matches('\n')
                .to_string();
            let out = if !stdout.is_empty() { stdout } else { stderr };
            (success, out)
        }
        Err(e) => (false, e.to_string()),
    }
}

/// Check if a git ref exists.
pub fn rev_exists(rev: &str, cwd: &Path) -> bool {
    try_run_git(&["rev-list", rev, "-1"], cwd).0
}

/// Check if a branch exists.
pub fn branch_exists(branch: &str, cwd: &Path) -> bool {
    rev_exists(&format!("refs/heads/{branch}"), cwd)
}

/// Get short SHA for a ref (7 chars).
pub fn rev_parse_short(rev: &str, cwd: &Path) -> Option<String> {
    let (ok, out) = try_run_git(&["rev-parse", "--short", rev], cwd);
    if ok { Some(out) } else { None }
}

/// Get full SHA for a ref.
pub fn rev_parse(rev: &str, cwd: &Path) -> Option<String> {
    let (ok, out) = try_run_git(&["rev-parse", rev], cwd);
    if ok { Some(out) } else { None }
}

/// Check if a commit is in the rev-list of a branch/ref.
pub fn commit_in_rev_list(commit: &str, list_head: &str, cwd: &Path) -> bool {
    let (ok, out) = try_run_git(&["rev-list", list_head], cwd);
    if !ok {
        return false;
    }
    out.lines().any(|line| line.starts_with(commit))
}
