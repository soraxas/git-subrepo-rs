use crate::commands::{
    Context, assert_clean_for_clone, edit_message_in_editor, normalize_subdir, subrepo_fetch,
};
use crate::encode::encode_subdir;
use crate::git_utils::{run_git, run_git_interactive, try_run_git};
use anyhow::Result;

#[allow(clippy::too_many_arguments)]
pub fn run(
    remote: String,
    subdir_opt: Option<String>,
    branch_opt: Option<String>,
    force: bool,
    method: Option<String>,
    quiet: bool,
    message: Option<String>,
    no_edit: bool,
    stage_only: bool,
) -> Result<()> {
    let mut ctx = Context::new()?;
    ctx.quiet = quiet;

    // Check HEAD exists (can't clone into empty repo)
    let (head_ok, _) = try_run_git(&["rev-parse", "HEAD"], &ctx.repo_root);
    if !head_ok {
        eprintln!("git-subrepo: You can't clone into an empty repository");
        std::process::exit(1);
    }

    // Determine subdir first so we can scope the clean check to the target only.
    // This allows staged changes in *other* subrepo dirs (e.g. a previous --stage-only clone).
    let subdir = match subdir_opt {
        Some(s) => normalize_subdir(&s),
        None => guess_subdir(&remote)?,
    };

    // Only block if the TARGET subdir is dirty or if staged changes are in non-subrepo paths.
    // This allows chaining multiple --stage-only clones before a single batch commit.
    assert_clean_for_clone(&ctx, &subdir)?;

    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");

    // Check subdir non-empty before any network operations (only for new clone)
    let subdir_path = ctx.repo_root.join(&subdir);
    if !gitrepo_path.exists() && subdir_path.exists() {
        let entries: Vec<_> = std::fs::read_dir(&subdir_path)
            .map(|r| r.collect::<Vec<_>>())
            .unwrap_or_default();
        if !entries.is_empty() {
            anyhow::bail!("The subdir '{}' exists and is not empty.", subdir);
        }
    }

    // Determine branch (only for non-force path; reclone will fetch it)
    let subrepo_branch = if let Some(ref b) = branch_opt {
        b.clone()
    } else if !force || !gitrepo_path.exists() {
        get_upstream_head_branch(&remote, &ctx)?
    } else {
        String::new() // will be determined in reclone path
    };

    let join_method = method.as_deref().unwrap_or("merge").to_string();

    // Handle reclone (--force with existing .gitrepo)
    if gitrepo_path.exists() && force {
        // Read current state to check if already up to date
        let existing_cfg = crate::gitrepo::read_gitrepo(&gitrepo_path, &ctx.repo_root).ok();

        // Fetch upstream first
        let fetch_branch = if let Some(b) = branch_opt.as_ref() {
            b.clone()
        } else {
            get_upstream_head_branch(&remote, &ctx)?
        };

        let upstream_head = subrepo_fetch(&ctx, &remote, &fetch_branch, &subref)?;

        // Check if already up to date
        if let Some(ref cfg) = existing_cfg
            && upstream_head == cfg.commit
        {
            if !quiet {
                println!("Subrepo '{subdir}' is up to date.");
            }
            return Ok(());
        }

        // Remove existing subdir for reclone
        let (_, ls) = try_run_git(&["ls-files", "--", &subdir], &ctx.repo_root);
        if !ls.trim().is_empty() {
            run_git(&["rm", "-r", "--", &subdir], &ctx.repo_root)?;
        }

        // Determine join method
        let join_method = method
            .as_deref()
            .unwrap_or(
                existing_cfg
                    .as_ref()
                    .map(|c| c.method.as_str())
                    .unwrap_or("merge"),
            )
            .to_string();

        // Create subdir
        std::fs::create_dir_all(ctx.repo_root.join(&subdir))?;

        // Read in upstream content
        let prefix = format!("{subdir}/");
        run_git(
            &["read-tree", "--prefix", &prefix, "-u", &upstream_head],
            &ctx.repo_root,
        )?;

        // Determine branch to use
        let actual_branch = branch_opt
            .as_deref()
            .map(|b| b.to_string())
            .unwrap_or(fetch_branch.clone());

        // Write .gitrepo file
        let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
        let gitrepo_path_str = gitrepo_path.to_string_lossy().into_owned();

        crate::gitrepo::write_new_gitrepo(
            &gitrepo_path,
            &remote,
            &actual_branch,
            &upstream_head,
            Some(&ctx.original_head_commit),
            &join_method,
            env!("CARGO_PKG_VERSION"),
            &ctx.repo_root,
        )?;

        run_git(&["add", "-f", "--", &gitrepo_path_str], &ctx.repo_root)?;
        let default_msg =
            build_clone_commit_message(&subdir, &upstream_head, &remote, &actual_branch, &ctx);
        let commit_msg = message.unwrap_or(default_msg);

        do_clone_commit(
            &ctx,
            &subdir,
            &subref,
            &upstream_head,
            commit_msg,
            no_edit,
            stage_only,
            quiet,
        )?;

        if !quiet && !stage_only {
            println!("Subrepo '{remote}' ({actual_branch}) recloned into '{subdir}'.");
        }
        return Ok(());
    }

    // Fetch upstream
    let upstream_head = subrepo_fetch(&ctx, &remote, &subrepo_branch, &subref)?;

    // Create subdir
    std::fs::create_dir_all(ctx.repo_root.join(&subdir))?;

    // Remove subdir from index if it has files
    let (_, ls_out) = try_run_git(&["ls-files", "--", &subdir], &ctx.repo_root);
    if !ls_out.trim().is_empty() {
        run_git(&["rm", "-r", "--", &subdir], &ctx.repo_root)?;
    }

    // Read in upstream content
    let prefix = format!("{subdir}/");
    run_git(
        &["read-tree", "--prefix", &prefix, "-u", &upstream_head],
        &ctx.repo_root,
    )?;

    // Write .gitrepo file
    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let gitrepo_path_str = gitrepo_path.to_string_lossy().into_owned();

    crate::gitrepo::write_new_gitrepo(
        &gitrepo_path,
        &remote,
        &subrepo_branch,
        &upstream_head,
        Some(&ctx.original_head_commit), // parent = HEAD before clone
        &join_method,
        env!("CARGO_PKG_VERSION"),
        &ctx.repo_root,
    )?;

    run_git(&["add", "-f", "--", &gitrepo_path_str], &ctx.repo_root)?;

    let default_msg =
        build_clone_commit_message(&subdir, &upstream_head, &remote, &subrepo_branch, &ctx);
    let commit_msg = message.unwrap_or(default_msg);

    do_clone_commit(
        &ctx,
        &subdir,
        &subref,
        &upstream_head,
        commit_msg,
        no_edit,
        stage_only,
        quiet,
    )?;

    if !quiet && !stage_only {
        println!("Subrepo '{remote}' ({subrepo_branch}) cloned into '{subdir}'.");
    }

    Ok(())
}

/// Finalise a clone: either commit (with optional editor) or leave staged.
/// Outcome of the interactive clone prompt.
enum CloneAction {
    Commit(String),
    StageOnly,
}

/// Returns the list of *other* subrepo subdirs that are already staged (from previous
/// `--stage-only` operations), excluding `current_subdir`.
fn other_staged_subrepos(ctx: &Context, current_subdir: &str) -> Vec<String> {
    let (ok, out) = crate::git_utils::try_run_git(
        &["diff-index", "--cached", "--name-only", "HEAD"],
        &ctx.repo_root,
    );
    if !ok || out.trim().is_empty() {
        return vec![];
    }
    crate::commands::pull::parse_other_staged_roots(&out, current_subdir)
}

#[allow(clippy::too_many_arguments)]
fn do_clone_commit(
    ctx: &Context,
    subdir: &str,
    subref: &str,
    upstream_head: &str,
    commit_msg: String,
    no_edit: bool,
    stage_only: bool,
    quiet: bool,
) -> Result<()> {
    use colored::Colorize;

    let extras = other_staged_subrepos(ctx, subdir);

    let action = if stage_only {
        CloneAction::StageOnly
    } else if no_edit && extras.is_empty() {
        // Only auto-commit with default message when there's nothing else staged.
        CloneAction::Commit(commit_msg)
    } else if no_edit && !extras.is_empty() {
        // --no-edit but other staged content exists: must edit to write a batch message.
        warn_extra_staged(&extras, subdir);
        CloneAction::Commit(edit_message_in_editor(&commit_msg)?)
    } else if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        prompt_clone_commit_action(commit_msg, subdir, &extras)?
    } else {
        CloneAction::Commit(edit_message_in_editor(&commit_msg)?)
    };

    match action {
        CloneAction::StageOnly => {
            if !quiet {
                println!(
                    "{} Changes for '{}' are staged. Review with {} then commit manually.",
                    "ℹ".bright_cyan().bold(),
                    subdir.bold().bright_yellow(),
                    "git diff --cached".bright_cyan(),
                );
            }
        }
        CloneAction::Commit(msg) => {
            run_git_interactive(&["commit", "-m", &msg], &ctx.repo_root)?;
            run_git(
                &[
                    "update-ref",
                    &format!("refs/subrepo/{subref}/commit"),
                    upstream_head,
                ],
                &ctx.repo_root,
            )?;
        }
    }

    Ok(())
}

/// Print a warning about other staged subrepos that will be bundled in the commit.
fn warn_extra_staged(extras: &[String], current: &str) {
    use colored::Colorize;
    println!(
        "\n  {} {} other staged subrepo{} will also be included in this commit:",
        "⚠".yellow().bold(),
        extras.len().to_string().yellow().bold(),
        if extras.len() == 1 { "" } else { "s" },
    );
    for e in extras {
        println!("    {} {}", "•".dimmed(), e.bright_yellow());
    }
    println!(
        "  {} Consider editing the commit message to cover all of them, or use\n  {} to stage '{}' too.\n",
        "→".dimmed(),
        "--stage-only".bright_cyan(),
        current,
    );
}

/// Interactive 3-way prompt for clone commit finalisation.
fn prompt_clone_commit_action(
    default_msg: String,
    subdir: &str,
    extras: &[String],
) -> Result<CloneAction> {
    use colored::Colorize;
    use dialoguer::Select;
    use dialoguer::theme::ColorfulTheme;

    println!(
        "\n  {} {}\n",
        "↓  Clone".bold().bright_cyan(),
        subdir.bold().bright_yellow(),
    );

    if !extras.is_empty() {
        warn_extra_staged(extras, subdir);
    }

    let choices: Vec<&str> = if extras.is_empty() {
        vec![
            "Edit commit message (open editor)",
            "Commit with default message",
            "Stage only — don't commit yet",
        ]
    } else {
        // When other staged subrepos exist, "Commit with default message" is misleading.
        vec![
            "Edit commit message (covers all staged subrepos)",
            "Stage only — don't commit yet",
        ]
    };

    let idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("How to finalise?")
        .items(&choices)
        .default(0)
        .interact()?;

    if extras.is_empty() {
        match idx {
            0 => Ok(CloneAction::Commit(edit_message_in_editor(&default_msg)?)),
            1 => Ok(CloneAction::Commit(default_msg)),
            _ => Ok(CloneAction::StageOnly),
        }
    } else {
        match idx {
            0 => Ok(CloneAction::Commit(edit_message_in_editor(&default_msg)?)),
            _ => Ok(CloneAction::StageOnly),
        }
    }
}

fn guess_subdir(remote: &str) -> Result<String> {
    let dir = remote.trim_end_matches('/').trim_end_matches(".git");
    let dir = dir.rsplit('/').next().unwrap_or(dir);
    if dir.is_empty() {
        anyhow::bail!("Can't determine subdir from '{remote}'.")
    }
    if dir
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        Ok(dir.to_string())
    } else {
        anyhow::bail!("Can't determine subdir from '{remote}'.")
    }
}

fn get_upstream_head_branch(remote: &str, ctx: &Context) -> Result<String> {
    let (ok, output) = try_run_git(&["ls-remote", "--symref", remote], &ctx.repo_root);
    if !ok || output.is_empty() {
        anyhow::bail!("Command failed: 'git ls-remote --symref {remote}'.");
    }

    for line in output.lines() {
        // Format: "ref: refs/heads/master\tHEAD"
        if line.starts_with("ref:") && (line.ends_with("HEAD") || line.contains("\tHEAD")) {
            // Split on tab to get "ref: refs/heads/master"
            let ref_part = line.split('\t').next().unwrap_or(line);
            let ref_part = ref_part.trim_start_matches("ref:").trim();
            if let Some(branch) = ref_part.strip_prefix("refs/heads/") {
                return Ok(branch.to_string());
            }
        }
    }

    anyhow::bail!("Problem finding remote default head branch.")
}

fn build_clone_commit_message(
    subdir: &str,
    upstream_head: &str,
    remote: &str,
    branch: &str,
    ctx: &Context,
) -> String {
    let short = crate::git_utils::rev_parse_short(upstream_head, &ctx.repo_root)
        .unwrap_or_else(|| "none".to_string());
    format!(
        "git subrepo clone {subdir}\n\nsubrepo:\n  subdir:   \"{subdir}\"\n  merged:   \"{short}\"\nupstream:\n  origin:   \"{remote}\"\n  branch:   \"{branch}\"\n  commit:   \"{short}\"\ngit-subrepo:\n  version:  \"{}\"\n  origin:   \"???\"\n  commit:   \"???\"\n",
        env!("CARGO_PKG_VERSION")
    )
}
