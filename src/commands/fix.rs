use anyhow::Result;
use colored::Colorize;
use dialoguer::MultiSelect;

use crate::commands::Context;
use crate::git_utils::try_run_git;
use crate::gitrepo::read_gitrepo;

#[derive(Debug)]
enum Issue {
    /// parent = in .gitrepo is not an ancestor of HEAD (caused by rebase).
    ParentNotAncestor {
        stored: String,
        candidate: Option<String>,
    },
    /// A required field is missing from .gitrepo.
    MissingField { field: String },
    /// A named git ref (e.g. refs/subrepo/X/fetch) points to a missing object.
    StaleRef { ref_name: String },
    /// The `commit =` SHA in .gitrepo is not reachable locally.
    CommitUnknown { commit: String },
}

impl Issue {
    fn label(&self) -> String {
        match self {
            Issue::ParentNotAncestor { stored, candidate } => {
                let fix_hint = if let Some(c) = candidate {
                    format!("  → auto-fixable: use {}", &c[..c.len().min(7)])
                } else {
                    "  → fix: run `git subrepo pull --force <subdir>`".to_string()
                };
                format!(
                    "⚠ parent {} not in HEAD history (caused by rebase){}",
                    &stored[..stored.len().min(7)],
                    fix_hint.dimmed()
                )
            }
            Issue::MissingField { field } => {
                format!("✗ .gitrepo is missing required field: {}", field.bold())
            }
            Issue::StaleRef { ref_name } => {
                format!(
                    "⚠ stale git ref: {} points to missing object",
                    ref_name.dimmed()
                )
            }
            Issue::CommitUnknown { commit } => {
                format!(
                    "⚠ pinned commit {} is not known locally (fetch needed)",
                    &commit[..commit.len().min(7)]
                )
            }
        }
    }

    fn is_auto_fixable(&self) -> bool {
        matches!(
            self,
            Issue::ParentNotAncestor {
                candidate: Some(_),
                ..
            } | Issue::StaleRef { .. }
        )
    }
}

struct SubrepoReport {
    subdir: String,
    issues: Vec<Issue>,
}

fn scan_subrepo(ctx: &Context, subdir: &str) -> Vec<Issue> {
    let mut issues = Vec::new();

    let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");
    let Ok(cfg) = read_gitrepo(&gitrepo_path, &ctx.repo_root) else {
        issues.push(Issue::MissingField {
            field: "(could not read .gitrepo)".into(),
        });
        return issues;
    };

    // --- Required fields present? ---
    for (field, val) in [
        ("remote", &cfg.remote),
        ("branch", &cfg.branch),
        ("commit", &cfg.commit),
        ("parent", &cfg.parent),
    ] {
        if val.trim().is_empty() {
            issues.push(Issue::MissingField {
                field: field.to_string(),
            });
        }
    }

    if cfg.parent.trim().is_empty() || cfg.remote.trim().is_empty() {
        // Can't do further checks without parent/remote
        return issues;
    }

    // --- parent in HEAD history? ---
    // Nested subrepos (fix already skips them) shouldn't reach here, but use
    // parent_check_ref for correctness anyway.
    let parent = cfg.parent.trim();
    let check_ref = super::parent_check_ref(ctx, subdir);
    let (is_ancestor, _) = try_run_git(
        &["merge-base", "--is-ancestor", parent, &check_ref],
        &ctx.repo_root,
    );
    if !is_ancestor {
        let candidate = super::find_new_parent_after_rebase(ctx, subdir, parent, &check_ref);
        issues.push(Issue::ParentNotAncestor {
            stored: parent.to_string(),
            candidate,
        });
    }

    // --- pinned commit reachable locally? ---
    let commit = cfg.commit.trim();
    if !commit.is_empty() {
        let (ok, _) = try_run_git(&["rev-parse", "--verify", commit], &ctx.repo_root);
        if !ok {
            issues.push(Issue::CommitUnknown {
                commit: commit.to_string(),
            });
        }
    }

    // --- stale subrepo refs ---
    for suffix in &["fetch", "branch", "commit", "push"] {
        let ref_name = format!("refs/subrepo/{subdir}/{suffix}");
        let (ref_exists, sha) = try_run_git(&["rev-parse", "--verify", &ref_name], &ctx.repo_root);
        if ref_exists {
            // ref exists — check that the object it points to is valid
            let (obj_ok, _) = try_run_git(&["cat-file", "-e", sha.trim()], &ctx.repo_root);
            if !obj_ok {
                issues.push(Issue::StaleRef { ref_name });
            }
        }
    }

    issues
}

fn apply_fix(ctx: &Context, subdir: &str, issue: &Issue) -> Result<()> {
    match issue {
        Issue::ParentNotAncestor {
            candidate: Some(c), ..
        } => {
            let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");
            crate::git_utils::run_git(
                &[
                    "config",
                    "--file",
                    &gitrepo_path.to_string_lossy(),
                    "subrepo.parent",
                    c,
                ],
                &ctx.repo_root,
            )?;
            eprintln!(
                "  {} Set parent = {} in {subdir}/.gitrepo",
                "✓".green().bold(),
                &c[..c.len().min(7)].green()
            );
        }
        Issue::StaleRef { ref_name } => {
            crate::git_utils::run_git(&["update-ref", "-d", ref_name], &ctx.repo_root)?;
            eprintln!(
                "  {} Deleted stale ref {}",
                "✓".green().bold(),
                ref_name.dimmed()
            );
        }
        _ => {
            eprintln!("  {} No auto-fix available for this issue.", "·".dimmed());
        }
    }
    Ok(())
}

pub fn run(ctx: &Context) -> Result<()> {
    // Collect all subrepo subdirs
    let (ok, ls_out) = try_run_git(&["ls-files", "--", "*.gitrepo"], &ctx.repo_root);
    if !ok {
        anyhow::bail!("Failed to list subrepos");
    }

    let all_subdirs: Vec<String> = ls_out
        .lines()
        .filter(|l| l.ends_with("/.gitrepo"))
        .map(|l| l.trim_end_matches("/.gitrepo").to_string())
        .collect();

    // Drop nested subrepos — those whose path is prefixed by another subrepo path.
    // Their `.gitrepo` parent= points at commits in the *upstream* repo, not this one.
    let subdirs: Vec<String> = all_subdirs
        .iter()
        .filter(|subdir| {
            !all_subdirs
                .iter()
                .any(|other| other != *subdir && subdir.starts_with(&format!("{other}/")))
        })
        .cloned()
        .collect();

    if subdirs.is_empty() {
        eprintln!("{}: No subrepos found.", "git-subrepo".yellow().bold());
        return Ok(());
    }

    // Scan all subrepos
    let reports: Vec<SubrepoReport> = subdirs
        .iter()
        .map(|subdir| SubrepoReport {
            subdir: subdir.clone(),
            issues: scan_subrepo(ctx, subdir),
        })
        .collect();

    let total_issues: usize = reports.iter().map(|r| r.issues.len()).sum();

    if total_issues == 0 {
        eprintln!(
            "{}: {} — no issues found.",
            "git-subrepo".green().bold(),
            format!("Scanned {} subrepo(s)", subdirs.len()).green()
        );
        return Ok(());
    }

    // Display all issues
    eprintln!(
        "{}: Scanned {} subrepo(s) — {} issue(s) found\n",
        "git-subrepo".yellow().bold(),
        subdirs.len(),
        total_issues.to_string().yellow().bold()
    );

    let mut fixable_items: Vec<(String, usize, usize)> = Vec::new(); // (subdir, report_idx, issue_idx)

    for (ri, report) in reports.iter().enumerate() {
        if report.issues.is_empty() {
            eprintln!("  {} {}", "✓".green(), report.subdir.bold());
            continue;
        }
        eprintln!("  {} {}:", "✗".red().bold(), report.subdir.bold());
        for (ii, issue) in report.issues.iter().enumerate() {
            let label = issue.label();
            if issue.is_auto_fixable() {
                eprintln!("    {}", label.yellow());
                fixable_items.push((report.subdir.clone(), ri, ii));
            } else {
                eprintln!("    {}", label.red());
            }
        }
    }
    eprintln!();

    if fixable_items.is_empty() {
        eprintln!(
            "{}",
            "No issues are auto-fixable. Manual intervention required.".dimmed()
        );
        return Ok(());
    }

    // Build multi-select prompt for fixable issues
    let labels: Vec<String> = fixable_items
        .iter()
        .map(|(subdir, ri, ii)| {
            let issue = &reports[*ri].issues[*ii];
            format!("{subdir}: {}", issue.label())
        })
        .collect();

    let defaults: Vec<bool> = vec![true; labels.len()];

    let selections = MultiSelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Select issues to fix (space to toggle, enter to apply)")
        .items(&labels)
        .defaults(&defaults)
        .interact_opt();

    let selected_indices = match selections {
        Ok(Some(v)) => v,
        Ok(None) | Err(_) => {
            eprintln!("{}", "Aborted.".dimmed());
            return Ok(());
        }
    };

    if selected_indices.is_empty() {
        eprintln!("{}", "Nothing selected.".dimmed());
        return Ok(());
    }

    eprintln!();
    let mut fixed = 0;
    for idx in selected_indices {
        let (subdir, ri, ii) = &fixable_items[idx];
        let issue = &reports[*ri].issues[*ii];
        eprintln!("  Fixing {subdir}: {}", issue.label());
        match apply_fix(ctx, subdir, issue) {
            Ok(_) => fixed += 1,
            Err(e) => eprintln!("    {} {}", "✗".red().bold(), e),
        }
    }

    if fixed > 0 {
        eprintln!(
            "\n{}: Fixed {} issue(s). Run `git subrepo pull <subdir>` to continue.",
            "git-subrepo".green().bold(),
            fixed.to_string().green().bold()
        );
    }

    Ok(())
}
