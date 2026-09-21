use crate::commands::{
    Context, branch_is_stale, normalize_subdir, subrepo_branch, worktree_is_dirty,
};
use crate::encode::encode_subdir;
use crate::git_utils::{branch_exists, rev_parse, rev_parse_short, run_git, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::Result;
use colored::Colorize;
use std::io::IsTerminal;

/// Ensure a remote named `name` points at `url` — added if missing, left alone (with a
/// warning) if it already exists pointing somewhere else, since it might belong to
/// something unrelated. Remotes are shared repo-wide config, not per-worktree, so this
/// never silently overwrites. Returns true if `name` is now safely usable as-is.
fn ensure_named_remote(ctx: &Context, name: &str, url: &str) -> Result<bool> {
    let (existing_ok, existing_url) = try_run_git(&["remote", "get-url", name], &ctx.repo_root);
    if existing_ok && existing_url != url {
        eprintln!(
            "git-subrepo: a remote named '{name}' already exists (→ '{existing_url}'); not \
             overwriting it. Fetch/push '{url}' directly by URL instead, or rename/remove the \
             existing '{name}' remote first."
        );
        Ok(false)
    } else if !existing_ok {
        run_git(&["remote", "add", name, url], &ctx.repo_root)?;
        Ok(true)
    } else {
        Ok(true)
    }
}

/// Ask `url` (via `ls-remote`, no local fetch) what its default branch is, for when
/// `remote`'s tracked branch doesn't exist there (e.g. a feature branch created only
/// after forking). Read-only; returns `None` on any failure.
fn upstream_default_branch(ctx: &Context, url: &str) -> Option<String> {
    let (ok, out) = try_run_git(&["ls-remote", "--symref", url, "HEAD"], &ctx.repo_root);
    if !ok {
        return None;
    }
    out.lines().find_map(|line| {
        line.strip_prefix("ref: refs/heads/")?
            .split_whitespace()
            .next()
            .map(str::to_string)
    })
}

/// Best-effort, read-only check: has `pinned` (the commit this subrepo is currently
/// pinned to) been rebased/force-pushed away on `url`'s `branch`? Fetches into a
/// throwaway ref (never touches the working tree or FETCH_HEAD) and checks ancestry
/// locally. Returns the remote's current tip (short SHA) if it has drifted away from
/// `pinned`, or `None` if aligned, unpinned, or the fetch itself failed (e.g. offline —
/// this must never block `workon`, it's advisory only).
fn check_drift(
    ctx: &Context,
    url: &str,
    branch: &str,
    pinned: &str,
    scratch_ref: &str,
) -> Option<String> {
    if pinned.is_empty() {
        return None;
    }
    let (ok, _) = try_run_git(
        &[
            "fetch",
            "--no-tags",
            "--quiet",
            url,
            &format!("+refs/heads/{branch}:{scratch_ref}"),
        ],
        &ctx.repo_root,
    );
    if !ok {
        return None;
    }
    let tip = rev_parse(scratch_ref, &ctx.repo_root)?;
    let (is_ancestor, _) = try_run_git(
        &["merge-base", "--is-ancestor", pinned, &tip],
        &ctx.repo_root,
    );
    if is_ancestor {
        None
    } else {
        rev_parse_short(&tip, &ctx.repo_root)
    }
}

/// Open a `subrepo/<subdir>` worktree wired up with real remotes:
///   - plain `git push`/`git pull`/`git fetch` always reach `remote` — exactly like
///     `git subrepo push`/`pull` already do, unchanged.
///   - if `upstream` is configured (the original project, when `remote` is your own
///     fork of it), it's additionally exposed as a plain named `upstream` remote, for
///     explicit, deliberate use (`git fetch upstream`, `git push upstream ...`) —
///     never the default target for bare push/pull.
///
/// Both `remote` and `upstream` are added as real named remotes (named literally
/// "remote"/"upstream", matching `.gitrepo`'s own field names) so plain `git log
/// --decorate` shows real, live tracking refs — never left as an opaque raw URL you
/// can't see anywhere. We never blindly `git remote add`, though: remotes are shared
/// repo-wide config, not per-worktree, so a name already taken by something unrelated
/// (most plausibly "remote" colliding with a real remote of that literal name, or
/// occasionally "upstream") is left untouched with a warning, falling back to a raw
/// URL in `branch.<name>.remote` for `remote` specifically (still fully functional,
/// just without the log decoration).
#[allow(clippy::too_many_arguments)]
pub fn run(
    subdir: String,
    upstream_override: Option<String>,
    force: bool,
    fetch: bool,
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

    // Wire up branch-scoped remote config. branch.<name>.remote/.merge drive bare
    // `git pull`/`fetch`/`push` — always `remote`, matching `git subrepo push`/`pull`.
    // Prefer a real named remote (scoped per-subrepo — `{subref}-remote`, not a bare
    // "remote" — since remote names are repo-global: a second subrepo's `workon`
    // would otherwise collide with the first, e.g. a nested `foo` and `foo/bar` both
    // wanting their own "remote"). Falls back to a raw URL if that exact scoped name
    // is somehow already taken by something unrelated — still fully functional, just
    // invisible to `git log --decorate`; see module doc.
    let remote_name = format!("{subref}-remote");
    let remote_ready = ensure_named_remote(&ctx, &remote_name, &cfg.remote)?;
    let branch_remote_value: &str = if remote_ready {
        &remote_name
    } else {
        &cfg.remote
    };
    run_git(
        &[
            "config",
            &format!("branch.{branch_name}.remote"),
            branch_remote_value,
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
    if remote_ready {
        // Best-effort: populates refs/remotes/<remote_name>/<branch> so a plain
        // decorated `git log` shows exactly where `remote` currently sits relative to
        // HEAD.
        try_run_git(
            &["fetch", "--no-tags", "--quiet", &remote_name, &cfg.branch],
            &ctx.repo_root,
        );
    }

    // If configured, also expose `upstream` as a plain named remote for explicit,
    // deliberate use (never the default target). Scoped per-subrepo for the same
    // reason as `remote_name` above: `{subref}-upstream`, not a bare "upstream".
    let mut divergence_marker: Option<String> = None;
    if let Some(url) = &upstream_url {
        let upstream_name = format!("{subref}-upstream");
        let upstream_ready = ensure_named_remote(&ctx, &upstream_name, url)?;

        // Best-effort: fetch upstream's tracked branch — this populates the normal
        // `refs/remotes/<upstream_name>/<branch>` tracking ref for free, so plain
        // `git log` (with --decorate, e.g. a `git lg` alias) shows it right where it
        // currently sits — then drop a permanent tag at the exact commit where local
        // history diverges from it, so that boundary stays visible even after
        // upstream moves further. Never fatal: offline, wrong branch name, etc. just
        // means no marker this time, not a broken `workon`.
        if upstream_ready {
            // `remote`'s tracked branch (e.g. a feature branch created after forking)
            // often simply doesn't exist on `upstream` — fall back to upstream's own
            // default branch rather than giving up.
            let mut upstream_branch = cfg.branch.clone();
            let mut fetch_ok = try_run_git(
                &[
                    "fetch",
                    "--no-tags",
                    "--quiet",
                    &upstream_name,
                    &upstream_branch,
                ],
                &ctx.repo_root,
            )
            .0;
            if !fetch_ok && let Some(default_branch) = upstream_default_branch(&ctx, url) {
                fetch_ok = try_run_git(
                    &[
                        "fetch",
                        "--no-tags",
                        "--quiet",
                        &upstream_name,
                        &default_branch,
                    ],
                    &ctx.repo_root,
                )
                .0;
                if fetch_ok {
                    upstream_branch = default_branch;
                }
            }
            if fetch_ok {
                let upstream_tracking_ref =
                    format!("refs/remotes/{upstream_name}/{upstream_branch}");
                if let Some(upstream_tip) = rev_parse(&upstream_tracking_ref, &ctx.repo_root) {
                    let (base_ok, base) =
                        try_run_git(&["merge-base", &branch_name, &upstream_tip], &ctx.repo_root);
                    let base = base.trim();
                    if base_ok && !base.is_empty() {
                        let marker = format!("{subref}-upstream-base");
                        if run_git(&["tag", "-f", &marker, base], &ctx.repo_root).is_ok() {
                            divergence_marker = Some(marker);
                        }
                    }
                }
            }
        }
    }

    // Opt-in (network cost): has `remote`/`upstream`'s branch moved past the pinned
    // commit in a way that means it was rebased/force-pushed, not just fast-forwarded?
    // Advisory only — never attempted to auto-resolve; `git subrepo pull` is the right
    // tool for that, since it requires real conflict resolution.
    if fetch {
        if let Some(tip_short) = check_drift(
            &ctx,
            &cfg.remote,
            &cfg.branch,
            &cfg.commit,
            &format!("refs/subrepo/{subref}/workon-drift-check"),
        ) {
            let pinned_short = cfg.commit.get(..7).unwrap_or(&cfg.commit);
            eprintln!(
                "{} remote's {} has moved past the pinned commit ({pinned_short}) — it looks like \
                 it was rebased/force-pushed since (remote is now at {tip_short}). Merging here \
                 will likely hit real conflicts from that rewrite.\n  Recommended: run \
                 `git subrepo pull {subdir}` first to re-sync.",
                "⚠".yellow().bold(),
                cfg.branch
            );
        }
        if let Some(url) = &upstream_url
            && let Some(tip_short) = check_drift(
                &ctx,
                url,
                &cfg.branch,
                &cfg.commit,
                &format!("refs/subrepo/{subref}/workon-drift-check-upstream"),
            )
        {
            let pinned_short = cfg.commit.get(..7).unwrap_or(&cfg.commit);
            eprintln!(
                "{} upstream's {} has moved past the pinned commit ({pinned_short}) — \
                 current upstream tip is {tip_short}.",
                "⚠".yellow().bold(),
                cfg.branch
            );
        }
    }

    if !quiet {
        println!(
            "{} workon session for '{}' at '{}'.",
            if reused { "Resumed" } else { "Opened" },
            subdir.green().bold(),
            worktree_display
        );
        let pin_note = if cfg.commit.is_empty() {
            String::new()
        } else {
            format!(", pinned @ {}", cfg.commit.get(..7).unwrap_or(&cfg.commit))
        };
        let remote_note = if remote_ready {
            format!(", as remote '{}' — see it with `git log`", remote_name)
        } else {
            String::new()
        };
        println!(
            "  remote:   {} [{}] (default push/pull{pin_note}{remote_note})",
            cfg.remote, cfg.branch
        );
        if let Some(url) = &upstream_url {
            let upstream_name = format!("{subref}-upstream");
            println!(
                "  upstream: {url} (added as remote '{upstream_name}' — use e.g. `git fetch {upstream_name}`)"
            );
            if let Some(marker) = &divergence_marker {
                println!(
                    "  Local work diverges from upstream at tag '{}' — see it with `git log`.",
                    marker.cyan()
                );
            }
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
