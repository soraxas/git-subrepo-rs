use crate::commands::{
    assert_clean_for,
    location::{cleanup_worktrees, resolve_commit},
    subrepo_commit,
};
use crate::encode::encode_subdir;
use crate::git_utils::{rev_exists, run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;

fn assert_clean_import(name: &str, path: &std::path::Path) -> Result<()> {
    if !run_git(
        &["status", "--porcelain", "-z", "--untracked-files=all"],
        path,
    )?
    .is_empty()
    {
        anyhow::bail!(
            "{name} '{}' has uncommitted changes. Commit or stash them before importing.",
            path.display()
        );
    }
    Ok(())
}

pub fn run(
    subdir: Option<String>,
    subrepo_commit_ref_arg: Option<String>,
    force: bool,
    fetch: bool,
    quiet: bool,
    message: Option<String>,
    verify: bool,
) -> Result<()> {
    let inferred = subdir.is_none();
    let location = resolve_commit(&std::env::current_dir()?, subdir.as_deref())?;
    let ctx = location.context;
    let subdir = location.subdir;
    if let Some(source) = &location.source_worktree {
        assert_clean_import("Workon tree", source)?;
        assert_clean_import("Owning checkout", &ctx.repo_root)?;
    } else if inferred {
        assert_clean_import("Owning checkout", &ctx.repo_root)?;
        for source in cleanup_worktrees(&ctx, &subdir)? {
            assert_clean_import("Subrepo worktree", &source)?;
        }
    }
    assert_clean_for("commit", &ctx)?;

    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    let refs_subrepo_fetch = format!("refs/subrepo/{subref}/fetch");

    let upstream_head = if fetch {
        crate::commands::subrepo_fetch(&ctx, &cfg.remote, &cfg.branch, &subref)?
    } else if rev_exists(&refs_subrepo_fetch, &ctx.repo_root) {
        run_git(&["rev-parse", &refs_subrepo_fetch], &ctx.repo_root)?
    } else {
        anyhow::bail!("Can't find ref '{refs_subrepo_fetch}'. Try using -F.");
    };

    let commit_ref = match &location.source_worktree {
        Some(source) => {
            let reference = subrepo_commit_ref_arg.as_deref().unwrap_or("HEAD");
            run_git(
                &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
                source,
            )?
        }
        None => subrepo_commit_ref_arg.unwrap_or_else(|| format!("subrepo/{subref}")),
    };

    let commit_msg = message.unwrap_or_else(|| {
        crate::commands::build_commit_message(
            "commit",
            &subdir,
            &commit_ref,
            &cfg.remote,
            &cfg.branch,
            &upstream_head,
            &ctx.repo_root,
        )
    });

    if (inferred || location.source_worktree.is_some()) && !quiet {
        println!(
            "Importing subrepo '{subdir}' into '{}' on branch '{}'.",
            ctx.repo_root.display(),
            ctx.original_head_branch
        );
    }
    subrepo_commit(
        &ctx,
        &subdir,
        &subref,
        &commit_ref,
        &cfg.remote,
        &cfg.branch,
        &upstream_head,
        &cfg.method,
        force,
        Some(&commit_msg),
        verify || cfg.verify,
        location.source_worktree.is_some(),
    )?;

    if !quiet {
        println!(
            "Subrepo commit '{commit_ref}' committed as subdir '{subdir}/' to branch '{}'.",
            ctx.original_head_branch
        );
    }

    Ok(())
}
