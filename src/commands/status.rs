use crate::commands::{Context, normalize_subdir, subrepo_fetch_with_pb};
use crate::encode::encode_subdir;
use crate::git_utils::{branch_exists, rev_parse_short, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::time::Duration;

/// Label column width (chars). Value starts at col 2 + LABEL_W + 2 = 22.
const LABEL_W: usize = 18;
/// Indent for continuation lines — matches value column (2 + 18 + 2 = 22 spaces).
const CONT_INDENT: &str = "                      ";

/// Print a single aligned `  Label:  Value` line.
macro_rules! field {
    ($label:expr, $value:expr) => {
        println!(
            "  {:<width$}  {}",
            $label.bold().green().to_string(),
            $value,
            width = LABEL_W
        )
    };
}

pub fn run(
    subdir_opt: Option<String>,
    quiet: bool,
    verbose: bool,
    fetch: bool,
    all: bool,
    all_all: bool,
    dirty: bool,
) -> Result<()> {
    let ctx = Context::new()?;

    if ctx.original_head_branch.is_empty() {
        anyhow::bail!("Must be on a branch to run this command.");
    }

    let has_subdir = subdir_opt.is_some();
    let subdirs = if let Some(s) = subdir_opt {
        vec![normalize_subdir(&s)]
    } else {
        get_all_subrepos(&ctx, all || all_all, all_all)?
    };

    if subdirs.is_empty() && !quiet {
        println!("{}", "No subrepos.".yellow());
        return Ok(());
    }

    // Fetch upstream refs in parallel so status is always fresh.
    if fetch && !quiet {
        // Collect (subdir, remote, branch, subref) for all fetchable subrepos.
        let tasks: Vec<(String, String, String, String)> = subdirs
            .iter()
            .filter_map(|subdir| {
                let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");
                if let Ok(cfg) = read_gitrepo(&gitrepo_path, &ctx.repo_root)
                    && !cfg.remote.is_empty()
                    && cfg.remote != "none"
                {
                    let subref = encode_subdir(subdir);
                    Some((
                        subdir.clone(),
                        cfg.remote.clone(),
                        cfg.branch.clone(),
                        subref,
                    ))
                } else {
                    None
                }
            })
            .collect();

        if !tasks.is_empty() {
            let mp = MultiProgress::new();
            let spinner_style = ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap();

            // Create one spinner per task up front so they all appear at once.
            let bars: Vec<ProgressBar> = tasks
                .iter()
                .map(|(subdir, _, _, _)| {
                    let pb = mp.add(ProgressBar::new_spinner());
                    pb.set_style(spinner_style.clone());
                    pb.set_message(format!("Fetching '{subdir}'...").bright_cyan().to_string());
                    pb.enable_steady_tick(Duration::from_millis(80));
                    pb
                })
                .collect();

            // Fetch all subrepos in parallel using scoped threads.
            std::thread::scope(|s| {
                for ((_, remote, branch, subref), pb) in tasks.iter().zip(bars.iter()) {
                    s.spawn(|| {
                        // Pass our MultiProgress-owned bar so subrepo_fetch doesn't create its own
                        let _ = subrepo_fetch_with_pb(&ctx, remote, branch, subref, Some(pb));
                        pb.finish_and_clear();
                    });
                }
            });

            let _ = mp.clear();
        }
    }

    if !subdirs.is_empty() && !quiet && !has_subdir {
        let count = subdirs.len();
        let s = if count == 1 { "" } else { "s" };
        println!("{}", format!("{count} subrepo{s}:").bold());
        println!();
    }

    for subdir in &subdirs {
        let subref = encode_subdir(subdir);
        let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");

        if !gitrepo_path.exists() {
            println!("{}", format!("'{subdir}' is not a subrepo").red());
            println!();
            continue;
        }

        let cfg = match read_gitrepo(&gitrepo_path, &ctx.repo_root) {
            Ok(c) => c,
            Err(_) => {
                println!("{}", format!("'{subdir}' is not a subrepo").red());
                println!();
                continue;
            }
        };

        if quiet {
            println!("{subdir}");
            continue;
        }

        println!(
            "{} '{}':",
            "Git subrepo".bold().bright_cyan(),
            subdir.bold().bright_yellow()
        );

        if branch_exists(&format!("subrepo/{subref}"), &ctx.repo_root) {
            field!("Subrepo Branch:", format!("subrepo/{subref}").cyan());
        }

        field!("Remote URL:", cfg.remote.bright_blue());

        // Tracking Branch: master (eb439faf) [up to date]
        //               or master (eb439faf) [remote HEAD: 81a64ae7 (2 commits ahead)]
        {
            let refs_subrepo_fetch = format!("refs/subrepo/{subref}/fetch");
            let pulled_short = if cfg.commit.is_empty() {
                None
            } else {
                rev_parse_short(&cfg.commit, &ctx.repo_root)
            };
            let upstream_short = rev_parse_short(&refs_subrepo_fetch, &ctx.repo_root);

            let mut tracking = cfg.branch.cyan().to_string();
            if let Some(ref p) = pulled_short {
                tracking.push_str(&format!(" ({})", p.yellow()));
            }

            // Compare upstream SHA to pulled commit SHA
            let fetch_sha = {
                let (ok, out) = try_run_git(&["rev-parse", &refs_subrepo_fetch], &ctx.repo_root);
                if ok {
                    out.trim().to_string()
                } else {
                    String::new()
                }
            };

            let up_to_date = !fetch_sha.is_empty()
                && !cfg.commit.is_empty()
                && (fetch_sha.starts_with(&cfg.commit)
                    || cfg.commit.starts_with(fetch_sha.as_str()));

            if up_to_date {
                tracking.push_str(&format!(" [{}]", "up to date".green()));
            } else if let Some(ref u) = upstream_short {
                // Count how many commits ahead the remote is
                let ahead_str = if !cfg.commit.is_empty() {
                    let (ok, cnt) = try_run_git(
                        &[
                            "rev-list",
                            "--count",
                            &format!("{}..{}", cfg.commit, refs_subrepo_fetch),
                        ],
                        &ctx.repo_root,
                    );
                    if ok {
                        let n: usize = cnt.trim().parse().unwrap_or(0);
                        if n > 0 {
                            format!(" ({} commit{} ahead)", n, if n == 1 { "" } else { "s" })
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                tracking.push_str(&format!(
                    " [remote HEAD: {}{}]",
                    u.bright_magenta(),
                    ahead_str.bright_magenta()
                ));
            }
            field!("Tracking Branch:", tracking);
        }

        // Pull Parent: 8cb9e7ae (N commits ago)  or warning if not in history
        if !cfg.parent.is_empty() {
            let parent_short = rev_parse_short(&cfg.parent, &ctx.repo_root)
                .unwrap_or_else(|| cfg.parent[..cfg.parent.len().min(7)].to_string());

            // How many commits ago is the parent?
            let check_ref = crate::commands::parent_check_ref(&ctx, subdir);
            let (ok, count_out) = try_run_git(
                &[
                    "rev-list",
                    "--count",
                    &format!("{}..{}", cfg.parent, check_ref),
                ],
                &ctx.repo_root,
            );
            let ancestor_count: Option<usize> = if ok {
                count_out.trim().parse().ok()
            } else {
                None
            };

            let (is_ancestor, _) = try_run_git(
                &["merge-base", "--is-ancestor", &cfg.parent, &check_ref],
                &ctx.repo_root,
            );

            let parent_display = if is_ancestor {
                let ago = match ancestor_count {
                    Some(0) => "at HEAD".dimmed().to_string(),
                    Some(n) => format!("{} commit{} ago", n, if n == 1 { "" } else { "s" })
                        .dimmed()
                        .to_string(),
                    None => String::new(),
                };
                format!("{} ({})", parent_short.dimmed(), ago)
            } else {
                format!(
                    "{} ({}) {}",
                    parent_short.yellow(),
                    ancestor_count
                        .map(|n| format!("{n} commits ago"))
                        .unwrap_or_else(|| "?".to_string())
                        .yellow(),
                    "⚠  not in history — run `git subrepo fix`".yellow().bold()
                )
            };
            field!("Pull Parent:", parent_display);
        }

        if dirty || verbose {
            print_dirty_status(&ctx, subdir, &cfg.parent);
            print_upstream_status(&ctx, &subref, subdir, &cfg.commit);
        }

        if verbose {
            print_status_refs(&ctx, subref.as_str());
        }

        println!();
    }

    Ok(())
}

fn get_all_subrepos(ctx: &Context, _all: bool, all_all: bool) -> Result<Vec<String>> {
    let (ok, out) = try_run_git(&["ls-files"], &ctx.repo_root);
    if !ok {
        return Ok(vec![]);
    }

    let mut paths: Vec<String> = out
        .lines()
        .filter_map(|line| {
            if line.ends_with("/.gitrepo") {
                Some(line.trim_end_matches("/.gitrepo").to_string())
            } else {
                None
            }
        })
        .collect();

    paths.sort();

    if all_all {
        return Ok(paths);
    }
    let mut result: Vec<String> = Vec::new();
    'outer: for path in &paths {
        for existing in &result {
            if path.starts_with(&format!("{existing}/")) {
                continue 'outer;
            }
        }
        result.push(path.clone());
    }

    Ok(result)
}

/// Print how many local commits touch `subdir/` since the last pull,
/// ignoring subrepo maintenance commits.
fn print_dirty_status(ctx: &Context, subdir: &str, parent: &str) {
    let range = if parent.is_empty() {
        "HEAD".to_string()
    } else {
        format!("{parent}..HEAD")
    };
    let subdir_path = format!("{subdir}/");
    let gitrepo_exclude = format!(":(exclude){subdir}/.gitrepo");
    let (ok, out) = try_run_git(
        &[
            "log",
            "--oneline",
            "--invert-grep",
            "--grep=^git subrepo ",
            &range,
            "--",
            &subdir_path,
            &gitrepo_exclude,
        ],
        &ctx.repo_root,
    );
    if !ok {
        return;
    }
    let commits: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    let n = commits.len();
    if n == 0 {
        field!("Unpushed:", "up to date".dimmed());
    } else {
        let s = if n == 1 { "commit" } else { "commits" };
        let summary = format!("{n} unpushed {s}").yellow().bold().to_string();
        let hint = format!("git subrepo push {subdir}")
            .bright_cyan()
            .to_string();
        field!("Unpushed:", format!("{summary}  →  {hint}"));
        for line in commits.iter().take(5) {
            println!("{CONT_INDENT}{}", line.dimmed());
        }
        if n > 5 {
            println!("{CONT_INDENT}{}", format!("…and {} more", n - 5).dimmed());
        }
    }
}

/// Show upstream commits available to pull (based on last fetch).
fn print_upstream_status(ctx: &Context, subref: &str, subdir: &str, pulled_commit: &str) {
    let fetch_ref = format!("refs/subrepo/{subref}/fetch");

    let (ok, fetch_sha) = try_run_git(&["rev-parse", &fetch_ref], &ctx.repo_root);
    if !ok || fetch_sha.trim().is_empty() {
        field!(
            "Upstream:",
            format!("not fetched  →  {}", "git subrepo fetch".bright_cyan())
        );
        return;
    }
    let fetch_sha = fetch_sha.trim();

    if pulled_commit.is_empty()
        || fetch_sha.starts_with(pulled_commit)
        || pulled_commit.starts_with(fetch_sha)
    {
        field!("Upstream:", "up to date".dimmed());
        return;
    }

    let range = format!("{pulled_commit}..{fetch_ref}");
    let (ok2, out) = try_run_git(&["log", "--oneline", &range], &ctx.repo_root);
    if !ok2 {
        field!(
            "Upstream:",
            format!("diverged  →  {}", "git subrepo pull".bright_cyan())
        );
        return;
    }
    let ahead: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    let n = ahead.len();
    if n == 0 {
        field!("Upstream:", "up to date".dimmed());
    } else {
        let s = if n == 1 { "commit" } else { "commits" };
        let summary = format!("{n} new upstream {s}")
            .bright_magenta()
            .bold()
            .to_string();
        let hint = format!("git subrepo pull {subdir}")
            .bright_cyan()
            .to_string();
        field!("Upstream:", format!("{summary}  →  {hint}"));
        for line in ahead.iter().take(5) {
            println!("{CONT_INDENT}{}", line.dimmed());
        }
        if n > 5 {
            println!("{CONT_INDENT}{}", format!("…and {} more", n - 5).dimmed());
        }
    }
}

fn print_status_refs(ctx: &Context, subref: &str) {
    let (ok, out) = try_run_git(&["show-ref"], &ctx.repo_root);
    if !ok {
        return;
    }

    let prefix = format!("refs/subrepo/{subref}/");
    let mut lines: Vec<String> = Vec::new();

    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() != 2 {
            continue;
        }
        let sha = parts[0];
        let ref_name = parts[1];
        if !ref_name.starts_with(&prefix) {
            continue;
        }
        let ref_type = &ref_name[prefix.len()..];
        let short_sha = crate::git_utils::rev_parse_short(sha, &ctx.repo_root)
            .unwrap_or_else(|| sha[..7.min(sha.len())].to_string());
        let label = match ref_type {
            "branch" => "Branch Ref:",
            "commit" => "Commit Ref:",
            "fetch" => "Fetch Ref:",
            "pull" => "Pull Ref:",
            "push" => "Push Ref:",
            _ => continue,
        };
        lines.push(format!(
            "  {:<width$}  {} {}",
            label.bold().green().to_string(),
            short_sha.yellow(),
            format!("({ref_name})").dimmed(),
            width = LABEL_W
        ));
    }

    if !lines.is_empty() {
        println!("  {}", "─".repeat(40).dimmed());
        for l in lines {
            println!("{l}");
        }
    }
}
