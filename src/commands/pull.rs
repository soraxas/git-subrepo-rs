use crate::commands::{
    Context, assert_clean_for, delete_branch_and_worktree, edit_message_in_editor,
    normalize_subdir, subrepo_branch, subrepo_fetch_with_pb,
};
use crate::encode::encode_subdir;
use crate::git_utils::{run_git, run_git_interactive, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;

/// Data produced by the parallel prepare phase; consumed by the sequential commit phase.
pub struct PullPrepared {
    pub subdir: String,
    pub subref: String,
    pub branch_name: String,
    pub upstream_head: String,
    pub remote: String,
    pub branch: String,
    pub join_method: String,
    pub commit_msg: String,
    pub no_edit: bool,
    pub stage_only: bool,
    pub verify: bool,
    pub update_remote: Option<String>,
    pub update_branch: Option<String>,
    /// Number of upstream commits being pulled in (0 = update-only / force re-pull).
    pub upstream_commit_count: usize,
}

/// Full single-subrepo pull (used when not --all).
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
    no_edit: bool,
    stage_only: bool,
    verify: bool,
) -> Result<()> {
    let mut ctx = Context::new()?;
    ctx.quiet = quiet;
    assert_clean_for("pull", &ctx)?;

    if let Some(prepared) = prepare(
        &ctx,
        subdir,
        branch_override,
        remote_override,
        force,
        method,
        quiet,
        update,
        message,
        no_edit,
        stage_only,
        verify,
        None,
    )? {
        commit_prepared(&ctx, prepared, quiet)?;
    }
    Ok(())
}

/// Phase 1 (parallelisable): fetch upstream, create subrepo branch, rebase in worktree.
/// Returns None when the subrepo is already up-to-date.
/// `pb`: optional caller-owned spinner to reuse (avoids double spinners in --all mode).
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    ctx: &Context,
    subdir: String,
    branch_override: Option<String>,
    remote_override: Option<String>,
    force: bool,
    method: Option<String>,
    quiet: bool,
    update: bool,
    message: Option<String>,
    no_edit: bool,
    stage_only: bool,
    verify: bool,
    pb: Option<indicatif::ProgressBar>,
) -> Result<Option<PullPrepared>> {
    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    if !gitrepo_path.exists() {
        anyhow::bail!("No '{subdir}/.gitrepo' file.");
    }
    let mut cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    let override_remote = remote_override.clone();
    let override_branch = branch_override.clone();

    if let Some(r) = remote_override {
        cfg.remote = r;
    }
    if let Some(b) = branch_override {
        cfg.branch = b;
    }
    if let Some(m) = method {
        cfg.method = m;
    }

    let upstream_head = subrepo_fetch_with_pb(ctx, &cfg.remote, &cfg.branch, &subref, pb.as_ref())?;

    if upstream_head == cfg.commit && !force && !update {
        if !quiet {
            println!("Subrepo '{subdir}' is up to date.");
        }
        return Ok(None);
    }

    // Count how many new upstream commits are coming in.
    let upstream_commit_count = if cfg.commit.is_empty() {
        // fresh clone-style pull — count all commits in fetch ref
        let fetch_ref = format!("refs/subrepo/{subref}/fetch");
        let (ok, out) = try_run_git(&["rev-list", "--count", &fetch_ref], &ctx.repo_root);
        if ok {
            out.trim().parse().unwrap_or(0)
        } else {
            0
        }
    } else {
        let range = format!("{}..{}", cfg.commit, upstream_head);
        let (ok, out) = try_run_git(&["rev-list", "--count", &range], &ctx.repo_root);
        if ok {
            out.trim().parse().unwrap_or(0)
        } else {
            0
        }
    };

    let branch_name = format!("subrepo/{subref}");

    // Delete any existing branch/worktree leftover
    delete_branch_and_worktree(ctx, &subdir, &subref)?;

    // Create subrepo branch (expensive; runs in parallel across siblings)
    let worktree = subrepo_branch(ctx, &subdir, &subref, &cfg.parent, &cfg.method, force)?;

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

    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/branch"),
            &branch_name,
        ],
        &ctx.repo_root,
    )?;

    let commit_msg = match message {
        Some(ref m) => m.clone(),
        None => build_pull_commit_message(
            &subdir,
            &branch_name,
            &cfg.remote,
            &cfg.branch,
            &upstream_head,
            ctx,
        ),
    };
    // Store whether message was explicitly provided; commit_prepared opens editor if needed.
    let has_explicit_message = message.is_some();

    Ok(Some(PullPrepared {
        subdir,
        subref,
        branch_name,
        upstream_head,
        remote: cfg.remote,
        branch: cfg.branch,
        join_method: cfg.method,
        commit_msg,
        no_edit: no_edit || has_explicit_message,
        stage_only,
        verify: verify || cfg.verify,
        update_remote: if update { override_remote } else { None },
        update_branch: if update { override_branch } else { None },
        upstream_commit_count,
    }))
}

/// Action chosen by the user (or inferred from flags) for how to finalise a pull.
pub enum CommitAction {
    /// Commit immediately with this message.
    Commit(String),
    /// Stage changes but do not commit (leave index dirty).
    StageOnly,
}

/// Phase 2 (sequential): commit (or stage) the prepared content into the main repo.
pub fn commit_prepared(ctx: &Context, prepared: PullPrepared, quiet: bool) -> Result<()> {
    let PullPrepared {
        ref subdir,
        ref subref,
        ref branch_name,
        ref upstream_head,
        ref remote,
        ref branch,
        ref join_method,
        commit_msg,
        no_edit,
        stage_only,
        verify,
        ref update_remote,
        ref update_branch,
        upstream_commit_count,
    } = prepared;

    // Stage first so we can accurately detect other staged content.
    stage_subrepo_content(
        ctx,
        subdir,
        subref,
        branch_name,
        remote,
        branch,
        upstream_head,
        join_method,
        update_remote.as_deref(),
        update_branch.as_deref(),
    )?;

    let extras = other_staged_subrepos(ctx, subdir);

    let action = if stage_only {
        CommitAction::StageOnly
    } else if no_edit && extras.is_empty() {
        CommitAction::Commit(commit_msg)
    } else if no_edit && !extras.is_empty() {
        warn_extra_staged(&extras, subdir);
        CommitAction::Commit(edit_message_in_editor(&commit_msg)?)
    } else if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        prompt_commit_action(
            commit_msg,
            subdir,
            remote,
            branch,
            upstream_commit_count,
            &extras,
        )?
    } else {
        CommitAction::Commit(edit_message_in_editor(&commit_msg)?)
    };

    match action {
        CommitAction::Commit(msg) => {
            let mut args = vec!["commit"];
            if !verify {
                args.push("--no-verify");
            }
            args.extend(["-m", &msg]);
            run_git_interactive(&args, &ctx.repo_root)?;
            // update commit ref
            run_git(
                &[
                    "update-ref",
                    &format!("refs/subrepo/{subref}/commit"),
                    branch_name,
                ],
                &ctx.repo_root,
            )?;
            // Remove worktree
            crate::commands::delete_branch_and_worktree(ctx, subdir, subref)?;
            if !quiet {
                use colored::Colorize;
                println!(
                    "Subrepo '{}' pulled from '{}' ({}).",
                    subdir.bold().bright_yellow(),
                    remote.bright_blue(),
                    branch.cyan()
                );
            }
        }
        CommitAction::StageOnly => {
            crate::commands::delete_branch_and_worktree(ctx, subdir, subref)?;
            use colored::Colorize;
            println!(
                "{} Changes for '{}' are staged. Review with {} then commit manually.",
                "ℹ".bright_cyan().bold(),
                subdir.bold().bright_yellow(),
                "git diff --cached".bright_cyan(),
            );
        }
    }
    Ok(())
}

/// Returns subrepo subdirs staged from previous `--stage-only` ops, excluding `current`.
fn other_staged_subrepos(ctx: &Context, current_subdir: &str) -> Vec<String> {
    let (ok, out) = crate::git_utils::try_run_git(
        &["diff-index", "--cached", "--name-only", "HEAD"],
        &ctx.repo_root,
    );
    if !ok || out.trim().is_empty() {
        return vec![];
    }
    parse_other_staged_roots(&out, current_subdir)
}

/// Pure helper: given raw `git diff-index --cached --name-only HEAD` output, return the
/// distinct top-level directory names that are NOT under `current_subdir`.
/// Kept separate so it can be unit-tested without a real git repo.
pub(crate) fn parse_other_staged_roots(output: &str, current_subdir: &str) -> Vec<String> {
    let prefix = format!("{current_subdir}/");
    let mut roots: std::collections::BTreeSet<String> = Default::default();
    for line in output.lines().filter(|l| !l.trim().is_empty()) {
        if line.starts_with(&prefix) || line == current_subdir {
            continue;
        }
        if let Some(root) = std::path::Path::new(line)
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            && root != current_subdir
        {
            roots.insert(root);
        }
    }
    roots.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::parse_other_staged_roots;

    #[test]
    fn no_staged_files_returns_empty() {
        assert!(parse_other_staged_roots("", "echo360").is_empty());
        assert!(parse_other_staged_roots("   \n  \n", "echo360").is_empty());
    }

    #[test]
    fn only_current_subrepo_staged_returns_empty() {
        let output = "echo360/.gitrepo\necho360/README.md\necho360/src/main.rs\n";
        assert!(parse_other_staged_roots(output, "echo360").is_empty());
    }

    #[test]
    fn other_subrepos_detected() {
        let output = "ooo/.gitrepo\nooo/README.md\ndimensify/Cargo.toml\ndimensify/.gitrepo\necho360/src/main.rs\n";
        let mut result = parse_other_staged_roots(output, "echo360");
        result.sort();
        assert_eq!(result, vec!["dimensify", "ooo"]);
    }

    #[test]
    fn mix_current_and_others() {
        let output = "foo/.gitrepo\nfoo/a.rs\nbar/.gitrepo\nbar/b.rs\nbaz/c.rs\n";
        let mut result = parse_other_staged_roots(output, "foo");
        result.sort();
        assert_eq!(result, vec!["bar", "baz"]);
    }

    #[test]
    fn root_level_file_not_in_any_subrepo_included() {
        // A file like `me` at root gets root component "me", which is returned.
        let output = "me\nooo/.gitrepo\n";
        let mut result = parse_other_staged_roots(output, "echo360");
        result.sort();
        assert_eq!(result, vec!["me", "ooo"]);
    }

    #[test]
    fn deduplicated_roots() {
        let output = "lib/a.rs\nlib/b.rs\nlib/c.rs\n";
        assert_eq!(parse_other_staged_roots(output, "current"), vec!["lib"]);
    }

    #[test]
    fn sorted_alphabetically() {
        let output = "zzz/a\naaa/b\nmid/c\n";
        assert_eq!(
            parse_other_staged_roots(output, "other"),
            vec!["aaa", "mid", "zzz"]
        );
    }
}

/// Warn about other staged subrepos that will be bundled into this commit.
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
        "  {} Consider editing the message to cover all of them, or stage '{}' too with {}.\n",
        "→".dimmed(),
        current,
        "--stage-only".bright_cyan(),
    );
}

/// Ask the user how to finalise a pull commit (interactive TTY only).
fn prompt_commit_action(
    default_msg: String,
    subdir: &str,
    remote: &str,
    branch: &str,
    commit_count: usize,
    extras: &[String],
) -> Result<CommitAction> {
    use colored::Colorize;
    use dialoguer::Select;
    use dialoguer::theme::ColorfulTheme;

    let commit_word = if commit_count == 1 {
        "commit"
    } else {
        "commits"
    };
    let count_str = if commit_count == 0 {
        "(re-pull / update)".dimmed().to_string()
    } else {
        format!(
            "{} {} from {} ({})",
            commit_count.to_string().bold().bright_green(),
            commit_word,
            remote.bright_blue(),
            branch.cyan(),
        )
    };
    println!(
        "\n  {} {}: {}\n",
        "↓  Pull".bold().bright_cyan(),
        subdir.bold().bright_yellow(),
        count_str,
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
            0 => Ok(CommitAction::Commit(edit_message_in_editor(&default_msg)?)),
            1 => Ok(CommitAction::Commit(default_msg)),
            _ => Ok(CommitAction::StageOnly),
        }
    } else {
        match idx {
            0 => Ok(CommitAction::Commit(edit_message_in_editor(&default_msg)?)),
            _ => Ok(CommitAction::StageOnly),
        }
    }
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
        "git subrepo pull {subdir}\n\nsubrepo:\n  subdir:   \"{subdir}\"\n  merged:   \"{merged}\"\nupstream:\n  origin:   \"{remote}\"\n  branch:   \"{branch}\"\n  commit:   \"{commit}\"\ngit-subrepo:\n  version:  \"{ver}\"\n"
    )
}

/// Stage the subrepo content into the main repo index (no commit).
/// Called by `commit_prepared` before either committing or leaving staged.
#[allow(clippy::too_many_arguments)]
fn stage_subrepo_content(
    ctx: &Context,
    subdir: &str,
    _subref: &str,
    subrepo_commit_ref: &str,
    remote: &str,
    branch: &str,
    upstream_head_commit: &str,
    join_method: &str,
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
        let (cat_ok, cat_out) = try_run_git(
            &[
                "cat-file",
                "-p",
                &format!("{}:{}", ctx.original_head_commit, gitrepo_rel),
            ],
            &ctx.repo_root,
        );
        if cat_ok && !cat_out.is_empty() {
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

    Ok(())
}
