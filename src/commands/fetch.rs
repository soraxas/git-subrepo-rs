use crate::commands::{Context, assert_clean_for, normalize_subdir, subrepo_fetch};
use crate::encode::encode_subdir;
use crate::gitrepo::read_gitrepo;
use anyhow::Result;

pub fn run(
    subdir: String,
    branch_override: Option<String>,
    remote_override: Option<String>,
    quiet: bool,
) -> Result<()> {
    let ctx = Context::new()?;
    assert_clean_for("fetch", &ctx)?;

    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let mut cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    if cfg.remote == "none" {
        println!("Ignored '{subdir}', no remote.");
        return Ok(());
    }

    if let Some(r) = remote_override {
        cfg.remote = r;
    }
    if let Some(b) = branch_override {
        cfg.branch = b;
    }

    let _upstream_head = subrepo_fetch(&ctx, &cfg.remote, &cfg.branch, &subref)?;

    if !quiet {
        println!("Fetched '{subdir}' from '{}' ({}).", cfg.remote, cfg.branch);
    }

    Ok(())
}
