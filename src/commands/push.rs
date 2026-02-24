use crate::commands::{
    Context, assert_clean_for, delete_branch_and_worktree, normalize_subdir, subrepo_branch,
};
use crate::encode::encode_subdir;
use crate::git_utils::{commit_in_rev_list, rev_parse, run_git, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;

pub fn run(
    subdir: String,
    branch_arg: Option<String>,
    remote_override: Option<String>,
    force: bool,
    method: Option<String>,
    squash: bool,
    quiet: bool,
    message: Option<String>,
) -> Result<()> {
    let ctx = Context::new()?;
    assert_clean_for("push", &ctx)?;

    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let mut cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    let original_remote = cfg.remote.clone();
    let remote_was_overridden = remote_override.is_some();

    if let Some(r) = remote_override {
        cfg.remote = r;
    }
    if let Some(m) = method {
        cfg.method = m;
    }

    let original_head_commit = ctx.original_head_commit.clone();

    // Fetch or detect new upstream
    let mut new_upstream = false;
    let fetch_branch = branch_arg.as_deref().unwrap_or(&cfg.branch).to_string();
    let upstream_head = {
        let fetch_result = subrepo_fetch_allow_fail(&ctx, &cfg.remote, &fetch_branch, &subref);
        match fetch_result {
            Ok(h) => h,
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("couldn't find remote ref") {
                    new_upstream = true;
                    String::new()
                } else {
                    return Err(e);
                }
            }
        }
    };

    // Check if upstream is ahead of what we have
    if !new_upstream && !force && upstream_head != cfg.commit {
        eprintln!("git-subrepo: There are new changes upstream, you need to pull first.");
        std::process::exit(1);
    }

    let branch_name = format!("subrepo/{subref}");
    let push_branch = branch_arg.as_deref().unwrap_or(&cfg.branch);

    // For squash, use HEAD^ as the subrepo_parent to only create one commit
    let effective_parent = if squash {
        // Resolve HEAD^ to use as subrepo_parent
        let (ok, head_parent) = try_run_git(&["rev-parse", "HEAD^"], &ctx.repo_root);
        if ok && !head_parent.is_empty() {
            head_parent
        } else {
            cfg.parent.clone()
        }
    } else {
        cfg.parent.clone()
    };

    // Delete existing branch
    delete_branch_and_worktree(&ctx, &subdir, &subref)?;

    // Create subrepo branch (worktree not used directly - we push the branch)
    let _worktree = subrepo_branch(&ctx, &subdir, &subref, &effective_parent, &cfg.method)?;

    // Check if there's anything to push
    let branch_head = rev_parse(&branch_name, &ctx.repo_root).unwrap_or_default();

    if !new_upstream && branch_head == upstream_head {
        // No new commits
        delete_branch_and_worktree(&ctx, &subdir, &subref)?;
        if !quiet {
            println!("Subrepo '{subdir}' has no new commits to push.");
        }
        return Ok(());
    }

    // Check branch contains upstream HEAD (unless force)
    if !force
        && !new_upstream
        && !upstream_head.is_empty()
        && !commit_in_rev_list(&upstream_head, &branch_name, &ctx.repo_root)
    {
        anyhow::bail!(
            "Can't commit: '{branch_name}' doesn't contain upstream HEAD: {upstream_head}"
        );
    }

    // Push the branch
    let force_flag = if force { vec!["--force"] } else { vec![] };
    let push_refspec = format!("{branch_name}:{push_branch}");
    let mut push_args = vec!["push"];
    push_args.extend_from_slice(&force_flag);
    push_args.push(&cfg.remote);
    push_args.push(&push_refspec);
    run_git(&push_args, &ctx.repo_root)?;

    // Get new upstream head after push
    let new_upstream_head =
        rev_parse(&branch_name, &ctx.repo_root).unwrap_or_else(|| branch_head.clone());

    // Update push ref
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/push"),
            &new_upstream_head,
        ],
        &ctx.repo_root,
    )?;

    // Delete the branch (before committing)
    delete_branch_and_worktree(&ctx, &subdir, &subref)?;

    // Update .gitrepo with new commit info
    let push_branch_str = push_branch.to_string();
    // Update remote in .gitrepo if it was explicitly overridden or was "none" before
    let update_remote = if remote_was_overridden || original_remote == "none" {
        Some(cfg.remote.as_str())
    } else {
        None
    };
    update_gitrepo_after_push(
        &ctx,
        &subdir,
        &subref,
        update_remote,
        &push_branch_str,
        &new_upstream_head,
        &original_head_commit,
        &cfg.method,
    )?;

    // Build push commit message
    let commit_msg = match message {
        Some(ref m) => m.clone(),
        None => build_push_commit_message(
            &subdir,
            &new_upstream_head,
            &cfg.remote,
            &push_branch_str,
            &ctx,
        ),
    };

    run_git(&["commit", "-m", &commit_msg], &ctx.repo_root)?;

    if !quiet {
        println!(
            "Subrepo '{subdir}' pushed to '{}' ({}).",
            cfg.remote, push_branch_str
        );
    }

    Ok(())
}

fn subrepo_fetch_allow_fail(
    ctx: &Context,
    remote: &str,
    branch: &str,
    subref: &str,
) -> Result<String> {
    let (ok, out) = try_run_git(
        &["fetch", "--no-tags", "--quiet", remote, branch],
        &ctx.repo_root,
    );
    if !ok {
        return Err(anyhow::anyhow!("{out}"));
    }
    let upstream_head = run_git(&["rev-parse", "FETCH_HEAD^0"], &ctx.repo_root)?;
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/fetch"),
            &upstream_head,
        ],
        &ctx.repo_root,
    )?;
    Ok(upstream_head)
}

fn update_gitrepo_after_push(
    ctx: &Context,
    subdir: &str,
    _subref: &str,
    update_remote: Option<&str>,
    branch: &str,
    new_upstream_head: &str,
    original_head_commit: &str,
    method: &str,
) -> Result<()> {
    let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");
    let gitrepo_rel = format!("{subdir}/.gitrepo");
    let gitrepo_path_str = gitrepo_path.to_string_lossy().into_owned();

    if !gitrepo_path.exists() {
        // Try to restore from original_head_commit
        let (cat_ok, cat_out) = crate::git_utils::try_run_git(
            &[
                "cat-file",
                "-p",
                &format!("{}:{}", ctx.original_head_commit, gitrepo_rel),
            ],
            &ctx.repo_root,
        );
        if cat_ok && !cat_out.is_empty() {
            std::fs::write(&gitrepo_path, cat_out + "\n")?;
        }
    }

    crate::gitrepo::update_gitrepo(
        &gitrepo_path,
        update_remote,
        None,
        new_upstream_head,
        Some(original_head_commit),
        method,
        env!("CARGO_PKG_VERSION"),
        &ctx.repo_root,
    )?;

    // Also update branch to the push branch (in case it changed)
    let path_str = gitrepo_path.to_string_lossy().into_owned();
    crate::git_utils::try_run_git(
        &["config", "--file", &path_str, "subrepo.branch", branch],
        &ctx.repo_root,
    );

    run_git(&["add", "-f", "--", &gitrepo_path_str], &ctx.repo_root)?;

    Ok(())
}

fn build_push_commit_message(
    subdir: &str,
    upstream_head: &str,
    remote: &str,
    branch: &str,
    ctx: &Context,
) -> String {
    let short = crate::git_utils::rev_parse_short(upstream_head, &ctx.repo_root)
        .unwrap_or_else(|| "none".to_string());
    let ver = env!("CARGO_PKG_VERSION");
    format!(
        "git subrepo push {subdir}\n\nsubrepo:\n  subdir:   \"{subdir}\"\n  merged:   \"{short}\"\nupstream:\n  origin:   \"{remote}\"\n  branch:   \"{branch}\"\n  commit:   \"{short}\"\ngit-subrepo:\n  version:  \"{ver}\"\n  origin:   \"???\"\n  commit:   \"???\"\n"
    )
}
