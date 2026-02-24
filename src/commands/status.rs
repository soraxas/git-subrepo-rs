use crate::commands::{Context, normalize_subdir};
use crate::encode::encode_subdir;
use crate::git_utils::{branch_exists, rev_parse_short, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;
use colored::Colorize;

pub fn run(
    subdir_opt: Option<String>,
    quiet: bool,
    verbose: bool,
    _fetch: bool,
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

    if !subdirs.is_empty() && !quiet {
        let count = subdirs.len();
        let s = if count == 1 { "" } else { "s" };
        if !has_subdir {
            println!("{}", format!("{count} subrepo{s}:").bold());
            println!();
        }
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

        let refs_subrepo_fetch = format!("refs/subrepo/{subref}/fetch");
        let upstream_short = rev_parse_short(&refs_subrepo_fetch, &ctx.repo_root);

        println!(
            "{} '{}':",
            "Git subrepo".bold().bright_cyan(),
            subdir.bold().bright_yellow()
        );
        if branch_exists(&format!("subrepo/{subref}"), &ctx.repo_root) {
            println!("  {}  subrepo/{subref}", "Subrepo Branch:".bold().green());
        }
        println!(
            "  {}  {}",
            "Remote URL:     ".bold().green(),
            cfg.remote.bright_blue()
        );
        if let Some(ref us) = upstream_short {
            println!("  {}  {}", "Upstream Ref:   ".bold().green(), us.yellow());
        }
        println!(
            "  {}  {}",
            "Tracking Branch:".bold().green(),
            cfg.branch.cyan()
        );
        if !cfg.commit.is_empty()
            && let Some(short) = rev_parse_short(&cfg.commit, &ctx.repo_root)
        {
            println!(
                "  {}  {}",
                "Pulled Commit:  ".bold().green(),
                short.yellow()
            );
        }
        if !cfg.parent.is_empty()
            && let Some(short) = rev_parse_short(&cfg.parent, &ctx.repo_root)
        {
            println!(
                "  {}  {}",
                "Pull Parent:    ".bold().green(),
                short.dimmed()
            );
        }

        if dirty || verbose {
            print_dirty_status(&ctx, subdir, &cfg.parent);
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

    // Filter out subrepos that are nested within other subrepos (unless all_all)
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

/// Count commits in the main repo touching `subdir/` since the last pull (parent commit).
/// Prints a colored line showing how many unpushed commits exist.
fn print_dirty_status(ctx: &Context, subdir: &str, parent: &str) {
    // Commits reachable from HEAD that touch subdir/ but are not reachable from `parent`
    // (i.e. were made locally since the last pull/clone).
    let range = if parent.is_empty() {
        "HEAD".to_string()
    } else {
        format!("{parent}..HEAD")
    };
    let subdir_path = format!("{subdir}/");
    let (ok, out) = try_run_git(
        &["log", "--oneline", &range, "--", &subdir_path],
        &ctx.repo_root,
    );
    if !ok {
        return;
    }
    let commits: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    let n = commits.len();
    if n == 0 {
        println!(
            "  {}  {}",
            "Unpushed:       ".bold().green(),
            "none".dimmed()
        );
    } else {
        let label = if n == 1 {
            format!("{n} unpushed commit").yellow().bold().to_string()
        } else {
            format!("{n} unpushed commits").yellow().bold().to_string()
        };
        println!(
            "  {}  {} — run: {}",
            "Unpushed:       ".bold().green(),
            label,
            format!("git subrepo push {subdir}").bright_cyan()
        );
        for line in commits.iter().take(5) {
            println!("    {}", line.dimmed());
        }
        if n > 5 {
            println!("    {} …and {} more", "".dimmed(), (n - 5).to_string().dimmed());
        }
    }
}

fn print_status_refs(ctx: &Context, subref: &str) {
    let (ok, out) = try_run_git(&["show-ref"], &ctx.repo_root);
    if !ok {
        return;
    }

    let prefix = format!("refs/subrepo/{subref}/");
    let mut output = String::new();

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

        match ref_type {
            "branch" => {
                output += &format!(
                    "    {}  {short_sha} ({ref_name})\n",
                    "Branch Ref:   ".bold().green()
                )
            }
            "commit" => {
                output += &format!(
                    "    {}  {short_sha} ({ref_name})\n",
                    "Commit Ref:   ".bold().green()
                )
            }
            "fetch" => {
                output += &format!(
                    "    {}  {short_sha} ({ref_name})\n",
                    "Fetch Ref:    ".bold().green()
                )
            }
            "pull" => {
                output += &format!(
                    "    {}  {short_sha} ({ref_name})\n",
                    "Pull Ref:     ".bold().green()
                )
            }
            "push" => {
                output += &format!(
                    "    {}  {short_sha} ({ref_name})\n",
                    "Push Ref:     ".bold().green()
                )
            }
            _ => {}
        }
    }

    if !output.is_empty() {
        print!("  Refs:\n{output}");
    }
}
