use crate::commands::{Context, assert_clean_for, normalize_subdir, subrepo_fetch_with_pb};
use crate::encode::encode_subdir;
use crate::gitrepo::read_gitrepo;
use anyhow::Result;
use colored::Colorize;

pub fn run(
    subdir: String,
    branch_override: Option<String>,
    remote_override: Option<String>,
    quiet: bool,
) -> Result<String> {
    run_with_pb(subdir, branch_override, remote_override, quiet, None)
}

pub fn run_with_pb(
    subdir: String,
    branch_override: Option<String>,
    remote_override: Option<String>,
    quiet: bool,
    pb: Option<indicatif::ProgressBar>,
) -> Result<String> {
    let mut ctx = Context::new()?;
    ctx.quiet = quiet;
    assert_clean_for("fetch", &ctx)?;

    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let mut cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    if cfg.remote == "none" {
        return Ok(format!("Ignored '{subdir}', no remote."));
    }

    if let Some(r) = branch_override {
        cfg.branch = r;
    }
    if let Some(r) = remote_override {
        cfg.remote = r;
    }

    let _upstream_head =
        subrepo_fetch_with_pb(&ctx, &cfg.remote, &cfg.branch, &subref, pb.as_ref())?;

    let msg = format!(
        "{}",
        format!("Fetched '{subdir}' from '{}' ({}).", cfg.remote, cfg.branch).green()
    );
    // When called standalone (no caller pb), print immediately.
    if pb.is_none() && !quiet {
        println!("{msg}");
    }
    Ok(msg)
}
