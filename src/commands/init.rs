use crate::commands::{Context, VERSION, assert_clean_for, normalize_subdir};
use crate::encode::encode_subdir;
use crate::git_utils::run_git;
use anyhow::Result;

pub fn run(
    subdir: String,
    remote: Option<String>,
    branch: Option<String>,
    method: Option<String>,
) -> Result<()> {
    let ctx = Context::new()?;
    assert_clean_for("init", &ctx)?;

    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);

    let subdir_path = ctx.repo_root.join(&subdir);
    if !subdir_path.exists() {
        anyhow::bail!("The subdir '{subdir}' does not exist.");
    }

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    if gitrepo_path.exists() {
        anyhow::bail!("The subdir '{subdir}' is already a subrepo.");
    }

    // Check that subdir is part of the repo
    let (_, log_out) = crate::git_utils::try_run_git(
        &["log", "-1", "--date=default", "--", &subdir],
        &ctx.repo_root,
    );
    if log_out.trim().is_empty() {
        anyhow::bail!("The subdir '{subdir}' is not part of this repo.");
    }

    let remote_val = remote.unwrap_or_else(|| "none".to_string());
    let branch_val = branch.unwrap_or_else(|| get_default_branch(&ctx));
    let method_val = method.unwrap_or_else(|| "merge".to_string());

    // Write .gitrepo file
    let gitrepo_path_str = gitrepo_path.to_string_lossy().into_owned();

    crate::gitrepo::write_new_gitrepo(
        &gitrepo_path,
        &remote_val,
        &branch_val,
        "",   // commit = empty
        None, // parent = not written (upstream_head_commit is empty)
        &method_val,
        VERSION,
        &ctx.repo_root,
    )?;

    run_git(&["add", "-f", "--", &gitrepo_path_str], &ctx.repo_root)?;

    // Build commit message
    let short_head = crate::git_utils::rev_parse_short("HEAD", &ctx.repo_root)
        .unwrap_or_else(|| "none".to_string());
    let commit_msg = format!(
        "git subrepo init {subdir}\n\nsubrepo:\n  subdir:   \"{subdir}\"\n  merged:   \"{short_head}\"\nupstream:\n  origin:   \"{remote_val}\"\n  branch:   \"{branch_val}\"\n  commit:   \"none\"\ngit-subrepo:\n  version:  \"{VERSION}\"\n  origin:   \"???\"\n  commit:   \"???\"\n"
    );

    run_git(&["commit", "-m", &commit_msg], &ctx.repo_root)?;

    // Update commit ref
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/commit"),
            &ctx.original_head_commit,
        ],
        &ctx.repo_root,
    )?;

    if remote_val == "none" {
        println!("Subrepo created from '{subdir}' (with no remote).");
    } else {
        println!("Subrepo created from '{subdir}' with remote '{remote_val}' ({branch_val}).");
    }

    Ok(())
}

fn get_default_branch(ctx: &Context) -> String {
    let (ok, out) =
        crate::git_utils::try_run_git(&["config", "--get", "init.defaultbranch"], &ctx.repo_root);
    if ok && !out.is_empty() {
        out
    } else {
        "master".to_string()
    }
}
