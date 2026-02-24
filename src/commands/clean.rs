use crate::commands::{Context, normalize_subdir};
use crate::encode::encode_subdir;
use crate::git_utils::{branch_exists, try_run_git};
use anyhow::Result;

pub fn run(subdir: Option<String>, force: bool, quiet: bool) -> Result<()> {
    let subdir = match subdir {
        Some(s) => s,
        None => anyhow::bail!("Command 'clean' requires arg 'subdir'."),
    };
    let ctx = Context::new()?;

    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);
    let branch_name = format!("subrepo/{subref}");

    let worktree = ctx.worktree_path(&subdir);

    // Remove worktree if exists (silently, as in bash)
    if worktree.exists() {
        std::fs::remove_dir_all(&worktree)?;
        try_run_git(&["worktree", "prune"], &ctx.repo_root);
    }

    // Remove branch if exists
    if branch_exists(&branch_name, &ctx.repo_root) {
        try_run_git(
            &["update-ref", "-d", &format!("refs/heads/{branch_name}")],
            &ctx.repo_root,
        );
        if !quiet {
            println!("Removed branch '{branch_name}'.");
        }
    }

    if force {
        // Remove all subrepo refs for this subref
        let (ok, out) = try_run_git(&["show-ref"], &ctx.repo_root);
        if ok {
            let prefix = format!("refs/subrepo/{subref}/");
            for line in out.lines() {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    let ref_name = parts[1];
                    if ref_name.starts_with(&prefix) {
                        try_run_git(&["update-ref", "-d", ref_name], &ctx.repo_root);
                    }
                }
            }
            // Also clean up refs/original/
            let orig_prefix = format!("refs/original/refs/heads/subrepo/{subref}/");
            for line in out.lines() {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    let ref_name = parts[1];
                    if ref_name.starts_with(&orig_prefix) {
                        try_run_git(&["update-ref", "-d", ref_name], &ctx.repo_root);
                    }
                }
            }
        }
    }

    Ok(())
}
