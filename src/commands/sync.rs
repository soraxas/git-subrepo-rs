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
    let mut to_pull: Vec<String> = Vec::new(); // subdirs chosen for pulling

    for members in &divergent {
        let norm = normalize_remote(&members[0].remote);
        let branch = &members[0].branch;

        // Determine relative ordering: find the "most advanced" commit
        // A commit is "most advanced" if it's a descendant of all others (or none can be compared).
        let mut leaders: Vec<&SubrepoInfo> = Vec::new();
        let mut behind: Vec<&SubrepoInfo> = Vec::new();

        'outer: for m in members {
            for other in members {
                if other.subdir == m.subdir {
                    continue;
                }
                if is_ancestor(&m.commit, &other.commit, &ctx.repo_root) && m.commit != other.commit
                {
                    // m is an ancestor of other → m is behind other
                    behind.push(m);
                    continue 'outer;
                }
            }
            leaders.push(m);
        }

        // Suggest which to pull
        let suggested: Vec<&SubrepoInfo> = if behind.is_empty() {
            members.iter().collect()
        } else {
            behind.to_vec()
        };

        if !quiet {
            println!(
                "  {} {}",
                "Shared remote:".bold(),
                format!("{norm}  (branch: {branch})").bright_blue()
            );
            for m in members {
                let short = if m.commit.len() >= 7 {
                    &m.commit[..7]
                } else {
                    &m.commit
                };
                let is_behind = behind.iter().any(|b| b.subdir == m.subdir);
                let tag = if is_behind {
                    format!("{}", " [behind]".red())
                } else if leaders.len() < members.len() {
                    format!("{}", " [ahead]".green())
                } else {
                    format!("{}", " [diverged]".yellow())
                };
                println!(
                    "    {:<40}  @ {}{}",
                    m.subdir.bright_yellow(),
                    short.yellow(),
                    tag
                );
            }
            println!();
            let suggested_names: Vec<&str> = suggested.iter().map(|m| m.subdir.as_str()).collect();
            println!(
                "  {}  {}",
                "Suggested pull:".dimmed(),
                suggested_names.join(", ").bright_cyan()
            );
        }

        // Interactive prompt (or auto with no_edit / quiet)
        let subdirs_to_pull: Vec<String> = if no_edit {
            suggested.iter().map(|m| m.subdir.clone()).collect()
        } else {
            let mut member_names: Vec<String> = members.iter().map(|m| m.subdir.clone()).collect();
            member_names.push("Pull all above".to_string());
            member_names.push("Skip this group".to_string());

            // Pre-select the suggested ones
            let defaults: Vec<bool> = members
                .iter()
                .map(|m| suggested.iter().any(|s| s.subdir == m.subdir))
                .chain(std::iter::once(false)) // "pull all"
                .chain(std::iter::once(false)) // "skip"
                .collect();

            println!();
            let prompt = format!("Which subrepos to pull for {}?", norm.bright_blue());

            use dialoguer::MultiSelect;
            let selection = MultiSelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt(&prompt)
                .items(&member_names)
                .defaults(&defaults)
                .interact_opt()?;

            match selection {
                None => Vec::new(), // Ctrl-C → skip
                Some(indices) => {
                    if indices.contains(&(member_names.len() - 1)) {
                        // "Skip" selected
                        Vec::new()
                    } else if indices.contains(&(member_names.len() - 2)) {
                        // "Pull all" selected
                        members.iter().map(|m| m.subdir.clone()).collect()
                    } else {
                        indices
                            .iter()
                            .filter(|&&i| i < members.len())
                            .map(|&i| members[i].subdir.clone())
                            .collect()
                    }
                }
            }
        };

        to_pull.extend(subdirs_to_pull);
        println!();
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
