//! Keep owner-only paths out of outgoing trees and preserve them on import.
use crate::git_utils::{run_git, run_git_bytes, run_git_env};
use anyhow::{Result, ensure};
use std::path::{Component, Path};

/// Require a literal, subtree-relative file or directory without traversal.
pub fn validate(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && !path.starts_with(':')
            && !path.contains('\0')
            && !path.split('/').any(|part| part == "." || part == "..")
            && Path::new(path).components().all(
                |part| matches!(part, Component::Normal(name) if !name.eq_ignore_ascii_case(".git"))
            ),
        "Invalid exclusion {path:?}: expected a literal subtree-relative path"
    );
    Ok(())
}

/// Restore selected paths from a baseline tree, including absent paths and deletions.
pub fn restore(cwd: &Path, target: &str, baseline: &str, paths: &[String]) -> Result<String> {
    if paths.is_empty() {
        return Ok(target.to_owned());
    }
    for path in paths {
        validate(path)?;
    }
    let specifications: Vec<_> = paths
        .iter()
        .map(|p| format!(":(literal){}", p.trim_end_matches('/')))
        .collect();
    let mut args = vec![
        "diff",
        "--binary",
        "--full-index",
        "--no-color",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        target,
        baseline,
        "--",
    ];
    args.extend(specifications.iter().map(String::as_str));
    let patch = run_git_bytes(cwd, &args, None, None)?;
    if patch.is_empty() {
        return Ok(target.to_owned());
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "subrepo-exclusion-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory)?;
    let index = directory.join("index");
    let result = (|| {
        let index_name = index.to_string_lossy();
        let env = [("GIT_INDEX_FILE", index_name.as_ref())];
        run_git_env(&["read-tree", target], cwd, &env)?;
        run_git_bytes(
            cwd,
            &["apply", "--cached", "--binary"],
            Some(&patch),
            Some(&index),
        )?;
        run_git_env(&["write-tree"], cwd, &env)
    })();
    let _ = std::fs::remove_dir_all(directory);
    result
}

/// Preserve the owner's excluded paths when importing a public subrepo tree.
pub fn import_tree(cwd: &Path, subdir: &str, incoming: &str) -> Result<String> {
    let cfg = crate::gitrepo::read_gitrepo(&cwd.join(subdir).join(".gitrepo"), cwd)?;
    restore(cwd, incoming, &format!("HEAD:{subdir}"), &cfg.exclude)
}

/// Rewrite every outgoing commit's excluded paths without changing upstream ancestry.
pub fn filter_branch(cwd: &Path, branch: &str, baseline: &str, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let empty;
    let (base, range) = if baseline.is_empty() {
        empty = String::from_utf8(run_git_bytes(
            cwd,
            &["hash-object", "-t", "tree", "-w", "--stdin"],
            Some(b""),
            None,
        )?)?
        .trim()
        .to_owned();
        (empty.as_str(), branch.to_owned())
    } else {
        (baseline, format!("{baseline}..{branch}"))
    };
    let mut mapped = std::collections::HashMap::<String, String>::new();
    let tip = run_git(&["rev-parse", branch], cwd)?;
    for commit in run_git(&["rev-list", "--reverse", "--topo-order", &range], cwd)?.lines() {
        let raw = run_git_bytes(cwd, &["cat-file", "commit", commit], None, None)?;
        let split = raw
            .windows(2)
            .position(|part| part == b"\n\n")
            .ok_or_else(|| anyhow::anyhow!("Invalid commit header"))?;
        let header = std::str::from_utf8(&raw[..split])?;
        let tree = run_git(&["rev-parse", &format!("{commit}^{{tree}}")], cwd)?;
        let tree = restore(cwd, &tree, base, paths)?;
        let mut rebuilt = Vec::new();
        let mut signature = false;
        for line in header.lines() {
            if line.starts_with(' ') && signature {
                continue;
            }
            signature = line.starts_with("gpgsig ")
                || line.starts_with("gpgsig-sha256 ")
                || line.starts_with("mergetag ");
            if signature {
                continue;
            }
            let line = if line.starts_with("tree ") {
                format!("tree {tree}")
            } else if let Some(parent) = line.strip_prefix("parent ") {
                format!(
                    "parent {}",
                    mapped.get(parent).map(String::as_str).unwrap_or(parent)
                )
            } else {
                line.to_owned()
            };
            rebuilt.extend_from_slice(line.as_bytes());
            rebuilt.push(b'\n');
        }
        rebuilt.push(b'\n');
        rebuilt.extend_from_slice(&raw[split + 2..]);
        let rewritten = String::from_utf8(run_git_bytes(
            cwd,
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            Some(&rebuilt),
            None,
        )?)?
        .trim()
        .to_owned();
        mapped.insert(commit.to_owned(), rewritten);
    }
    if let Some(rewritten) = mapped.get(&tip) {
        run_git(
            &[
                "update-ref",
                &format!("refs/heads/{branch}"),
                rewritten,
                &tip,
            ],
            cwd,
        )?;
    }
    Ok(())
}
