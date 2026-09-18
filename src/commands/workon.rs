use crate::commands::{
    Context, branch_is_stale, normalize_subdir, subrepo_branch, worktree_is_dirty,
};
use crate::encode::encode_subdir;
use crate::git_utils::{branch_exists, run_git, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;

/// Open a `subrepo/<subdir>` worktree wired up with real remotes:
///   - plain `git push`/`git pull`/`git fetch` always reach `remote` — exactly like
///     `git subrepo push`/`pull` already do, unchanged.
///   - if `upstream` is configured (the original project, when `remote` is your own
///     fork of it), it's additionally exposed as a plain named `upstream` remote, for
///     explicit, deliberate use (`git fetch upstream`, `git push upstream ...`) —
///     never the default target for bare push/pull.
///
/// For the default (bare) push/pull we avoid `git remote add`: remotes live in the
/// shared repo config, not per-worktree, so adding one (e.g. named "origin") would
/// also mutate the main repo's config and could collide with an existing remote of
/// the same name. Instead we set `branch.<name>.remote`/`.merge` (git accepts a URL
/// there, not just a remote name), scoped to the unique `subrepo/<subdir>` branch
/// name — so it can't collide with anything. The `upstream` remote (added only when
/// configured) is a deliberate exception: it's meant to be invoked by name.
pub fn run(
    subdir: String,
    upstream_override: Option<String>,
    force: bool,
    quiet: bool,
    no_shell: bool,
) -> Result<()> {
    let mut ctx = Context::new()?;
    ctx.quiet = quiet;

    let subdir = normalize_subdir(&subdir);
    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let cfg = read_gitrepo(&gitrepo_path, &ctx.repo_root)?;

    if cfg.remote.is_empty() || cfg.remote == "none" {
        anyhow::bail!(
            "Subrepo '{subdir}' has no remote configured.\n\
             Run 'git subrepo config {subdir} remote <url> --force' first."
        );
    }

    let upstream_url = upstream_override
        .filter(|s| !s.is_empty())
        .or_else(|| Some(cfg.upstream.clone()).filter(|s| !s.is_empty()));

    let branch_name = format!("subrepo/{subref}");
    let worktree_display = ctx.worktree_display(&subdir);
    let worktree_path = ctx.worktree_path(&subdir);

    let exists = branch_exists(&branch_name, &ctx.repo_root);
    // A branch that's up to date is only truly "reused" if its worktree directory is
    // still there — it can vanish under us (manual `rm -rf`, cleaned-up CI workspace)
    // without the branch itself being touched.
    let reused =
        !force && exists && worktree_path.is_dir() && !branch_is_stale(&ctx, &subdir, &branch_name);

    if !reused {
        // About to (re)build the branch/worktree. If one already exists and holds
        // uncommitted work, don't silently discard it.
        if exists && !force && worktree_is_dirty(&worktree_path) {
            anyhow::bail!(
                "Worktree '{worktree_display}' has uncommitted changes, but mainline has \
                 moved on and it needs to be rebuilt.\n\
                 Commit or stash your changes there first, or re-run with --force to discard them."
            );
        }
        subrepo_branch(
            &ctx,
            &subdir,
            &subref,
            &cfg.parent,
            &cfg.method,
            force || exists,
        )?;
    }

    // Wire up branch-scoped remote config (see module doc for why not `git remote add`
    // for this part). branch.<name>.remote/.merge drive bare `git pull`/`fetch`/`push` —
    // always `remote`, matching `git subrepo push`/`pull`.
    run_git(
        &[
            "config",
            &format!("branch.{branch_name}.remote"),
            &cfg.remote,
        ],
        &ctx.repo_root,
    )?;
    run_git(
        &[
            "config",
            &format!("branch.{branch_name}.merge"),
            &format!("refs/heads/{}", cfg.branch),
        ],
        &ctx.repo_root,
    )?;
    // Clean up any pushRemote override left behind by an older version of this command.
    try_run_git(
        &[
            "config",
            "--unset",
            &format!("branch.{branch_name}.pushRemote"),
        ],
        &ctx.repo_root,
    );

    // If configured, also expose `upstream` as a plain named remote for explicit,
    // deliberate use (never the default target). Remotes are shared repo-wide config,
    // not per-worktree, so only ever add — never overwrite an existing "upstream"
    // remote that points somewhere else; it might belong to something unrelated.
    if let Some(url) = &upstream_url {
        let (existing_ok, existing_url) =
            try_run_git(&["remote", "get-url", "upstream"], &ctx.repo_root);
        if existing_ok && &existing_url != url {
            eprintln!(
                "git-subrepo: a remote named 'upstream' already exists (→ '{existing_url}'); \
                 not overwriting it. Fetch/push '{url}' directly by URL instead, or rename/remove \
                 the existing 'upstream' remote first."
            );
        } else if !existing_ok {
            run_git(&["remote", "add", "upstream", url], &ctx.repo_root)?;
        }
    }

    if !quiet {
        println!(
            "{} workon session for '{}' at '{}'.",
            if reused { "Resumed" } else { "Opened" },
            subdir.green().bold(),
            worktree_display
        );
        println!(
            "  remote:   {} [{}] (default push/pull)",
            cfg.remote, cfg.branch
        );
        if let Some(url) = &upstream_url {
            println!(
                "  upstream: {url} (added as remote 'upstream' — use e.g. `git fetch upstream`)"
            );
        }
        println!(
            "  {}",
            "Note: `git subrepo push/pull` on this subdir elsewhere rebuilds this worktree \
             from mainline and discards anything not pushed from here yet."
                .dimmed()
        );
    }

    if no_shell || !std::io::stdout().is_terminal() {
        if !quiet && std::io::stdout().is_terminal() {
            println!("  {}", format!("cd '{worktree_display}'").bright_cyan());
        }
        return Ok(());
    }

    // Spawning with a nonexistent `current_dir` fails with the same ENOENT as a
    // missing shell binary, which would otherwise misleadingly blame $SHELL — this
    // should be unreachable given the rebuild above, but check explicitly rather
    // than risk a confusing error message.
    if !worktree_path.is_dir() {
        anyhow::bail!("Worktree '{worktree_display}' does not exist after setup — this is a bug.");
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    match std::process::Command::new(&shell)
        .current_dir(&worktree_path)
        .env("GIT_SUBREPO_WORKON", &subdir)
        .status()
    {
        Ok(status) => {
            if !quiet {
                println!(
                    "Left workon session for '{subdir}' (shell exit: {}).",
                    status.code().unwrap_or(-1)
                );
            }
        }
        Err(e) => {
            eprintln!("Could not spawn shell '{shell}': {e}");
            println!("  {}", format!("cd '{worktree_display}'").bright_cyan());
        }
    }

    Ok(())
}
