//! Resolve user paths before dispatch while keeping the caller's cwd intact.
use super::{Context, location};
use crate::cli::{Cli, Commands};
use crate::git_utils::try_run_git;
use anyhow::{Context as _, Result, ensure};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

pub(super) struct ResolvedPath {
    pub path: String,
    pub root_fallback: bool,
}

fn physical_path(path: &Path) -> Result<PathBuf> {
    let mut parent = path;
    let mut suffix = Vec::new();
    while parent.symlink_metadata().is_err() {
        suffix.push(parent.file_name().context("Cannot resolve target path")?);
        parent = parent.parent().context("Cannot resolve target parent")?;
    }
    let mut resolved = parent.canonicalize()?;
    for name in suffix.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

/// Resolve cwd-relative paths, with an explicit compatibility fallback for known subrepos.
pub(super) fn resolve_path(
    root: &Path,
    cwd: &Path,
    value: &str,
    fallback: bool,
) -> Result<ResolvedPath> {
    ensure!(!value.is_empty(), "Subrepo path cannot be empty");
    let candidate = physical_path(&cwd.join(value))?;
    let mut root_fallback = false;
    let candidate = if fallback
        && !candidate.exists()
        && !Path::new(value).is_absolute()
        && !value.starts_with('.')
    {
        let other = physical_path(&root.join(value))?;
        if other.join(".gitrepo").is_file() {
            root_fallback = true;
            other
        } else {
            candidate
        }
    } else {
        candidate
    };
    let relative = candidate.strip_prefix(root).with_context(|| {
        format!(
            "Subrepo path is outside the selected checkout: {}",
            candidate.display()
        )
    })?;
    ensure!(
        !relative.as_os_str().is_empty(),
        "The owning checkout root cannot itself be a subrepo target"
    );
    Ok(ResolvedPath {
        path: relative
            .to_str()
            .context("Subrepo path must be UTF-8")?
            .to_owned(),
        root_fallback,
    })
}

fn optional_target(root: &Path, cwd: &Path, value: &mut Option<String>, all: bool) -> Result<bool> {
    if let Some(path) = value {
        let resolved = resolve_path(root, cwd, path, true)?;
        *path = resolved.path;
        return Ok(resolved.root_fallback);
    }
    if !all {
        *value =
            location::nearest_subrepo(cwd.strip_prefix(root)?, &location::tracked_subrepos(root)?)
                .map(str::to_owned);
    }
    Ok(false)
}

fn remote(root: &Path, cwd: &Path, value: &mut Option<String>) -> Result<()> {
    // At the root a relative path already means the same thing; keep it portable in .gitrepo.
    if cwd == root {
        return Ok(());
    }
    if let Some(value) = value {
        let named = try_run_git(&["remote", "get-url", value], root).0;
        if !named && cwd.join(&*value).exists() {
            *value = cwd
                .join(&*value)
                .canonicalize()?
                .to_string_lossy()
                .into_owned();
        }
    }
    Ok(())
}

fn confirm(
    root: &Path,
    cwd: &Path,
    target: &str,
    fallback: bool,
    write: bool,
    cli: &Cli,
) -> Result<bool> {
    if cwd == root {
        return Ok(true);
    }
    let interactive =
        write && !cli.yes && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !cli.quiet || fallback || interactive {
        eprintln!("Checkout: {}\nTarget: {target}", root.display());
        if fallback {
            eprintln!(
                "Using the existing checkout-root-relative subrepo; the cwd-relative path does not exist."
            );
        }
    }
    if !write || cli.yes {
        return Ok(true);
    }
    if interactive {
        return Ok(
            dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                .with_prompt("Execute in this checkout?")
                .default(false)
                .interact_opt()?
                .unwrap_or(false),
        );
    }
    ensure!(
        !fallback,
        "Root-relative fallback needs confirmation. Use a cwd-relative path (for example '.') or --yes after checking the checkout and target."
    );
    Ok(true)
}

/// Discover the checkout, normalize user-facing paths, and confirm interactive redirection.
pub fn prepare(cli: &mut Cli) -> Result<bool> {
    // Preserve command syntax errors before repository discovery or confirmation.
    if matches!(&cli.command, Some(Commands::Clone(args)) if cli.all || cli.all_all || !args.extra.is_empty())
        || matches!(&cli.command, Some(Commands::Pull(args)) if args.update && args.branch.is_none() && args.remote.is_none() && !cli.all && !cli.all_all)
        || matches!(&cli.command, Some(Commands::Pr(_)) if cli.all || cli.all_all || cli.force || cli.fetch)
    {
        return Ok(true);
    }
    if matches!(cli.command, None | Some(Commands::Completion(_)))
        || matches!(&cli.command, Some(Commands::Pr(args)) if args.action.is_none())
    {
        return Ok(true);
    }
    let cwd = std::env::current_dir()?.canonicalize()?;
    if let Some(Commands::Commit(args)) = &cli.command {
        let destination = location::resolve_commit(&cwd, args.subdir.as_deref())?;
        let fallback = if destination.source_worktree.is_none() {
            args.subdir
                .as_deref()
                .map(|path| resolve_path(&destination.context.repo_root, &cwd, path, true))
                .transpose()?
                .is_some_and(|path| path.root_fallback)
        } else {
            false
        };
        return confirm(
            &destination.context.repo_root,
            &cwd,
            &destination.subdir,
            fallback,
            true,
            cli,
        );
    }
    let root = Context::at(&cwd)?.repo_root;
    let all = cli.all || cli.all_all;
    let mut fallback = false;
    let mut write = true;
    let target;
    match cli.command.as_mut().expect("command checked") {
        Commands::Clone(args) => {
            let path = args
                .subdir
                .clone()
                .map(Ok)
                .unwrap_or_else(|| super::clone::guess_subdir(&args.remote))?;
            args.subdir = Some(resolve_path(&root, &cwd, &path, false)?.path);
            let mut source = Some(args.remote.clone());
            remote(&root, &cwd, &mut source)?;
            args.remote = source.expect("remote supplied");
            target = args.subdir.clone().unwrap();
        }
        Commands::Init(args) => {
            args.subdir = resolve_path(&root, &cwd, &args.subdir, false)?.path;
            remote(&root, &cwd, &mut args.remote)?;
            target = args.subdir.clone();
        }
        Commands::Config(args) => {
            let resolved = resolve_path(&root, &cwd, &args.subdir, true)?;
            args.subdir = resolved.path;
            fallback = resolved.root_fallback;
            write = args.value.is_some();
            target = args.subdir.clone();
        }
        Commands::Workon(args) => {
            let resolved = resolve_path(&root, &cwd, &args.subdir, true)?;
            args.subdir = resolved.path;
            fallback = resolved.root_fallback;
            remote(&root, &cwd, &mut args.upstream)?;
            target = args.subdir.clone();
        }
        Commands::Pull(args) => {
            fallback = optional_target(&root, &cwd, &mut args.subdir, all)?;
            remote(&root, &cwd, &mut args.remote)?;
            target = if all {
                "all selected subrepos".to_owned()
            } else {
                args.subdir
                    .clone()
                    .context("Command 'pull' requires arg 'subdir'.")?
            };
        }
        Commands::Push(args) => {
            fallback = optional_target(&root, &cwd, &mut args.subdir, all)?;
            remote(&root, &cwd, &mut args.remote)?;
            target = if all {
                "all selected subrepos".to_owned()
            } else {
                args.subdir
                    .clone()
                    .context("Command 'push' requires arg 'subdir'.")?
            };
        }
        Commands::Fetch(args) => {
            fallback = optional_target(&root, &cwd, &mut args.subdir, all)?;
            remote(&root, &cwd, &mut args.remote)?;
            target = if all {
                "all selected subrepos".to_owned()
            } else {
                args.subdir
                    .clone()
                    .context("Command 'fetch' requires arg 'subdir'.")?
            };
        }
        Commands::Branch(args) => {
            fallback = optional_target(&root, &cwd, &mut args.subdir, all)?;
            target = if all {
                "all selected subrepos".to_owned()
            } else {
                args.subdir
                    .clone()
                    .context("Command 'branch' requires arg 'subdir'.")?
            };
        }
        Commands::Clean(args) => {
            fallback = optional_target(&root, &cwd, &mut args.subdir, all)?;
            target = if all {
                "all selected subrepos".to_owned()
            } else {
                args.subdir
                    .clone()
                    .context("Command 'clean' requires arg 'subdir'.")?
            };
        }
        Commands::Status(args) => {
            fallback = optional_target(&root, &cwd, &mut args.subdir, all)?;
            write = false;
            target = args
                .subdir
                .clone()
                .unwrap_or_else(|| "all selected subrepos".to_owned());
        }
        Commands::Pr(args) => {
            fallback = optional_target(&root, &cwd, &mut args.subdir, false)?;
            remote(&root, &cwd, &mut args.remote)?;
            remote(&root, &cwd, &mut args.upstream)?;
            write = args.action.as_deref() != Some("status");
            target = args.subdir.clone().context(
                "PR operation requires <subdir>; run 'git subrepo pr' for guided actions",
            )?;
        }
        Commands::Sync | Commands::Fix => {
            target = "all selected subrepos".to_owned();
        }
        Commands::Completion(_) | Commands::Commit(_) => unreachable!(),
    }
    confirm(&root, &cwd, &target, fallback, write, cli)
}
