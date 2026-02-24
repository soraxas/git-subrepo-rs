use crate::commands::{
    Context, assert_clean_for, delete_branch_and_worktree, normalize_subdir, subrepo_branch,
    subrepo_fetch,
};
use crate::encode::encode_subdir;
use crate::git_utils::branch_exists;
use crate::gitrepo::read_gitrepo;
use anyhow::Result;

pub fn run(subdir: String, force: bool, fetch: bool, quiet: bool) -> Result<()> {
    let ctx = Context::new()?;
    assert_clean_for("branch", &ctx)?;

    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    if fetch {
        subrepo_fetch(&ctx, &cfg.remote, &cfg.branch, &subref)?;
    }

    let branch_name = format!("subrepo/{subref}");

    if force {
        delete_branch_and_worktree(&ctx, &subdir, &subref)?;
    }

    if branch_exists(&branch_name, &ctx.repo_root) {
        anyhow::bail!("Branch '{branch_name}' already exists. Use '--force' to override.");
    }

    let _worktree = subrepo_branch(&ctx, &subdir, &subref, &cfg.parent, &cfg.method)?;

    if !quiet {
        println!(
            "Created branch '{branch_name}' and worktree '{}'.",
            ctx.worktree_display(&subdir)
        );
    }

    Ok(())
}
