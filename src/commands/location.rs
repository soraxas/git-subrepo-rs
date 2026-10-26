//! Resolve a commit's destination without trusting shell environment hints.
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::{Context, normalize_subdir};
use crate::encode::encode_subdir;
use crate::git_utils::{run_git, try_run_git};

pub(super) struct CommitLocation {
    pub context: Context,
    pub subdir: String,
    pub source_worktree: Option<PathBuf>,
}

/// Parse registered worktrees without losing spaces or newlines in their paths.
pub(super) fn registered_worktrees(
    porcelain: &str,
) -> impl Iterator<Item = (PathBuf, Option<&str>)> {
    porcelain.split("\0\0").filter_map(|record| {
        let root = record
            .split('\0')
            .find_map(|field| field.strip_prefix("worktree "))?;
        let branch = record
            .split('\0')
            .find_map(|field| field.strip_prefix("branch "));
        Some((PathBuf::from(root), branch))
    })
}

/// Every worktree that legacy commit cleanup could remove, including moved ones.
pub(super) fn cleanup_worktrees(ctx: &Context, subdir: &str) -> Result<Vec<PathBuf>> {
    let branch = format!("refs/heads/subrepo/{}", encode_subdir(subdir));
    let porcelain = run_git(&["worktree", "list", "--porcelain", "-z"], &ctx.repo_root)?;
    let mut paths: Vec<_> = registered_worktrees(&porcelain)
        .filter(|(_, name)| *name == Some(branch.as_str()))
        .map(|(path, _)| path)
        .filter(|path| path.exists())
        .collect();
    let standard = ctx.worktree_path(subdir);
    if standard.exists() {
        paths.push(standard);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// List tracked subrepo metadata paths, including nested subrepos.
pub(super) fn tracked_subrepos(root: &Path) -> Result<Vec<String>> {
    Ok(run_git(
        &[
            "ls-files",
            "--full-name",
            "-z",
            "--",
            ":(top,glob)**/.gitrepo",
        ],
        root,
    )?
    .split('\0')
    .filter_map(|path| path.strip_suffix("/.gitrepo"))
    .map(str::to_owned)
    .collect())
}

fn nearest_subrepo<'a>(relative: &Path, subrepos: &'a [String]) -> Option<&'a str> {
    subrepos
        .iter()
        .filter(|subdir| relative.starts_with(subdir))
        .max_by_key(|subdir| Path::new(subdir).components().count())
        .map(String::as_str)
}

pub(super) fn resolve_commit(cwd: &Path, explicit: Option<&str>) -> Result<CommitLocation> {
    let missing = || anyhow::anyhow!("Command 'commit' requires arg 'subdir'.");
    let (ok, root) = try_run_git(&["rev-parse", "--show-toplevel"], cwd);
    if !ok {
        if explicit.is_none() {
            return Err(missing());
        }
        bail!("Not inside a git repository.");
    }
    let root = PathBuf::from(root).canonicalize()?;
    let common = root
        .join(run_git(&["rev-parse", "--git-common-dir"], &root)?)
        .canonicalize()?;
    let git_dir =
        PathBuf::from(run_git(&["rev-parse", "--absolute-git-dir"], &root)?).canonicalize()?;
    let workon_base = common.join("tmp/subrepo");
    let path_subdir = root.strip_prefix(&workon_base).ok().and_then(Path::to_str);
    let (_, branch) = try_run_git(&["symbolic-ref", "--short", "--quiet", "HEAD"], &root);

    // A registered linked worktree at our standard path is authoritative even
    // after a branch switch. For moved worktrees, match the encoded branch to
    // tracked .gitrepo paths instead of trying to decode a lossy ref name.
    if git_dir != common && (path_subdir.is_some() || branch.starts_with("subrepo/")) {
        let porcelain = run_git(&["worktree", "list", "--porcelain", "-z"], &root)?;
        let mut owners = Vec::new();
        for (owner, _) in registered_worktrees(&porcelain) {
            let Ok(owner) = owner.canonicalize() else {
                continue;
            };
            if owner == root || owner.starts_with(&workon_base) {
                continue;
            }
            for subdir in tracked_subrepos(&owner)? {
                let matches = match path_subdir {
                    Some(path) => path == subdir,
                    None => branch == format!("subrepo/{}", encode_subdir(&subdir)),
                };
                if matches {
                    owners.push((owner.clone(), subdir));
                }
            }
        }
        owners.sort();
        owners.dedup();
        if owners.len() > 1 {
            bail!(
                "Multiple checkouts contain this subrepo: {}. Run 'git subrepo commit <subdir> <ref>' from the intended checkout root.",
                owners
                    .iter()
                    .map(|(path, _)| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let Some((owner, subdir)) = owners.pop() else {
            bail!(
                "Cannot identify the owning checkout for this subrepo worktree. Run 'git subrepo commit <subdir> <ref>' from its owning checkout root."
            );
        };
        if explicit.is_some_and(|arg| normalize_subdir(arg) != subdir) {
            bail!(
                "This worktree belongs to subrepo '{subdir}'. Run the command from the intended checkout to import another subrepo."
            );
        }
        let context = Context::at(&owner)?;
        if context.original_head_branch.is_empty() {
            bail!(
                "Owning checkout '{}' has a detached HEAD. Check out the destination branch there first.",
                owner.display()
            );
        }
        return Ok(CommitLocation {
            context,
            subdir,
            source_worktree: Some(root),
        });
    }

    let subdir = match explicit {
        Some(subdir) => normalize_subdir(subdir),
        None => {
            let cwd = cwd.canonicalize()?;
            let relative = cwd.strip_prefix(&root)?;
            nearest_subrepo(relative, &tracked_subrepos(&root)?)
                .ok_or_else(missing)?
                .to_owned()
        }
    };
    Ok(CommitLocation {
        context: Context::at(&root)?,
        subdir,
        source_worktree: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_match_uses_path_components_and_prefers_nested_subrepos() {
        let subrepos = vec![
            "lib/a".to_owned(),
            "lib/a/nested".to_owned(),
            "space dir".to_owned(),
        ];
        assert_eq!(
            nearest_subrepo(Path::new("lib/a/nested/src"), &subrepos),
            Some("lib/a/nested")
        );
        assert_eq!(
            nearest_subrepo(Path::new("lib/a/src"), &subrepos),
            Some("lib/a")
        );
        assert_eq!(
            nearest_subrepo(Path::new("space dir"), &subrepos),
            Some("space dir")
        );
        for path in ["", "lib/ab", "unrelated"] {
            assert_eq!(nearest_subrepo(Path::new(path), &subrepos), None);
        }
        assert_eq!(nearest_subrepo(Path::new("lib/a"), &[]), None);
    }

    #[test]
    fn worktree_records_preserve_spaces_and_newlines() {
        assert_eq!(
            registered_worktrees(
                "worktree /main\0HEAD abc\0branch refs/heads/main\0\0worktree /space dir/line\nbreak\0HEAD def\0detached\0\0"
            ).collect::<Vec<_>>(),
            vec![
                (PathBuf::from("/main"), Some("refs/heads/main")),
                (PathBuf::from("/space dir/line\nbreak"), None)
            ]
        );
        assert!(registered_worktrees("").next().is_none());
    }
}
