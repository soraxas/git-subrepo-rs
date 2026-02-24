use crate::commands::{
    Context, assert_clean_for, delete_branch_and_worktree, normalize_subdir, subrepo_branch,
    subrepo_fetch,
};
use crate::encode::encode_subdir;
use crate::git_utils::{run_git, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub fn run(
    subdir: String,
    branch_override: Option<String>,
    remote_override: Option<String>,
    force: bool,
    method: Option<String>,
    quiet: bool,
    update: bool,
    message: Option<String>,
    edit: bool,
) -> Result<()> {
    let ctx = Context::new()?;
    assert_clean_for("pull", &ctx)?;

    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    if !gitrepo_path.exists() {
        anyhow::bail!("No '{subdir}/.gitrepo' file.");
    }
    let mut cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    let override_remote = remote_override.clone();
    let override_branch = branch_override.clone();

    // Apply overrides
    if let Some(r) = remote_override {
        cfg.remote = r;
    }
    if let Some(b) = branch_override {
        cfg.branch = b;
    }
    if let Some(m) = method {
        cfg.method = m;
    }

    // Fetch upstream
    let upstream_head = subrepo_fetch(&ctx, &cfg.remote, &cfg.branch, &subref)?;

    // Check if up to date (and not force)
    if upstream_head == cfg.commit && !force && !update {
        println!("Subrepo '{subdir}' is up to date.");
        return Ok(());
    }

    let branch_name = format!("subrepo/{subref}");

    // Delete existing branch if any
    delete_branch_and_worktree(&ctx, &subdir, &subref)?;

    // Create subrepo branch
    let worktree = subrepo_branch(&ctx, &subdir, &subref, &cfg.parent, &cfg.method)?;

    // Merge upstream fetch into worktree
    let refs_subrepo_fetch = format!("refs/subrepo/{subref}/fetch");

    if cfg.method == "rebase" {
        let (ok, out) = try_run_git(&["rebase", &refs_subrepo_fetch, &branch_name], &worktree);
        if !ok {
            anyhow::bail!(
                "The \"git rebase\" command failed:\n\n  {}",
                out.replace('\n', "\n  ")
            );
        }
    } else {
        let (ok, out) = try_run_git(&["merge", &refs_subrepo_fetch], &worktree);
        if !ok {
            anyhow::bail!(
                "The \"git merge\" command failed:\n\n  {}",
                out.replace('\n', "\n  ")
            );
        }
    }

    // Update branch ref
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/branch"),
            &branch_name,
        ],
        &ctx.repo_root,
    )?;

    // Commit the merged content
    let commit_msg = match message {
        Some(ref m) => m.clone(),
        None => build_pull_commit_message(
            &subdir,
            &branch_name,
            &cfg.remote,
            &cfg.branch,
            &upstream_head,
            &ctx,
        ),
    };
    let commit_msg = if edit {
        edit_message_in_editor(&commit_msg)?
    } else {
        commit_msg
    };

    // Inline commit (like subrepo_commit but with pull-specific logic)
    let update_remote = if update {
        override_remote.as_deref()
    } else {
        None
    };
    let update_branch = if update {
        override_branch.as_deref()
    } else {
        None
    };

    do_subrepo_commit(
        &ctx,
        &subdir,
        &subref,
        &branch_name,
        &cfg.remote,
        &cfg.branch,
        &upstream_head,
        &cfg.method,
        &commit_msg,
        update_remote,
        update_branch,
    )?;

    if !quiet {
        println!(
            "Subrepo '{subdir}' pulled from '{}' ({}).",
            cfg.remote, cfg.branch
        );
    }

    Ok(())
}

fn edit_message_in_editor(msg: &str) -> Result<String> {
    let tmp = std::env::temp_dir().join(format!("git-subrepo-msg-{}", std::process::id()));
    std::fs::write(&tmp, msg)?;
    let editor = std::env::var("GIT_EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let shell_cmd = format!("{} {}", editor, tmp.display());
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&shell_cmd)
        .status()?;
    if !status.success() {
        anyhow::bail!("Editor exited with non-zero status");
    }
    let result = std::fs::read_to_string(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    Ok(result)
}

fn build_pull_commit_message(
    subdir: &str,
    subrepo_commit_ref: &str,
    remote: &str,
    branch: &str,
    upstream_head: &str,
    ctx: &Context,
) -> String {
    let merged = crate::git_utils::rev_parse_short(subrepo_commit_ref, &ctx.repo_root)
        .unwrap_or_else(|| "none".to_string());
    let commit = crate::git_utils::rev_parse_short(upstream_head, &ctx.repo_root)
        .unwrap_or_else(|| "none".to_string());
    let ver = env!("CARGO_PKG_VERSION");
    format!(
        "git subrepo pull {subdir}\n\nsubrepo:\n  subdir:   \"{subdir}\"\n  merged:   \"{merged}\"\nupstream:\n  origin:   \"{remote}\"\n  branch:   \"{branch}\"\n  commit:   \"{commit}\"\ngit-subrepo:\n  version:  \"{ver}\"\n  origin:   \"???\"\n  commit:   \"???\"\n"
    )
}

#[allow(clippy::too_many_arguments)]
fn do_subrepo_commit(
    ctx: &Context,
    subdir: &str,
    subref: &str,
    subrepo_commit_ref: &str,
    remote: &str,
    branch: &str,
    upstream_head_commit: &str,
    join_method: &str,
    commit_msg: &str,
    update_remote: Option<&str>,
    update_branch: Option<&str>,
) -> Result<()> {
    use crate::git_utils::{rev_exists, rev_parse};

    if !rev_exists(subrepo_commit_ref, &ctx.repo_root) {
        anyhow::bail!("Commit ref '{subrepo_commit_ref}' does not exist.");
    }

    // Remove existing subdir from index
    let (_, ls_out) = try_run_git(&["ls-files", "--", subdir], &ctx.repo_root);
    if !ls_out.trim().is_empty() {
        run_git(&["rm", "-r", "--", subdir], &ctx.repo_root)?;
    }

    // Read in the subrepo content
    let prefix = format!("{subdir}/");
    run_git(
        &["read-tree", "--prefix", &prefix, "-u", subrepo_commit_ref],
        &ctx.repo_root,
    )?;

    // Determine parent (only write when fast-forward: resolved_ref == upstream_head)
    let resolved_ref = rev_parse(subrepo_commit_ref, &ctx.repo_root);
    let parent = if !upstream_head_commit.is_empty() && !subrepo_commit_ref.is_empty() {
        if resolved_ref.as_deref() == Some(upstream_head_commit) {
            Some(ctx.original_head_commit.as_str())
        } else {
            None
        }
    } else {
        None
    };

    let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");
    let gitrepo_path_str = gitrepo_path.to_string_lossy().into_owned();
    let gitrepo_rel = format!("{subdir}/.gitrepo");

    if gitrepo_path.exists() {
        // File exists from read-tree: update specific fields only
        crate::gitrepo::update_gitrepo(
            &gitrepo_path,
            update_remote,
            update_branch,
            upstream_head_commit,
            parent,
            join_method,
            env!("CARGO_PKG_VERSION"),
            &ctx.repo_root,
        )?;
    } else {
        // File does not exist (filter-branch removed it from subrepo branch).
        // Try to restore it from original_head_commit (same as bash's update-gitrepo-file).
        let (cat_ok, cat_out) = try_run_git(
            &[
                "cat-file",
                "-p",
                &format!("{}:{}", ctx.original_head_commit, gitrepo_rel),
            ],
            &ctx.repo_root,
        );
        if cat_ok && !cat_out.is_empty() {
            // Restore old .gitrepo (preserves parent field etc.) then update fields
            std::fs::write(&gitrepo_path, cat_out + "\n")?;
            crate::gitrepo::update_gitrepo(
                &gitrepo_path,
                update_remote,
                update_branch,
                upstream_head_commit,
                parent,
                join_method,
                env!("CARGO_PKG_VERSION"),
                &ctx.repo_root,
            )?;
        } else {
            // Truly new: write from scratch
            crate::gitrepo::write_new_gitrepo(
                &gitrepo_path,
                update_remote.unwrap_or(remote),
                update_branch.unwrap_or(branch),
                upstream_head_commit,
                parent,
                join_method,
                env!("CARGO_PKG_VERSION"),
                &ctx.repo_root,
            )?;
        }
    }

    run_git(&["add", "-f", "--", &gitrepo_path_str], &ctx.repo_root)?;
    run_git(&["commit", "-m", commit_msg], &ctx.repo_root)?;

    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/commit"),
            subrepo_commit_ref,
        ],
        &ctx.repo_root,
    )?;

    // Remove worktree
    let worktree = ctx.worktree_path(subdir);
    if worktree.exists() {
        std::fs::remove_dir_all(&worktree)?;
        try_run_git(&["worktree", "prune"], &ctx.repo_root);
    }

    Ok(())
}
