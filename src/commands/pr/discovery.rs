//! Read-only discovery shared by the menu and actionable command diagnostics.
use super::{git, state::Session};
use crate::commands::{Context, location};
use crate::git_utils::run_git;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// An export and the owning checkout whose accepted baseline it will update.
pub(crate) struct Export {
    pub owner: PathBuf,
    pub repository: PathBuf,
    pub branch: String,
    pub phase: String,
    pub published: String,
    pub base: String,
    pub base_branch: String,
    pub upstream: String,
    pub remote: String,
}

/// Resolve the checkout-local session directory for a normalized subrepo path.
pub(super) fn session_root(owner: &Path, subdir: &str) -> Result<PathBuf> {
    let dir = run_git(&["rev-parse", "--absolute-git-dir"], owner)?;
    let key = git::bytes(
        owner,
        &["hash-object", "--stdin"],
        Some(subdir.as_bytes()),
        None,
    )?;
    Ok(PathBuf::from(dir)
        .join("subrepo-pr")
        .join(String::from_utf8(key)?.trim()))
}

/// Find retained active exports across this repository's registered checkouts.
pub(crate) fn exports(ctx: &Context, subdir: &str) -> Result<Vec<Export>> {
    let listing = run_git(&["worktree", "list", "--porcelain", "-z"], &ctx.repo_root)?;
    let mut found = Vec::new();
    for (owner, _) in location::registered_worktrees(&listing) {
        if !owner.is_dir() {
            continue;
        }
        let root = session_root(&owner, subdir)?;
        if !root.join("session").is_file() {
            continue;
        }
        let session = Session::load(&root)?;
        if session.get("phase")? == "closed" || session.get("subdir")? != subdir {
            continue;
        }
        // A moved checkout needs deliberate repair, not dispatch to its old path.
        if Path::new(session.get("owner")?).canonicalize()? != owner.canonicalize()? {
            bail!(
                "Export record belongs to a different checkout: {}",
                root.display()
            );
        }
        found.push(Export {
            owner,
            repository: session.repo()?,
            branch: session.get("branch")?.to_owned(),
            phase: session.get("phase")?.to_owned(),
            published: session.optional("published").to_owned(),
            base: session.get("base")?.to_owned(),
            base_branch: session.get("base-branch")?.to_owned(),
            upstream: session.get("upstream")?.to_owned(),
            remote: session.get("remote")?.to_owned(),
        });
    }
    found.sort_by_key(|export| (export.owner != ctx.repo_root, export.owner.clone()));
    Ok(found)
}

/// Recognize a retained export checkout so its menu can return to the owner.
pub(crate) fn export_owner(repository: &Path) -> Result<Option<(PathBuf, String)>> {
    let Some(parent) = repository.parent() else {
        return Ok(None);
    };
    if !parent.join("session").is_file() {
        return Ok(None);
    }
    let session = Session::load(parent)?;
    if session.repo()?.canonicalize()? != repository.canonicalize()? {
        return Ok(None);
    }
    Ok(Some((
        PathBuf::from(session.get("owner")?),
        session.get("subdir")?.to_owned(),
    )))
}

/// Show changed source files without treating literal subrepo names as patterns.
pub(crate) fn source_changes(ctx: &Context, subdir: &str) -> Result<String> {
    run_git(
        &[
            "status",
            "--short",
            "--untracked-files=all",
            "--",
            &format!(":(literal){subdir}"),
        ],
        &ctx.repo_root,
    )
}

/// Explain a dirty export source and point to existing work without modifying it.
pub(crate) fn require_committed_source(ctx: &Context, subdir: &str) -> Result<()> {
    let changes = source_changes(ctx, subdir)?;
    if changes.is_empty() {
        return Ok(());
    }
    let mut message = format!(
        "Uncommitted changes in subrepo '{subdir}'.\nCheckout: {}\n\n{changes}\n\nPreparation exports committed source. Review these files and commit the intended changes before preparing a new contribution.",
        ctx.repo_root.display()
    );
    for export in exports(ctx, subdir)? {
        message.push_str(&format!(
            "\n\nExisting export: {}\nOwner: {}\nRepository: {}\nRun 'git subrepo pr' in this checkout to open it; no new preparation is needed.",
            export.branch, export.owner.display(), export.repository.display()
        ));
    }
    bail!("{message}")
}
