//! Guided PR actions using the same terminal widgets as the other subcommands.
use super::{Context, discovery};
use crate::commands::location;
use crate::git_utils::run_git;
use crate::gitrepo::read_gitrepo;
use anyhow::{Result, ensure};
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy)]
enum Action {
    Open,
    ReviewExport,
    ReviewSource,
    Prepare,
    Publish,
    Reconcile,
}

/// Open the guided menu from an owner checkout, its subdirectories, or an export.
pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = PathBuf::from(run_git(&["rev-parse", "--show-toplevel"], &cwd)?);
    let (root, selected) = match discovery::export_owner(&root)? {
        Some((owner, subdir)) => (owner, Some(subdir)),
        None => (root, None),
    };
    let subrepos = location::tracked_subrepos(&root)?;
    if subrepos.is_empty() {
        println!(
            "No tracked subrepos in {}. Run 'git subrepo help' for clone/init commands.",
            root.display()
        );
        return Ok(());
    }
    let selected = selected.or_else(|| {
        let relative = cwd.strip_prefix(&root).ok()?;
        subrepos
            .iter()
            .filter(|s| relative.starts_with(s))
            .max_by_key(|s| Path::new(s).components().count())
            .cloned()
    });
    let subdir = if let Some(subdir) = selected {
        subdir
    } else if subrepos.len() == 1 {
        subrepos[0].clone()
    } else {
        println!("\nChoose a subrepo in {}", root.display());
        let Some(index) = choose("Select subrepo", &subrepos)? else {
            return Ok(());
        };
        subrepos[index].clone()
    };
    loop {
        // Shells may commit or switch branches; refresh before showing next actions.
        let ctx = Context::at(&root)?;
        let exports = discovery::exports(&ctx, &subdir)?;
        let changes = discovery::source_changes(&ctx, &subdir)?;
        println!(
            "\n{subdir}\nCurrent checkout: {} ({})\nLocal changes: {}",
            root.display(),
            ctx.original_head_branch,
            if changes.is_empty() {
                "committed"
            } else {
                "uncommitted"
            }
        );
        for export in &exports {
            println!(
                "Existing export: {} [{}]\n  Owner: {}\n  Destination: {}",
                export.branch,
                export_status(export)?,
                export.owner.display(),
                export.remote
            );
        }
        let mut actions = Vec::new();
        if !exports.is_empty() {
            actions.extend([
                (Action::Open, "Open existing export"),
                (Action::ReviewExport, "Review export differences"),
            ]);
        }
        actions.extend([
            (Action::ReviewSource, "Review local changes"),
            (Action::Prepare, "Prepare new contribution"),
        ]);
        if exports.iter().any(|e| e.phase == "prepared") {
            actions.push((Action::Publish, "Publish reviewed export"));
        }
        if exports.iter().any(|e| {
            !e.published.is_empty() && ["prepared", "resolving"].contains(&e.phase.as_str())
        }) {
            actions.push((Action::Reconcile, "Bring accepted upstream changes back"));
        }
        let labels: Vec<_> = actions.iter().map(|(_, label)| label.to_string()).collect();
        let Some(index) = choose("Select action", &labels)? else {
            return Ok(());
        };
        // A recoverable operation error returns to the menu with the context intact.
        if let Err(error) = perform(actions[index].0, &ctx, &subdir, &exports) {
            eprintln!("git-subrepo: {error}");
        }
    }
}

fn perform(
    action: Action,
    ctx: &Context,
    subdir: &str,
    exports: &[discovery::Export],
) -> Result<()> {
    match action {
        Action::ReviewSource => {
            println!("{}", discovery::source_changes(ctx, subdir)?);
            let path = format!(":(literal){subdir}");
            git_view(&ctx.repo_root, &["diff", "--", &path])?;
            git_view(&ctx.repo_root, &["diff", "--cached", "--", &path])
        }
        Action::Prepare => prepare(ctx, subdir),
        _ => {
            let eligible: Vec<_> = exports
                .iter()
                .filter(|e| match action {
                    Action::Publish => e.phase == "prepared",
                    Action::Reconcile => {
                        !e.published.is_empty()
                            && ["prepared", "resolving"].contains(&e.phase.as_str())
                    }
                    _ => true,
                })
                .collect();
            let Some(export) = select_export(&eligible)? else {
                return Ok(());
            };
            println!(
                "\nExport: {}\nOwner: {}\nRepository: {}",
                export.branch,
                export.owner.display(),
                export.repository.display()
            );
            match action {
                Action::Open => open_shell(&export.repository),
                Action::ReviewExport => {
                    let head = run_git(&["rev-parse", "HEAD"], &export.repository)?;
                    review_export(export, &head)
                }
                Action::Publish => {
                    let head = run_git(&["rev-parse", "HEAD"], &export.repository)?;
                    review_export(export, &head)?;
                    println!(
                        "\nRevision: {head}\nPush to: {} ({})\nContinue only after reviewing and testing this revision. This publishes the branch; it does not create a GitHub PR.",
                        export.remote, export.branch
                    );
                    if confirm("Publish this exact revision?")? {
                        super::confirmed(export, subdir, "publish", &head)?;
                    }
                    Ok(())
                }
                Action::Reconcile => {
                    println!(
                        "Import accepted code from {} into {}. This stages changes; it does not commit them.",
                        export.upstream,
                        export.owner.display()
                    );
                    let merged = prompt("Merged commit SHA from the accepted upstream PR", None)?;
                    if confirm("Stage this accepted contribution in its owning checkout?")? {
                        super::confirmed(export, subdir, "reconcile", &merged)?;
                    }
                    Ok(())
                }
                _ => unreachable!(),
            }
        }
    }
}

fn prepare(ctx: &Context, subdir: &str) -> Result<()> {
    discovery::require_committed_source(ctx, subdir)?;
    let current = discovery::exports(ctx, subdir)?;
    if current
        .iter()
        .any(|e| e.owner == ctx.repo_root && ["prepared", "resolving"].contains(&e.phase.as_str()))
    {
        println!(
            "An export is already active in this checkout. Choose Open existing export to continue it."
        );
        return Ok(());
    }
    let cfg = read_gitrepo(&ctx.repo_root.join(subdir).join(".gitrepo"), &ctx.repo_root)?;
    let name = Path::new(subdir)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let suggested_branch = format!("contribute/{name}-{}", &ctx.original_head_commit[..7]);
    let suggested_message = format!("Update {name}");
    let upstream = if cfg.upstream.is_empty() {
        &cfg.remote
    } else {
        &cfg.upstream
    };
    let branch = prompt("Topic branch", Some(&suggested_branch))?;
    let message = prompt("Public commit message", Some(&suggested_message))?;
    let upstream = prompt("Upstream URL", Some(upstream))?;
    let base = prompt("Base branch", Some(&cfg.branch))?;
    let remote = prompt("Push URL", Some(&cfg.remote))?;
    println!("Excluded from export: root .gitrepo");
    for path in &cfg.exclude {
        println!("Excluded local changes: {path:?}");
    }
    println!(
        "\nSource: {}\nScope: committed changes in {subdir}, respecting configured exclusions\nUpstream: {upstream} ({base})\nTopic: {branch}\nPush destination: {remote}\nPublic message: {message}\nPreparation creates a local export commit; it does not push or commit the source checkout.",
        ctx.repo_root.display()
    );
    if confirm("Prepare this contribution?")? {
        subrepo(
            &ctx.repo_root,
            &[
                "pr",
                "prepare",
                subdir,
                "--branch",
                &branch,
                "--message",
                &message,
                "--upstream",
                &upstream,
                "--base",
                &base,
                "--remote",
                &remote,
            ],
        )?;
    }
    Ok(())
}

fn export_status(export: &discovery::Export) -> Result<&'static str> {
    match export.phase.as_str() {
        "resolving" => return Ok("import needs conflict resolution"),
        "staged" => return Ok("import staged; review and commit in owner"),
        _ => {}
    }
    if !export.repository.is_dir() {
        return Ok("checkout missing");
    }
    if !run_git(&["status", "--porcelain"], &export.repository)?.is_empty() {
        return Ok("uncommitted review changes");
    }
    if export.published.is_empty() {
        Ok("local; not published")
    } else if run_git(&["rev-parse", "HEAD"], &export.repository)? == export.published {
        Ok("published")
    } else {
        Ok("new commits to publish")
    }
}

fn review_export(export: &discovery::Export, head: &str) -> Result<()> {
    let range = format!("{}..{head}", export.base);
    git_view(&export.repository, &["log", "--format=fuller", &range])?;
    git_view(&export.repository, &["diff", &range])?;
    git_view(&export.repository, &["status", "--short"])
}

fn select_export<'a>(exports: &[&'a discovery::Export]) -> Result<Option<&'a discovery::Export>> {
    if exports.len() == 1 {
        return Ok(Some(exports[0]));
    }
    let labels: Vec<_> = exports
        .iter()
        .map(|e| format!("{} — {}", e.branch, e.owner.display()))
        .collect();
    Ok(choose("Select export", &labels)?.map(|i| exports[i]))
}

fn choose(prompt: &str, labels: &[String]) -> Result<Option<usize>> {
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("{prompt} (↑/↓ move, Enter select, Esc back)"))
        .items(labels)
        .item("Quit / back")
        .default(0)
        .interact_opt()?;
    Ok(selection.filter(|&index| index < labels.len()))
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    let theme = ColorfulTheme::default();
    let mut input = Input::<String>::with_theme(&theme).with_prompt(label);
    if let Some(value) = default {
        input = input.default(value.to_owned());
    }
    Ok(input.interact_text()?)
}

fn confirm(label: &str) -> Result<bool> {
    Ok(Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(label)
        .default(false)
        .interact_opt()?
        .unwrap_or(false))
}

fn subrepo(owner: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new(std::env::current_exe()?)
        .args(args)
        .current_dir(owner)
        .status()?;
    ensure!(
        status.success(),
        "Operation stopped; see the details above."
    );
    Ok(())
}

fn git_view(cwd: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .arg("--no-pager")
        .args(args)
        .current_dir(cwd)
        .status()?;
    ensure!(status.success(), "Could not display Git changes.");
    Ok(())
}

fn open_shell(path: &Path) -> Result<()> {
    ensure!(
        path.is_dir(),
        "Checkout is missing: {}. It has not been rebuilt.",
        path.display()
    );
    println!(
        "Opening {}. Type exit to return to the menu.",
        path.display()
    );
    io::stdout().flush()?;
    Command::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()))
        .current_dir(path)
        .env_remove("GIT_SUBREPO_WORKON")
        .status()?;
    Ok(())
}
