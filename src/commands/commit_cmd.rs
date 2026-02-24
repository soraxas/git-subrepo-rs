use crate::commands::{Context, assert_clean_for, normalize_subdir, subrepo_commit};
use crate::encode::encode_subdir;
use crate::git_utils::{rev_exists, run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;

pub fn run(
    subdir: String,
    subrepo_commit_ref_arg: Option<String>,
    force: bool,
    fetch: bool,
    quiet: bool,
    message: Option<String>,
) -> Result<()> {
    let ctx = Context::new()?;
    assert_clean_for("commit", &ctx)?;

    let subdir = normalize_subdir(&subdir);
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

    let commit_ref = subrepo_commit_ref_arg.unwrap_or_else(|| format!("subrepo/{subref}"));

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
    )?;

    if !quiet {
        println!(
            "Subrepo commit '{commit_ref}' committed as subdir '{subdir}/' to branch '{}'.",
            ctx.original_head_branch
        );
    }

    Ok(())
}
