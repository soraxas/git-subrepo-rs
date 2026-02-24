use crate::commands::{Context, normalize_subdir, subrepo_branch, subrepo_fetch};
use crate::encode::encode_subdir;
use crate::git_utils::{branch_exists, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;

pub fn run(subdir: String, force: bool, fetch: bool, quiet: bool) -> Result<()> {
    let mut ctx = Context::new()?;
    ctx.quiet = quiet;

    let subdir = normalize_subdir(&subdir);
    // NOTE: no clean-tree check here. `branch` creates a worktree in .git/tmp/subrepo/,
    // it never modifies the working-directory files under `subdir/`, so staged/unstaged
    // changes there are irrelevant to the operation.

    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    if fetch {
        subrepo_fetch(&ctx, &cfg.remote, &cfg.branch, &subref)?;
    }

    let branch_name = format!("subrepo/{subref}");
    let worktree_display = ctx.worktree_display(&subdir);
    let worktree_path = ctx.worktree_path(&subdir);

    // Preflight: if the branch already exists, check whether re-creating it would produce
    // a different result. If no new commits touch the subdir since the branch was last
    // built, the existing branch is already correct — no --force needed.
    if !force && branch_exists(&branch_name, &ctx.repo_root) {
        let subdir_path = format!("{subdir}/");
        let (_, new_commits) = try_run_git(
            &[
                "rev-list",
                "--ancestry-path",
                &format!("{}..HEAD", branch_name),
                "--",
                &subdir_path,
            ],
            &ctx.repo_root,
        );
        if new_commits.trim().is_empty() {
            // Branch is already current. Reuse it.
            if !quiet {
                println!("Branch '{branch_name}' is already up to date.");
                if worktree_path.exists() && std::io::stdout().is_terminal() {
                    println!("  {}", format!("cd '{worktree_display}'").bright_cyan());
                }
            }
            return Ok(());
        }
        // There ARE new commits — fall through to recreate with force=true
        // (user opted not to pass --force, but we detected drift so we recreate)
    }

    match subrepo_branch(
        &ctx,
        &subdir,
        &subref,
        &cfg.parent,
        &cfg.method,
        force || branch_exists(&branch_name, &ctx.repo_root),
    ) {
        Ok(_) => {
            if !quiet {
                println!(
                    "Created branch '{}' and worktree '{worktree_display}'.",
                    branch_name.green().bold()
                );
                if std::io::stdout().is_terminal() {
                    println!("  {}", format!("cd '{worktree_display}'").bright_cyan());
                }
            }
        }
        Err(e) if e.to_string() == "no_commits" => {
            if !quiet {
                println!("Subrepo '{subdir}' has no local commits to branch.");
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}
