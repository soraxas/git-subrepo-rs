mod cli;
mod commands;
mod encode;
mod error;
mod git_utils;
mod gitrepo;

use clap::Parser;
use cli::{Cli, Commands};
use colored::Colorize;

fn get_all_subrepos(all_all: bool) -> anyhow::Result<Vec<String>> {
    let cwd = std::env::current_dir()?;
    let repo_root_str = git_utils::run_git(&["rev-parse", "--show-toplevel"], &cwd)?;
    let repo_root = std::path::PathBuf::from(repo_root_str.trim());

    let files = git_utils::run_git(&["ls-files"], &repo_root)?;
    let mut subrepos: Vec<String> = files
        .lines()
        .filter(|f| f.ends_with("/.gitrepo"))
        .map(|f| f.trim_end_matches("/.gitrepo").to_string())
        .collect();
    subrepos.sort();

    if !all_all {
        // Filter out sub-subrepos: skip if path starts with another subrepo's path + /
        let all = subrepos.clone();
        subrepos.retain(|s| {
            !all.iter()
                .any(|other| other != s && s.starts_with(&format!("{other}/")))
        });
    }

    Ok(subrepos)
}

fn print_no_command_help() {
    println!(
        "{}\n",
        "git-subrepo - Manage git sub-repositories"
            .bright_cyan()
            .bold()
    );
    println!("Usage: {} <command> [options]\n", "git subrepo".bold());
    println!("{}:", "Commands".bold());
    let cmds = [
        (
            "clone",
            "<remote> [<subdir>]",
            "Clone a remote repository into a subdir",
        ),
        ("init", "<subdir>", "Turn a subdir into a subrepo"),
        ("pull", "[<subdir>]", "Pull upstream changes"),
        ("push", "[<subdir>]", "Push local commits upstream"),
        ("fetch", "[<subdir>]", "Fetch upstream (creates ref)"),
        (
            "branch",
            "[<subdir>]",
            "Create a branch of local subrepo commits",
        ),
        ("commit", "<subdir>", "Commit a merged branch into mainline"),
        ("status", "[<subdir>]", "Show subrepo status"),
        ("clean", "[<subdir>]", "Remove subrepo branches/refs"),
        ("config", "<subdir> <key>", "Get/set subrepo config"),
    ];
    for (name, args, desc) in &cmds {
        println!("  {:<8} {:<26}  {}", name.green().bold(), args, desc);
    }
    println!("\n{}:", "Global options".bold());
    let opts = [
        ("-a, --all", "Operate on all subrepos"),
        ("-f, --force", "Force operation"),
        ("-F, --fetch", "Fetch upstream first"),
        ("-q, --quiet", "Suppress output"),
        ("-v, --verbose", "Verbose output"),
    ];
    for (flag, desc) in &opts {
        println!("  {:<20}  {}", flag.yellow().bold(), desc);
    }
}

/// Group subrepo paths by path depth (number of '/' separators).
fn group_by_depth(subrepos: Vec<String>) -> Vec<Vec<String>> {
    let mut levels: Vec<Vec<String>> = Vec::new();
    for subdir in subrepos {
        let depth = subdir.chars().filter(|&c| c == '/').count();
        while levels.len() <= depth {
            levels.push(Vec::new());
        }
        levels[depth].push(subdir);
    }
    levels
}

/// Run a blocking operation on each subrepo in parallel within each depth level.
/// Each task gets its own spinner in a shared `MultiProgress`; completion messages
/// are printed via `mp.println()` so they never interleave with spinner output.
async fn run_all_parallel(
    levels: Vec<Vec<String>>,
    total: usize,
    quiet: bool,
    verb_start: &'static str,
    f: impl Fn(String, indicatif::ProgressBar) -> anyhow::Result<String> + Send + Sync + Clone + 'static,
) -> anyhow::Result<()> {
    use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
    use std::time::Duration;

    let mp = if quiet {
        None
    } else {
        Some(MultiProgress::new())
    };

    let mut idx = 0usize;
    for level in levels {
        let mut set: tokio::task::JoinSet<anyhow::Result<()>> = tokio::task::JoinSet::new();
        for s in level {
            idx += 1;
            let cur_idx = idx;

            // Create a spinner for this subrepo (or a hidden no-op bar in quiet mode).
            let pb = if let Some(ref m) = mp {
                let bar = m.add(ProgressBar::new_spinner());
                bar.set_style(
                    ProgressStyle::default_spinner()
                        .template("{spinner:.cyan} {msg}")
                        .unwrap(),
                );
                bar.set_message(
                    format!("[{cur_idx}/{total}] {verb_start} '{s}'...")
                        .bright_cyan()
                        .to_string(),
                );
                bar.enable_steady_tick(Duration::from_millis(80));
                bar
            } else {
                ProgressBar::hidden()
            };

            let f_clone = f.clone();
            let mp_clone = mp.clone();
            let s_clone = s.clone();

            set.spawn(async move {
                let pb_clone = pb.clone();
                let result = tokio::task::spawn_blocking(move || f_clone(s_clone, pb_clone))
                    .await
                    .unwrap_or_else(|e| Err(anyhow::anyhow!("task panicked: {e}")));

                pb.finish_and_clear();

                match result {
                    Ok(msg) => {
                        if let Some(ref m) = mp_clone {
                            let _ = m.println(&msg);
                        }
                        Ok(())
                    }
                    Err(e) => {
                        let err_line = format!("{}: {e}", "git-subrepo".red().bold());
                        if let Some(ref m) = mp_clone {
                            let _ = m.println(err_line);
                        } else {
                            eprintln!("{}: {e}", "git-subrepo".red().bold());
                        }
                        Ok(()) // don't abort the whole set on one error
                    }
                }
            });
        }
        while let Some(res) = set.join_next().await {
            res??;
        }
    }
    if let Some(m) = mp {
        let _ = m.clear();
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    // Use try_parse to format errors ourselves
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            let msg = match e.kind() {
                ErrorKind::InvalidSubcommand => {
                    let rendered = e.render().to_string();
                    // Extract the single-quoted subcommand name from the rendered error
                    // e.g. "error: unrecognized subcommand 'main'"
                    if let Some(start) = rendered.find('\'') {
                        let after = &rendered[start + 1..];
                        if let Some(end) = after.find('\'') {
                            let subcmd = &after[..end];
                            format!("'{}' is not a command. See 'git subrepo help'.", subcmd)
                        } else {
                            rendered
                                .lines()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .trim_start_matches("error: ")
                                .to_string()
                        }
                    } else {
                        rendered
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .trim_start_matches("error: ")
                            .to_string()
                    }
                }
                ErrorKind::UnknownArgument => {
                    let rendered = e.render().to_string();
                    // Transform "unexpected argument '--foo' found" → "error: unknown option `foo'"
                    if let Some(start) = rendered.find("'--") {
                        let after = &rendered[start + 3..];
                        if let Some(end) = after.find('\'') {
                            let option_name = &after[..end];
                            format!("error: unknown option `{}'", option_name)
                        } else {
                            rendered
                                .lines()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .trim_start_matches("error: ")
                                .to_string()
                        }
                    } else {
                        rendered
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .trim_start_matches("error: ")
                            .to_string()
                    }
                }
                ErrorKind::MissingRequiredArgument => {
                    let rendered = e.render().to_string();
                    if rendered.contains("<SUBDIR>") {
                        // Find which subcommand was invoked via env args
                        let known_cmds = [
                            "clone", "init", "pull", "push", "fetch", "branch", "commit", "status",
                            "clean", "config",
                        ];
                        let cmd = std::env::args()
                            .find(|a| known_cmds.contains(&a.as_str()))
                            .unwrap_or_default();
                        format!("Command '{}' requires arg 'subdir'.", cmd)
                    } else if rendered.contains("<REMOTE>")
                        && std::env::args()
                            .any(|a| a == "--all" || a == "-a" || a == "--ALL" || a == "-A")
                    {
                        // clone --all
                        "Invalid option '--all' for 'clone'.".to_string()
                    } else {
                        rendered
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .trim_start_matches("error: ")
                            .to_string()
                    }
                }
                _ => e.render().to_string(),
            };
            eprintln!("git-subrepo: {msg}");
            std::process::exit(1);
        }
    };

    if cli.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    let quiet = cli.quiet;
    let verbose = cli.verbose;
    let all = cli.all;
    let all_all = cli.all_all;
    let force = cli.force;
    let fetch = cli.fetch;
    let no_edit = cli.no_edit;

    let result: anyhow::Result<()> = async {
        match cli.command {
            None => {
                print_no_command_help();
                std::process::exit(0);
            }
            Some(Commands::Clone {
                remote,
                subdir,
                branch,
                method,
                quiet: q,
                message,
                extra,
            }) => {
                if all || all_all {
                    anyhow::bail!("Invalid option '--all' for 'clone'.");
                }
                if !extra.is_empty() {
                    anyhow::bail!(
                        "Unknown argument(s) '{}' for 'clone' command.",
                        extra.join(" ")
                    );
                }
                commands::clone::run(remote, subdir, branch, force, method, quiet || q, message)
            }
            Some(Commands::Init {
                subdir,
                remote,
                branch,
                method,
            }) => commands::init::run(subdir, remote, branch, method),
            Some(Commands::Pull {
                subdir,
                branch,
                remote,
                method,
                quiet: q,
                update,
                message,
            }) => {
                if !all && !all_all && update && branch.is_none() && remote.is_none() {
                    anyhow::bail!("Can't use '--update' without '--branch' or '--remote'.");
                }
                if all || all_all {
                    let subrepos = get_all_subrepos(all_all)?;
                    let total = subrepos.len();
                    let levels = group_by_depth(subrepos);
                    let mut idx = 0usize;

                    for level in levels {
                        // Phase 1: prepare all subrepos at this depth in parallel
                        // (fetch + branch creation + rebase – independent per subrepo)
                        let level_len = level.len();
                        let mut handles: Vec<(
                            usize,
                            String,
                            tokio::task::JoinHandle<
                                anyhow::Result<Option<commands::pull::PullPrepared>>,
                            >,
                        )> = Vec::new();

                        for s in level {
                            idx += 1;
                            let cur_idx = idx;
                            if !quiet && !q {
                                eprintln!(
                                    "{}",
                                    format!("[{cur_idx}/{total}] Preparing '{s}'...").bright_cyan()
                                );
                            }
                            let s_clone = s.clone();
                            let branch = branch.clone();
                            let remote = remote.clone();
                            let method = method.clone();
                            let message = message.clone();
                            let h = tokio::task::spawn_blocking(move || {
                                let ctx = commands::Context::new()?;
                                commands::pull::prepare(
                                    &ctx,
                                    s_clone,
                                    branch,
                                    remote,
                                    force,
                                    method,
                                    quiet || q,
                                    update,
                                    message,
                                    no_edit,
                                    None,
                                )
                            });
                            handles.push((cur_idx, s, h));
                        }

                        // Collect prepared results, separating successes from failures
                        let mut prepared_list = Vec::with_capacity(level_len);
                        let mut failed_list: Vec<(usize, String, anyhow::Error)> = Vec::new();
                        for (cur_idx, s, h) in handles {
                            match h.await? {
                                Ok(Some(p)) => prepared_list.push((cur_idx, s, p)),
                                Ok(None) => {} // up-to-date
                                Err(e) => failed_list.push((cur_idx, s, e)),
                            }
                        }

                        // If there are failures, show them and ask the user one-by-one
                        if !failed_list.is_empty() {
                            eprintln!(
                                "\n{} {} need{}attention:\n",
                                failed_list.len().to_string().yellow().bold(),
                                if failed_list.len() == 1 {
                                    "subrepo"
                                } else {
                                    "subrepos"
                                },
                                if failed_list.len() == 1 { " " } else { "s " },
                            );
                            for (cur_idx, s, e) in &failed_list {
                                eprintln!(
                                    "  {} {} — {}",
                                    format!("[{cur_idx}/{total}]").dimmed(),
                                    s.yellow().bold(),
                                    e
                                );
                            }
                            eprintln!();

                            for (cur_idx, s, e) in failed_list {
                                let prompt = format!(
                                    "[{cur_idx}/{total}] What to do with '{}'?",
                                    s.yellow().bold()
                                );
                                let choices = &[
                                    "Force pull (bypass upstream check)",
                                    "Fetch then force pull",
                                    "Skip",
                                    "Abort",
                                ];
                                let selection = dialoguer::Select::with_theme(
                                    &dialoguer::theme::ColorfulTheme::default(),
                                )
                                .with_prompt(&prompt)
                                .items(choices)
                                .default(0)
                                .interact_opt()?;

                                match selection {
                                    Some(0) => {
                                        // Force pull: retry prepare with force=true
                                        let ctx = commands::Context::new()?;
                                        match commands::pull::prepare(
                                            &ctx,
                                            s.clone(),
                                            branch.clone(),
                                            remote.clone(),
                                            true, // force
                                            method.clone(),
                                            quiet || q,
                                            update,
                                            message.clone(),
                                            no_edit,
                                            None,
                                        ) {
                                            Ok(Some(p)) => {
                                                prepared_list.push((cur_idx, s, p));
                                            }
                                            Ok(None) => {}
                                            Err(e2) => {
                                                eprintln!("{}: {}", "git-subrepo".red().bold(), e2);
                                            }
                                        }
                                    }
                                    Some(1) => {
                                        // Fetch then force pull
                                        let ctx = commands::Context::new()?;
                                        let gitrepo_path = ctx.repo_root.join(&s).join(".gitrepo");
                                        if let Ok(cfg) = crate::gitrepo::read_gitrepo(
                                            &gitrepo_path,
                                            &ctx.repo_root,
                                        ) {
                                            let subref = crate::encode::encode_subdir(&s);
                                            let _ = commands::subrepo_fetch(
                                                &ctx,
                                                &cfg.remote,
                                                &cfg.branch,
                                                &subref,
                                            );
                                        }
                                        match commands::pull::prepare(
                                            &ctx,
                                            s.clone(),
                                            branch.clone(),
                                            remote.clone(),
                                            true, // force
                                            method.clone(),
                                            quiet || q,
                                            update,
                                            message.clone(),
                                            no_edit,
                                            None,
                                        ) {
                                            Ok(Some(p)) => {
                                                prepared_list.push((cur_idx, s, p));
                                            }
                                            Ok(None) => {}
                                            Err(e2) => {
                                                eprintln!("{}: {}", "git-subrepo".red().bold(), e2);
                                            }
                                        }
                                    }
                                    Some(2) | None => {
                                        // Skip
                                        eprintln!("  {} Skipping '{s}'.", "→".dimmed());
                                        let _ = e; // suppress unused warning
                                    }
                                    _ => {
                                        // Abort
                                        anyhow::bail!("Aborted.");
                                    }
                                }
                            }
                        }

                        // Sort prepared_list back into original index order before committing
                        prepared_list.sort_by_key(|(i, _, _)| *i);

                        // Phase 2: commit sequentially (git commit is not concurrent-safe)
                        let ctx = commands::Context::new()?;
                        for (cur_idx, s, prepared) in prepared_list {
                            if !quiet && !q {
                                eprintln!(
                                    "{}",
                                    format!("[{cur_idx}/{total}] Pulling '{s}'...").bright_cyan()
                                );
                            }
                            commands::pull::commit_prepared(&ctx, prepared, quiet || q)?;
                        }
                    }
                    Ok(())
                } else {
                    let subdir = subdir
                        .ok_or_else(|| anyhow::anyhow!("Command 'pull' requires arg 'subdir'."))?;
                    if subdir.starts_with('/') {
                        anyhow::bail!("The subdir '{}' should not be absolute path.", subdir);
                    }
                    commands::pull::run(
                        subdir,
                        branch,
                        remote,
                        force,
                        method,
                        quiet || q,
                        update,
                        message,
                        no_edit,
                    )
                }
            }
            Some(Commands::Push {
                subdir,
                branch,
                remote,
                method,
                squash,
                quiet: q,
                update: _,
                message,
            }) => {
                if all || all_all {
                    let subrepos = get_all_subrepos(all_all)?;
                    let total = subrepos.len();
                    // Sequential: push commits to the repo; concurrent commits conflict.
                    let levels = group_by_depth(subrepos);
                    let mut idx = 0usize;
                    for level in levels {
                        for s in level {
                            idx += 1;
                            if !quiet && !q {
                                eprintln!(
                                    "{}",
                                    format!("[{idx}/{total}] Pushing '{s}'...").bright_cyan()
                                );
                            }
                            commands::push::run(
                                s,
                                branch.clone(),
                                remote.clone(),
                                force,
                                method.clone(),
                                squash,
                                quiet || q,
                                message.clone(),
                            )?;
                        }
                    }
                    Ok(())
                } else {
                    let subdir = subdir
                        .ok_or_else(|| anyhow::anyhow!("Command 'push' requires arg 'subdir'."))?;
                    commands::push::run(
                        subdir,
                        branch,
                        remote,
                        force,
                        method,
                        squash,
                        quiet || q,
                        message,
                    )
                }
            }
            Some(Commands::Fetch {
                subdir,
                branch,
                remote,
                quiet: q,
            }) => {
                if all || all_all {
                    let subrepos = get_all_subrepos(all_all)?;
                    let total = subrepos.len();
                    let levels = group_by_depth(subrepos);
                    let branch_cap = branch.clone();
                    let remote_cap = remote.clone();
                    let q_cap = q;
                    run_all_parallel(levels, total, quiet || q, "Fetching", move |s, pb| {
                        commands::fetch::run_with_pb(
                            s.clone(),
                            branch_cap.clone(),
                            remote_cap.clone(),
                            q_cap,
                            Some(pb),
                        )
                    })
                    .await?;
                    Ok(())
                } else {
                    let subdir = subdir
                        .ok_or_else(|| anyhow::anyhow!("Command 'fetch' requires arg 'subdir'."))?;
                    commands::fetch::run(subdir, branch, remote, quiet || q).map(|_| ())
                }
            }
            Some(Commands::Branch { subdir, quiet: q }) => {
                if all || all_all {
                    let subrepos = get_all_subrepos(all_all)?;
                    let total = subrepos.len();
                    // Sequential: branch modifies the repo; concurrent operations conflict.
                    let levels = group_by_depth(subrepos);
                    let mut idx = 0usize;
                    for level in levels {
                        for s in level {
                            idx += 1;
                            if !quiet && !q {
                                eprintln!(
                                    "{}",
                                    format!("[{idx}/{total}] Branching '{s}'...").bright_cyan()
                                );
                            }
                            // branch --all: skip subrepos with no new commits
                            let _ = commands::branch_cmd::run(s, force, fetch, quiet || q);
                        }
                    }
                    Ok(())
                } else {
                    let subdir = subdir.ok_or_else(|| {
                        anyhow::anyhow!("Command 'branch' requires arg 'subdir'.")
                    })?;
                    commands::branch_cmd::run(subdir, force, fetch, quiet || q)
                }
            }
            Some(Commands::Commit {
                subdir,
                subrepo_commit_ref,
                quiet: q,
                message,
            }) => commands::commit_cmd::run(
                subdir,
                subrepo_commit_ref,
                force,
                fetch,
                quiet || q,
                message,
            ),
            Some(Commands::Status {
                subdir,
                quiet: q,
                verbose: v,
                dirty,
            }) => {
                commands::status::run(subdir, quiet || q, verbose || v, fetch, all, all_all, dirty)
            }
            Some(Commands::Clean { subdir, quiet: q }) => {
                if (all || all_all) && subdir.is_none() {
                    let subrepos = get_all_subrepos(all_all)?;
                    let total = subrepos.len();
                    let levels = group_by_depth(subrepos);
                    run_all_parallel(levels, total, quiet || q, "Cleaning", move |s, _pb| {
                        let subdir = s.clone();
                        commands::clean::run(Some(s), force, true)?;
                        Ok(format!("{}", format!("Cleaned '{subdir}'").green()))
                    })
                    .await?;
                    Ok(())
                } else {
                    commands::clean::run(subdir, force, quiet || q)
                }
            }
            Some(Commands::Config { subdir, key, value }) => {
                commands::config::run(subdir, key, value, force)
            }
        }
    }
    .await;

    if let Err(e) = result {
        eprintln!("git-subrepo: {e}");
        std::process::exit(1);
    }

    // suppress unused warning
    let _ = no_edit;
    let _ = verbose;
}
