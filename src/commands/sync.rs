use crate::commands::{Context, normalize_subdir};
use crate::git_utils::try_run_git;
use crate::gitrepo::read_gitrepo;
use anyhow::Result;
use colored::Colorize;
use std::collections::HashMap;

/// Normalize a remote URL so that case-insensitive GitHub URLs and .git suffixes
/// all map to the same key.
fn normalize_remote(remote: &str) -> String {
    let s = remote.trim().to_lowercase();
    // git@github.com:org/repo → github.com/org/repo
    let s = if let Some(rest) = s.strip_prefix("git@") {
        rest.replacen(':', "/", 1)
    } else {
        s.trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string()
    };
    s.trim_end_matches(".git").to_string()
}

#[derive(Debug, Clone)]
struct SubrepoInfo {
    subdir: String,
    remote: String,
    branch: String,
    commit: String,
}

/// Returns true if `ancestor` is a git ancestor of `descendant` (or equal).
fn is_ancestor(ancestor: &str, descendant: &str, repo_root: &std::path::Path) -> bool {
    if ancestor.is_empty() || descendant.is_empty() {
        return false;
    }
    if ancestor == descendant {
        return true;
    }
    let (ok, _) = try_run_git(
        &["merge-base", "--is-ancestor", ancestor, descendant],
        repo_root,
    );
    ok
}

/// Compare the committed content of two subdirs, ignoring .gitrepo.
/// Returns a human-readable summary of the content difference.
fn content_diff_summary(ctx: &Context, subdir_a: &str, subdir_b: &str) -> String {
    let tree_a_ref = format!("HEAD:{subdir_a}");
    let tree_b_ref = format!("HEAD:{subdir_b}");

    let (ok_a, tree_a) = try_run_git(&["rev-parse", &tree_a_ref], &ctx.repo_root);
    let (ok_b, tree_b) = try_run_git(&["rev-parse", &tree_b_ref], &ctx.repo_root);
    if !ok_a || !ok_b {
        return "unable to compare (subdir not found in HEAD)".to_string();
    }
    let tree_a = tree_a.trim();
    let tree_b = tree_b.trim();

    if tree_a == tree_b {
        return "identical".to_string();
    }

    // diff-tree gives per-file changes between two tree objects
    let (ok, out) = try_run_git(
        &[
            "diff-tree",
            "--no-commit-id",
            "-r",
            "--name-only",
            tree_a,
            tree_b,
        ],
        &ctx.repo_root,
    );
    if !ok {
        return "unable to compare".to_string();
    }

    // Filter out .gitrepo (it always differs — it holds the commit SHA)
    let changed: Vec<&str> = out
        .lines()
        .filter(|l| !l.trim().is_empty() && l.trim() != ".gitrepo")
        .collect();

    if changed.is_empty() {
        "identical (only .gitrepo differs)".to_string()
    } else {
        let n = changed.len();
        let s = if n == 1 { "file" } else { "files" };
        format!("{n} {s} differ")
    }
}

pub fn run(no_edit: bool, verify: bool, quiet: bool) -> Result<()> {
    let ctx = Context::new()?;

    // Collect all subrepos tracked in the repo
    let (ok, files) = try_run_git(&["ls-files"], &ctx.repo_root);
    if !ok {
        anyhow::bail!("Could not list tracked files.");
    }

    let subdirs: Vec<String> = files
        .lines()
        .filter(|f| f.ends_with("/.gitrepo"))
        .map(|f| f.trim_end_matches("/.gitrepo").to_string())
        .collect();

    if subdirs.is_empty() {
        if !quiet {
            println!("{}", "No subrepos found.".yellow());
        }
        return Ok(());
    }

    let mut subrepos: Vec<SubrepoInfo> = Vec::new();
    for subdir in &subdirs {
        let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");
        if let Ok(cfg) = read_gitrepo(&gitrepo_path, &ctx.repo_root)
            && !cfg.remote.is_empty()
        {
            subrepos.push(SubrepoInfo {
                subdir: normalize_subdir(subdir),
                remote: cfg.remote.clone(),
                branch: cfg.branch.clone(),
                commit: cfg.commit.clone(),
            });
        }
    }

    // Group by (normalized_remote, branch)
    let mut groups: HashMap<(String, String), Vec<SubrepoInfo>> = HashMap::new();
    for s in subrepos {
        let key = (normalize_remote(&s.remote), s.branch.clone());
        groups.entry(key).or_default().push(s);
    }

    // Find groups with >1 member and at least 2 distinct commits
    let mut divergent: Vec<Vec<SubrepoInfo>> = groups
        .into_values()
        .filter(|members| {
            members.len() > 1 && {
                let first = &members[0].commit;
                members.iter().any(|m| &m.commit != first)
            }
        })
        .collect();

    if divergent.is_empty() {
        if !quiet {
            println!(
                "{}",
                "All subrepos with shared remotes are in sync.".green()
            );
        }
        return Ok(());
    }

    // Sort groups by normalized remote for stable output
    divergent.sort_by_key(|members| normalize_remote(&members[0].remote));

    if !quiet {
        println!(
            "{}",
            format!(
                "{} group(s) of subrepos share a remote but have diverged commits:",
                divergent.len()
            )
            .yellow()
            .bold()
        );
        println!();
    }

    // Classify each member: who's ahead, who's behind
    // "ahead" = someone else's commit is an ancestor of mine
    // "behind" = my commit is an ancestor of someone else's

    struct GroupInfo<'a> {
        norm: String,
        branch: String,
        members: &'a Vec<SubrepoInfo>,
        behind: Vec<&'a SubrepoInfo>,
        suggested: Vec<&'a SubrepoInfo>,
    }

    let groups_info: Vec<GroupInfo> = divergent
        .iter()
        .map(|members| {
            let norm = normalize_remote(&members[0].remote);
            let branch = members[0].branch.clone();

            let mut leaders: Vec<&SubrepoInfo> = Vec::new();
            let mut behind: Vec<&SubrepoInfo> = Vec::new();

            'outer: for m in members {
                for other in members {
                    if other.subdir == m.subdir {
                        continue;
                    }
                    if is_ancestor(&m.commit, &other.commit, &ctx.repo_root)
                        && m.commit != other.commit
                    {
                        behind.push(m);
                        continue 'outer;
                    }
                }
                leaders.push(m);
            }

            let suggested: Vec<&SubrepoInfo> = if behind.is_empty() {
                members.iter().collect()
            } else {
                behind.to_vec()
            };

            let _ = leaders; // used only for tag computation below
            GroupInfo {
                norm,
                branch,
                members,
                behind,
                suggested,
            }
        })
        .collect();

    // Pass 1: display all groups with content diff
    if !quiet {
        for gi in &groups_info {
            println!(
                "  {} {}",
                "Shared remote:".bold(),
                format!("{}  (branch: {})", gi.norm, gi.branch).bright_blue()
            );
            let has_clear_order = !gi.behind.is_empty();
            for m in gi.members.iter() {
                let short = if m.commit.len() >= 7 {
                    &m.commit[..7]
                } else {
                    &m.commit
                };
                let is_behind = gi.behind.iter().any(|b| b.subdir == m.subdir);
                let tag = if is_behind {
                    " [behind]".red().to_string()
                } else if has_clear_order {
                    " [ahead]".green().to_string()
                } else {
                    " [diverged]".yellow().to_string()
                };
                println!(
                    "    {:<40}  @ {}{}",
                    m.subdir.bright_yellow(),
                    short.yellow(),
                    tag
                );
            }
            println!();
            // Content comparison: compare each member against the first
            let ref_subdir = &gi.members[0].subdir;
            for m in gi.members.iter().skip(1) {
                let diff = content_diff_summary(&ctx, ref_subdir, &m.subdir);
                let diff_colored = if diff == "identical" || diff.starts_with("identical") {
                    diff.green().to_string()
                } else if diff.starts_with("unable") {
                    diff.dimmed().to_string()
                } else {
                    diff.red().to_string()
                };
                println!(
                    "  {}  {} ↔ {}:  {}",
                    "Content diff:".dimmed(),
                    ref_subdir.bright_yellow(),
                    m.subdir.bright_yellow(),
                    diff_colored
                );
            }
            println!();
            let suggested_names: Vec<&str> =
                gi.suggested.iter().map(|m| m.subdir.as_str()).collect();
            println!(
                "  {}  {}",
                "Suggested pull:".dimmed(),
                suggested_names.join(", ").bright_cyan()
            );
            println!();
        }
    }

    // Pass 2: collect which subrepos to pull (prompt per group, or auto with no_edit)
    let mut to_pull: Vec<String> = Vec::new();

    for gi in &groups_info {
        let subdirs_to_pull: Vec<String> = if no_edit {
            gi.suggested.iter().map(|m| m.subdir.clone()).collect()
        } else {
            let mut member_names: Vec<String> =
                gi.members.iter().map(|m| m.subdir.clone()).collect();
            member_names.push("Pull all above".to_string());
            member_names.push("Skip this group".to_string());

            let defaults: Vec<bool> = gi
                .members
                .iter()
                .map(|m| gi.suggested.iter().any(|s| s.subdir == m.subdir))
                .chain(std::iter::once(false))
                .chain(std::iter::once(false))
                .collect();

            let prompt = format!("Which subrepos to pull for {}?", gi.norm.bright_blue());

            use dialoguer::MultiSelect;
            let selection = MultiSelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt(&prompt)
                .items(&member_names)
                .defaults(&defaults)
                .interact_opt();

            // Gracefully handle non-terminal environments
            let selection = match selection {
                Ok(s) => s,
                Err(_) => {
                    if !quiet {
                        eprintln!(
                            "  {} Not running in a terminal — use {} to auto-accept suggestions.",
                            "hint:".dimmed(),
                            "-n / --no-edit".bright_cyan()
                        );
                    }
                    None
                }
            };

            match selection {
                None => Vec::new(), // Ctrl-C or non-terminal → skip
                Some(indices) => {
                    if indices.contains(&(member_names.len() - 1)) {
                        Vec::new()
                    } else if indices.contains(&(member_names.len() - 2)) {
                        gi.members.iter().map(|m| m.subdir.clone()).collect()
                    } else {
                        indices
                            .iter()
                            .filter(|&&i| i < gi.members.len())
                            .map(|&i| gi.members[i].subdir.clone())
                            .collect()
                    }
                }
            }
        };

        to_pull.extend(subdirs_to_pull);
    }

    if to_pull.is_empty() {
        if !quiet {
            println!("{}", "Nothing to sync.".dimmed());
        }
        return Ok(());
    }

    // Execute pulls sequentially
    let total = to_pull.len();
    for (i, subdir) in to_pull.iter().enumerate() {
        if !quiet {
            println!(
                "{}",
                format!("[{}/{}] Pulling '{}'...", i + 1, total, subdir).bright_cyan()
            );
        }
        crate::commands::pull::run(
            subdir.clone(),
            None,  // branch — use .gitrepo
            None,  // remote — use .gitrepo
            false, // force
            None,  // method — use .gitrepo
            quiet,
            false, // update
            None,  // message
            no_edit,
            false, // stage_only
            verify,
        )?;
    }

    Ok(())
}
