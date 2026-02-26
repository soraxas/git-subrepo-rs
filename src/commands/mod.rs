pub mod branch_cmd;
pub mod clean;
pub mod clone;
pub mod commit_cmd;
pub mod config;
pub mod fetch;
pub mod fix;
pub mod init;
pub mod pull;
pub mod push;
pub mod status;
pub mod sync;

use crate::git_utils::{
    branch_exists, commit_in_rev_list, rev_exists, run_git, run_git_interactive, try_run_git,
};
use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Context {
    pub repo_root: PathBuf,
    pub git_common_dir: PathBuf,
    pub git_common_dir_display: String, // raw (possibly relative) for display
    pub original_head_commit: String,
    pub original_head_branch: String,
    pub quiet: bool,
    #[allow(dead_code)]
    pub verbose: bool,
}

impl Context {
    pub fn new() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let (toplevel_ok, repo_root_str) = try_run_git(&["rev-parse", "--show-toplevel"], &cwd);
        if !toplevel_ok {
            anyhow::bail!("Not inside a git repository.");
        }
        let repo_root = PathBuf::from(repo_root_str.trim());

        // Must run from repo top-level directory
        if cwd != repo_root {
            anyhow::bail!("Need to run subrepo command from top level directory of the repo.");
        }

        let git_common_dir_str = run_git(&["rev-parse", "--git-common-dir"], &repo_root)?;
        let git_common_dir_display = git_common_dir_str.trim().to_string();
        let git_common_dir = if git_common_dir_str.starts_with('/') {
            PathBuf::from(git_common_dir_str.trim())
        } else {
            repo_root.join(git_common_dir_str.trim())
        };

        let original_head_branch = {
            let (ok, out) =
                try_run_git(&["symbolic-ref", "--short", "--quiet", "HEAD"], &repo_root);
            if ok { out } else { String::new() }
        };

        let original_head_commit = {
            let (ok, out) = try_run_git(&["rev-parse", "HEAD"], &repo_root);
            if ok { out } else { String::new() }
        };

        Ok(Context {
            repo_root,
            git_common_dir,
            git_common_dir_display,
            original_head_commit,
            original_head_branch,
            quiet: false,
            verbose: false,
        })
    }

    pub fn worktree_path(&self, subdir: &str) -> PathBuf {
        self.git_common_dir.join("tmp").join("subrepo").join(subdir)
    }

    /// Display-friendly worktree path (relative like .git/tmp/subrepo/bar).
    pub fn worktree_display(&self, subdir: &str) -> String {
        format!("{}/tmp/subrepo/{}", self.git_common_dir_display, subdir)
    }
}

/// Normalize a subdir path (remove leading ./, trailing /, collapse //).
pub fn normalize_subdir(subdir: &str) -> String {
    let mut s = subdir
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    s
}

/// Open the user's editor with `msg` pre-filled; return the edited text.
/// Honours GIT_EDITOR, VISUAL, EDITOR (in that order), falling back to `vi`.
pub fn edit_message_in_editor(msg: &str) -> Result<String> {
    let tmp = std::env::temp_dir().join(format!("git-subrepo-msg-{}", std::process::id()));
    std::fs::write(&tmp, msg)?;
    let editor = std::env::var("GIT_EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let shell_cmd = format!("{} {}", editor, tmp.display());
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&shell_cmd)
        .status()?;
    if !status.success() {
        anyhow::bail!("Editor exited with non-zero status");
    }
    let result = std::fs::read_to_string(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    Ok(result)
}

/// Assert that the working copy is clean.
/// If `subdir` is Some, only checks for changes within that subtree (used by `branch`).
pub fn assert_clean_for(cmd: &str, ctx: &Context) -> Result<()> {
    assert_clean_for_subdir(cmd, ctx, None)
}

pub fn assert_clean_for_subdir(cmd: &str, ctx: &Context, subdir: Option<&str>) -> Result<()> {
    run_git(
        &["update-index", "-q", "--ignore-submodules", "--refresh"],
        &ctx.repo_root,
    )?;

    // Build path-limiter args
    let path_args: Vec<&str> = if let Some(s) = subdir {
        vec!["--", s]
    } else {
        vec![]
    };

    let mut diff_files_args = vec!["diff-files", "--quiet", "--ignore-submodules"];
    diff_files_args.extend_from_slice(&path_args);
    let (ok1, _) = try_run_git(&diff_files_args, &ctx.repo_root);
    if !ok1 {
        // Allow through if the only dirty files are .gitrepo files
        // (e.g. after `git subrepo fix` updated parent= without staging).
        let (_, dirty_out) = try_run_git(
            &["diff-files", "--name-only", "--ignore-submodules"],
            &ctx.repo_root,
        );
        let non_gitrepo_dirty = dirty_out
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.ends_with("/.gitrepo"))
            .count();
        if non_gitrepo_dirty > 0 {
            anyhow::bail!("Can't {cmd} subrepo. Unstaged changes.");
        }
    }

    if !ctx.original_head_commit.is_empty() {
        let mut diff_index_args = vec!["diff-index", "--quiet", "--ignore-submodules", "HEAD"];
        diff_index_args.extend_from_slice(&path_args);
        let (ok2, _) = try_run_git(&diff_index_args, &ctx.repo_root);
        if !ok2 {
            anyhow::bail!("Can't {cmd} subrepo. Working tree has changes.");
        }

        let mut diff_cached_args = vec![
            "diff-index",
            "--quiet",
            "--cached",
            "--ignore-submodules",
            "HEAD",
        ];
        diff_cached_args.extend_from_slice(&path_args);
        let (ok3, _) = try_run_git(&diff_cached_args, &ctx.repo_root);
        if !ok3 {
            anyhow::bail!("Can't {cmd} subrepo. Index has changes.");
        }
    }

    Ok(())
}

/// Like `assert_clean_for` but allows staged changes that are entirely within other subrepo
/// directories (identified by a co-staged or on-disk `.gitrepo` file).
///
/// This enables the workflow of chaining multiple `--stage-only` clones/pulls before
/// making a single batch commit, while still blocking arbitrary staged changes.
pub fn assert_clean_for_clone(ctx: &Context, target_subdir: &str) -> Result<()> {
    run_git(
        &["update-index", "-q", "--ignore-submodules", "--refresh"],
        &ctx.repo_root,
    )?;

    // Always block unstaged modifications to tracked files globally.
    let (ok_files, _) = try_run_git(
        &["diff-files", "--quiet", "--ignore-submodules"],
        &ctx.repo_root,
    );
    if !ok_files {
        anyhow::bail!("Can't clone subrepo. Unstaged changes.");
    }

    if ctx.original_head_commit.is_empty() {
        return Ok(());
    }

    // Block any changes (staged or unstaged) specifically in the target subdir.
    let (ok_target, _) = try_run_git(
        &[
            "diff-index",
            "--quiet",
            "--ignore-submodules",
            "HEAD",
            "--",
            target_subdir,
        ],
        &ctx.repo_root,
    );
    if !ok_target {
        anyhow::bail!("Can't clone subrepo. Working tree has changes.");
    }

    // Inspect staged files outside the target subdir.
    let (ok_list, staged_out) = try_run_git(
        &["diff-index", "--cached", "--name-only", "HEAD"],
        &ctx.repo_root,
    );
    if !ok_list || staged_out.trim().is_empty() {
        return Ok(());
    }

    // Collect the set of subrepo-root dirs: any dir that has a .gitrepo staged
    // OR already has a .gitrepo on disk.
    let staged_files: Vec<&str> = staged_out.lines().filter(|l| !l.is_empty()).collect();
    let mut subrepo_roots: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &staged_files {
        if f.ends_with("/.gitrepo") {
            subrepo_roots.insert(f.trim_end_matches("/.gitrepo").to_string());
        }
    }
    // Also scan the working tree for any known .gitrepo files in the index.
    let (ok_ls, ls_out) = try_run_git(&["ls-files", "--", "*.gitrepo"], &ctx.repo_root);
    if ok_ls {
        for f in ls_out.lines().filter(|l| l.ends_with("/.gitrepo")) {
            subrepo_roots.insert(f.trim_end_matches("/.gitrepo").to_string());
        }
    }

    // If every staged file lives under a known subrepo root, it's fine to proceed.
    for f in &staged_files {
        let in_subrepo = subrepo_roots
            .iter()
            .any(|root| f.starts_with(&format!("{root}/")));
        if !in_subrepo {
            anyhow::bail!("Can't clone subrepo. Working tree has changes.");
        }
    }

    Ok(())
}

/// Assert that the working copy is clean (uses "run" as command name for backwards compat).
#[allow(dead_code)]
pub fn assert_clean(ctx: &Context) -> Result<()> {
    assert_clean_for("run", ctx)
}

/// Build the commit message for a subrepo operation.
#[allow(clippy::too_many_arguments)]
pub fn build_commit_message(
    command: &str,
    subdir: &str,
    subrepo_commit_ref: &str,
    remote: &str,
    branch: &str,
    upstream_head_commit: &str,
    repo_root: &std::path::Path,
) -> String {
    let merged = if !subrepo_commit_ref.is_empty() && rev_exists(subrepo_commit_ref, repo_root) {
        let (ok, out) = try_run_git(&["rev-parse", "--short", subrepo_commit_ref], repo_root);
        if ok { out } else { "none".to_string() }
    } else {
        "none".to_string()
    };

    let commit = if !upstream_head_commit.is_empty() && rev_exists(upstream_head_commit, repo_root)
    {
        let (ok, out) = try_run_git(&["rev-parse", "--short", upstream_head_commit], repo_root);
        if ok { out } else { "none".to_string() }
    } else {
        "none".to_string()
    };

    // Add (merge) suffix when the commit ref is a merge commit (for non-push commands)
    let is_merge = if command != "push" && !subrepo_commit_ref.is_empty() {
        let (_, show_out) = try_run_git(&["show", "--summary", subrepo_commit_ref], repo_root);
        show_out.lines().any(|l| l.starts_with("Merge:"))
    } else {
        false
    };
    let merge_suffix = if is_merge { " (merge)" } else { "" };

    format!(
        "git subrepo {command}{merge_suffix} {subdir}\n\nsubrepo:\n  subdir:   \"{subdir}\"\n  merged:   \"{merged}\"\nupstream:\n  origin:   \"{remote}\"\n  branch:   \"{branch}\"\n  commit:   \"{commit}\"\ngit-subrepo:\n  version:  \"{VERSION}\"\n"
    )
}

/// Perform the subrepo:fetch operation.
/// Returns the upstream HEAD commit SHA.
pub fn subrepo_fetch(ctx: &Context, remote: &str, branch: &str, subref: &str) -> Result<String> {
    subrepo_fetch_with_pb(ctx, remote, branch, subref, None)
}

/// Like `subrepo_fetch` but reuses an existing `ProgressBar` from the caller (e.g. `--all` loop).
pub fn subrepo_fetch_with_pb(
    ctx: &Context,
    remote: &str,
    branch: &str,
    subref: &str,
    caller_pb: Option<&indicatif::ProgressBar>,
) -> Result<String> {
    use indicatif::{ProgressBar, ProgressStyle};

    // If the caller already owns a spinner, update its message instead of creating a new one.
    let owned_pb = if let Some(pb) = caller_pb {
        pb.set_message(format!("Fetching {remote} ({branch})..."));
        None
    } else if !ctx.quiet {
        let p = ProgressBar::new_spinner();
        p.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        p.set_message(format!("Fetching {remote} ({branch})..."));
        p.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(p)
    } else {
        None
    };

    // Fetch directly into the per-subrepo ref to avoid FETCH_HEAD race in parallel fetches.
    let fetch_ref = format!("refs/subrepo/{subref}/fetch");
    let refspec = format!("+{branch}:{fetch_ref}");
    let fetch_result = run_git(
        &["fetch", "--no-tags", "--quiet", remote, &refspec],
        &ctx.repo_root,
    );
    // Only finish/clear if we own the bar (not the caller's bar)
    if let Some(p) = owned_pb {
        p.finish_and_clear();
    }
    fetch_result?;

    let upstream_head = run_git(&["rev-parse", &fetch_ref], &ctx.repo_root)?;

    Ok(upstream_head)
}

/// Perform the subrepo:branch operation.
/// Returns the worktree path.
pub fn subrepo_branch(
    ctx: &Context,
    subdir: &str,
    subref: &str,
    subrepo_parent: &str,
    join_method: &str,
    force: bool,
) -> Result<PathBuf> {
    let branch_name = format!("subrepo/{subref}");

    if branch_exists(&branch_name, &ctx.repo_root) {
        if force {
            delete_branch_and_worktree(ctx, subdir, subref)?;
        } else {
            return Err(anyhow::anyhow!(
                "Branch '{branch_name}' already exists. Use '--force' to override."
            ));
        }
    }

    if subrepo_parent.is_empty() {
        // No parent: use subdirectory filter
        subrepo_branch_no_parent(ctx, subdir, subref, &branch_name)?;
    } else {
        // Has parent: build commit chain
        match subrepo_branch_with_parent(
            ctx,
            subdir,
            subref,
            subrepo_parent,
            join_method,
            &branch_name,
            force,
        ) {
            Ok(()) => {}
            Err(e) if e.to_string() == "no_commits" => {
                // parent == HEAD: no local subrepo commits to replay.
                // Create the branch pointing directly at the upstream fetch ref so
                // the merge/rebase step in pull can proceed normally.
                let fetch_ref = format!("refs/subrepo/{subref}/fetch");
                let (ok, fetch_sha) = try_run_git(&["rev-parse", &fetch_ref], &ctx.repo_root);
                if ok && !fetch_sha.trim().is_empty() {
                    crate::git_utils::run_git(
                        &["branch", &branch_name, fetch_sha.trim()],
                        &ctx.repo_root,
                    )?;
                } else {
                    // fetch ref not available yet — fall back to no_parent path
                    subrepo_branch_no_parent(ctx, subdir, subref, &branch_name)?;
                }
            }
            Err(e) => return Err(e),
        }
    }

    let worktree = ctx.worktree_path(subdir);
    let (ok, err_out) = try_run_git(
        &["worktree", "add", &worktree.to_string_lossy(), &branch_name],
        &ctx.repo_root,
    );
    if !ok {
        anyhow::bail!(
            "Could not create worktree for '{subdir}':\n  {}\n\n\
             If a stale worktree remains, run:  git subrepo clean {subdir}",
            err_out.trim().replace('\n', "\n  ")
        );
    }

    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/branch"),
            &branch_name,
        ],
        &ctx.repo_root,
    )?;

    Ok(worktree)
}

fn subrepo_branch_no_parent(
    ctx: &Context,
    subdir: &str,
    _subref: &str,
    branch_name: &str,
) -> Result<()> {
    run_git(&["branch", branch_name, "HEAD"], &ctx.repo_root)?;

    // Filter to subdirectory content
    crate::git_utils::try_run_git_env(
        &[
            "filter-branch",
            "-f",
            "--subdirectory-filter",
            subdir,
            branch_name,
        ],
        &ctx.repo_root,
        &[("FILTER_BRANCH_SQUELCH_WARNING", "1")],
    );

    // Remove .gitrepo from commits
    crate::git_utils::try_run_git_env(
        &[
            "filter-branch",
            "-f",
            "--prune-empty",
            "--tree-filter",
            "rm -f .gitrepo",
            "--",
            branch_name,
            "--first-parent",
        ],
        &ctx.repo_root,
        &[("FILTER_BRANCH_SQUELCH_WARNING", "1")],
    );

    Ok(())
}

#[derive(Debug)]
enum ParentCleanness {
    /// Subdir tree objects are identical (compared against `reference`).
    Identical { reference: String },
    /// Only .gitrepo differs (compared against `reference`).
    OnlyGitrepo { reference: String },
    /// Real files differ — list of changed paths.
    Modified {
        reference: String,
        files: Vec<String>,
    },
    /// Could not determine (object unavailable etc.).
    Unknown,
}

/// Compare `<candidate>:<subdir>` tree against `<ref_commit>:<subdir>` tree.
/// If `stored_parent` is unavailable locally (rebased away), falls back to HEAD.
fn check_parent_cleanliness(
    ctx: &Context,
    subdir: &str,
    stored_parent: &str,
    candidate: &str,
) -> ParentCleanness {
    // Determine reference: prefer stored_parent, fall back to HEAD.
    let (ref_commit, ref_label) = {
        let (ok, _) = try_run_git(&["rev-parse", "--verify", stored_parent], &ctx.repo_root);
        if ok {
            (stored_parent.to_string(), "stored parent".to_string())
        } else {
            (
                "HEAD".to_string(),
                "HEAD (stored parent no longer exists locally)".to_string(),
            )
        }
    };

    let (ok_a, tree_a) = try_run_git(
        &["rev-parse", &format!("{ref_commit}:{subdir}")],
        &ctx.repo_root,
    );
    let (ok_b, tree_b) = try_run_git(
        &["rev-parse", &format!("{candidate}:{subdir}")],
        &ctx.repo_root,
    );
    if !ok_a || !ok_b {
        return ParentCleanness::Unknown;
    }
    let tree_a = tree_a.trim();
    let tree_b = tree_b.trim();

    if tree_a == tree_b {
        return ParentCleanness::Identical {
            reference: ref_label,
        };
    }

    let (ok, diff) = try_run_git(
        &[
            "diff-tree",
            "--no-commit-id",
            "-r",
            "--name-only",
            tree_a,
            tree_b,
        ],
        &ctx.repo_root,
    );
    if !ok {
        return ParentCleanness::Unknown;
    }

    let changed: Vec<String> = diff
        .lines()
        .filter(|l| !l.trim().is_empty() && l.trim() != ".gitrepo")
        .map(|l| l.to_string())
        .collect();

    if changed.is_empty() {
        ParentCleanness::OnlyGitrepo {
            reference: ref_label,
        }
    } else {
        ParentCleanness::Modified {
            reference: ref_label,
            files: changed,
        }
    }
}

/// Given a subdir path, return the outer subrepo that contains it (if any).
/// e.g. `task-engine/datamodel` → `Some("task-engine")` if `task-engine/.gitrepo` exists.
/// The parent commit in a nested subrepo's .gitrepo refers to a commit in the *outer*
/// subrepo's upstream history, not the main repo's HEAD.
pub(super) fn find_outer_subrepo(ctx: &Context, subdir: &str) -> Option<String> {
    let mut path = subdir;
    while let Some(pos) = path.rfind('/') {
        path = &path[..pos];
        let candidate = ctx.repo_root.join(path).join(".gitrepo");
        if candidate.exists() {
            return Some(path.to_string());
        }
    }
    None
}

/// Return the ref to use as the "history root" for parent-ancestry checks.
/// For a nested subrepo inside `outer`, use `refs/subrepo/<outer>/branch`.
/// For a top-level subrepo (or if the outer ref doesn't exist), use `HEAD`.
pub(super) fn parent_check_ref(ctx: &Context, subdir: &str) -> String {
    if let Some(outer) = find_outer_subrepo(ctx, subdir) {
        let branch_ref = format!("refs/subrepo/{outer}/branch");
        let (ok, _) = try_run_git(&["rev-parse", "--verify", &branch_ref], &ctx.repo_root);
        if ok {
            return branch_ref;
        }
        // outer branch ref doesn't exist locally — fall back to HEAD
    }
    "HEAD".to_string()
}

/// `commit =` line in `<subdir>/.gitrepo`.  That commit is the rebased subrepo
/// pull commit; its parent is the rebased equivalent of the stored parent and
/// is guaranteed to be reachable from the history root (`check_ref`).
///
/// For nested subrepos, `check_ref` is `refs/subrepo/<outer>/branch`; for
/// top-level subrepos it is `HEAD`.
pub(super) fn find_new_parent_after_rebase(
    ctx: &Context,
    subdir: &str,
    _stored_parent: &str,
    check_ref: &str,
) -> Option<String> {
    // --- Primary: find the most-recent commit in check_ref history that
    // changed `commit =` in the subdir's .gitrepo ---
    let gitrepo_rel = format!("{subdir}/.gitrepo");
    let (ok, pull_commit) = try_run_git(
        &[
            "log",
            check_ref,
            "-1",
            "-G",
            "parent =",
            "--format=%H",
            "--",
            &gitrepo_rel,
        ],
        &ctx.repo_root,
    );
    if ok && !pull_commit.trim().is_empty() {
        let pull_commit = pull_commit.trim();
        let (ok2, parent) = try_run_git(&["rev-parse", &format!("{pull_commit}^")], &ctx.repo_root);
        if ok2 && !parent.trim().is_empty() {
            let parent = parent.trim().to_string();
            // Sanity check: the candidate should have the subdir already present.
            let (has_subdir, _) = try_run_git(
                &["rev-parse", "--verify", &format!("{parent}:{subdir}")],
                &ctx.repo_root,
            );
            if has_subdir {
                return Some(parent);
            }
        }
    }

    // --- Fallback: content-walk in check_ref history ---
    let (ok, log) = try_run_git(
        &[
            "log",
            check_ref,
            "-1",
            "--format=%H",
            "--",
            subdir,
            &format!(":(exclude){subdir}/.gitrepo"),
        ],
        &ctx.repo_root,
    );
    if ok && !log.trim().is_empty() {
        let content_change_commit = log.trim();
        let (ok2, parent) = try_run_git(
            &["rev-parse", &format!("{content_change_commit}^")],
            &ctx.repo_root,
        );
        if ok2 && !parent.trim().is_empty() {
            let parent = parent.trim().to_string();
            let (has_subdir, _) = try_run_git(
                &["rev-parse", "--verify", &format!("{parent}:{subdir}")],
                &ctx.repo_root,
            );
            if has_subdir {
                return Some(parent);
            }
        }
    }

    // --- Final fallback: use tip of check_ref ---
    let (ok, tip) = try_run_git(&["rev-parse", check_ref], &ctx.repo_root);
    if ok && !tip.trim().is_empty() {
        return Some(tip.trim().to_string());
    }

    None
}

fn subrepo_branch_with_parent(
    ctx: &Context,
    subdir: &str,
    subref: &str,
    subrepo_parent: &str,
    join_method: &str,
    branch_name: &str,
    force: bool,
) -> Result<()> {
    // For nested subrepos, parent= refers to a commit in the outer subrepo's upstream,
    // not the main repo's HEAD.  Use the appropriate ref for ancestry checks.
    let check_ref = parent_check_ref(ctx, subdir);
    let is_nested = check_ref != "HEAD";

    let (is_ancestor, _) = try_run_git(
        &["merge-base", "--is-ancestor", subrepo_parent, &check_ref],
        &ctx.repo_root,
    );

    if !is_ancestor {
        // Parent is not an ancestor — likely caused by a rebase.
        let new_parent = find_new_parent_after_rebase(ctx, subdir, subrepo_parent, &check_ref);

        let stored_short = &subrepo_parent[..subrepo_parent.len().min(7)];

        let history_label = if is_nested {
            format!("outer subrepo history ({})", check_ref)
        } else {
            "HEAD history".to_string()
        };

        eprintln!(
            "{}: '{}' parent {} is not in {} (caused by a rebase).",
            "git-subrepo".yellow().bold(),
            subdir,
            stored_short,
            history_label
        );

        match new_parent {
            Some(ref candidate) => {
                let candidate_short = &candidate[..candidate.len().min(7)];

                // How far is the candidate from check_ref?
                let (_, ahead_out) = try_run_git(
                    &["rev-list", "--count", &format!("{candidate}..{check_ref}")],
                    &ctx.repo_root,
                );
                let commits_ahead: usize = ahead_out.trim().parse().unwrap_or(0);

                // How many subdir files differ between candidate and HEAD (excl .gitrepo)?
                let (_, diff_out) = try_run_git(
                    &[
                        "diff",
                        "--name-only",
                        &format!("{candidate}:{subdir}"),
                        &format!("{check_ref}:{subdir}"),
                    ],
                    &ctx.repo_root,
                );
                let changed_files: Vec<&str> = diff_out
                    .lines()
                    .filter(|l| !l.trim().is_empty() && l.trim() != ".gitrepo")
                    .collect();

                // Validate: compare candidate's subdir tree with stored parent's subdir tree.
                let parent_clean = check_parent_cleanliness(ctx, subdir, subrepo_parent, candidate);

                let ref_short = check_ref
                    .strip_prefix("refs/subrepo/")
                    .unwrap_or(&check_ref);
                let commits_str = if commits_ahead == 0 {
                    format!("📍 {} behind {}", "0 commits".green().bold(), ref_short)
                } else {
                    format!(
                        "📍 {} behind {}",
                        format!("{commits_ahead} commits").yellow().bold(),
                        ref_short
                    )
                };
                let files_str = if changed_files.is_empty() {
                    format!("📂 {}", "0 files differ".green().bold())
                } else {
                    format!(
                        "📂 {}",
                        format!("{} files differ", changed_files.len())
                            .yellow()
                            .bold()
                    )
                };
                eprintln!(
                    "  Found likely new parent: {}  ({},  {})",
                    candidate_short.green().bold(),
                    commits_str,
                    files_str,
                );
                if !changed_files.is_empty() {
                    for f in changed_files.iter().take(5) {
                        eprintln!("      {}", f.dimmed());
                    }
                    if changed_files.len() > 5 {
                        eprintln!("      … and {} more", changed_files.len() - 5);
                    }
                }
                match &parent_clean {
                    ParentCleanness::Identical { reference } => {
                        eprintln!("  {}", format!("✓ Likely clean — subdir content is identical to {reference} (safe to auto-repair)").green());
                    }
                    ParentCleanness::OnlyGitrepo { reference } => {
                        eprintln!("  {}", format!("✓ Likely clean — only .gitrepo differs vs {reference} (safe to auto-repair)").green());
                    }
                    ParentCleanness::Modified { reference, files } => {
                        eprintln!(
                            "  {} — {} file(s) differ vs {}:",
                            "⚠ Content likely differs".yellow().bold(),
                            files.len(),
                            reference
                        );
                        for f in files.iter().take(10) {
                            eprintln!("      {}", f.dimmed());
                        }
                        if files.len() > 10 {
                            eprintln!("      … and {} more", files.len() - 10);
                        }
                    }
                    ParentCleanness::Unknown => {
                        eprintln!(
                            "  {}",
                            "(could not resolve subdir trees for comparison)".dimmed()
                        );
                    }
                }
                eprintln!(
                    "  Repair hint: git subrepo config {} parent {}",
                    subdir, candidate_short
                );
                eprintln!();

                // Try interactive prompt — gracefully skip if no TTY.
                use dialoguer::Select;
                let force_label = if is_nested {
                    format!(
                        "Force: reset parent to tip of {} (discards local divergence)",
                        ref_short
                    )
                } else {
                    "Force: reset parent to HEAD (discards local divergence)".to_string()
                };
                let choices = &[
                    format!("Auto-repair: use {} as new parent", candidate_short),
                    force_label,
                    "Abort".to_string(),
                ];
                let selection = Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
                    .with_prompt("How to proceed?")
                    .items(choices)
                    .default(0)
                    .interact_opt();

                match selection {
                    Ok(Some(0)) => {
                        // Repair: update .gitrepo with the discovered parent and continue
                        let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");
                        crate::git_utils::run_git(
                            &[
                                "config",
                                "--file",
                                &gitrepo_path.to_string_lossy(),
                                "subrepo.parent",
                                candidate,
                            ],
                            &ctx.repo_root,
                        )?;
                        return subrepo_branch_with_parent(
                            ctx,
                            subdir,
                            subref,
                            candidate,
                            join_method,
                            branch_name,
                            force,
                        );
                    }
                    Ok(Some(1)) => {
                        // Force: treat tip of check_ref as the parent
                        let (_, tip) = try_run_git(&["rev-parse", &check_ref], &ctx.repo_root);
                        let tip = tip.trim().to_string();
                        return subrepo_branch_with_parent(
                            ctx,
                            subdir,
                            subref,
                            &tip,
                            join_method,
                            branch_name,
                            force,
                        );
                    }
                    _ => {
                        // Abort or no TTY — bail with repair hint
                        anyhow::bail!(
                            "Aborted. To repair: git subrepo config {} parent {}",
                            subdir,
                            candidate_short
                        );
                    }
                }
            }
            None => {
                anyhow::bail!(
                    "No matching commit found. Use `--force` to treat {} as the new parent.",
                    if is_nested { &check_ref } else { "HEAD" }
                );
            }
        }
    }

    // Get rev-list from subrepo_parent..HEAD
    let range = format!("{subrepo_parent}..HEAD");
    let (ok, commit_list_str) = try_run_git(
        &[
            "rev-list",
            "--reverse",
            "--ancestry-path",
            "--topo-order",
            &range,
        ],
        &ctx.repo_root,
    );

    if !ok || commit_list_str.is_empty() {
        // No commits to process
        anyhow::bail!("no_commits");
    }

    let commit_list: Vec<&str> = commit_list_str.lines().collect();

    let mut prev_commit: Option<String> = None;
    let mut ancestor: Option<String> = None;
    let mut first_gitrepo_commit: Option<String> = None;
    let mut last_gitrepo_commit = String::new();

    let refs_subrepo_fetch = format!("refs/subrepo/{subref}/fetch");

    for commit in &commit_list {
        // Try to get .gitrepo subrepo.commit from this commit
        let gitrepo_commit_key = format!("{commit}:{subdir}/.gitrepo");
        let (ok, gitrepo_commit) = try_run_git(
            &["config", "--blob", &gitrepo_commit_key, "subrepo.commit"],
            &ctx.repo_root,
        );

        if !ok || gitrepo_commit.is_empty() {
            continue;
        }

        // Check that gitrepo_commit is reachable from the fetch ref (skipped with --force)
        if !force
            && rev_exists(&refs_subrepo_fetch, &ctx.repo_root)
            && !commit_in_rev_list(&gitrepo_commit, &refs_subrepo_fetch, &ctx.repo_root)
        {
            anyhow::bail!(
                "Local repository does not contain {}. Try to 'git subrepo fetch {}' or add the '-F' flag to always fetch the latest content.",
                gitrepo_commit,
                subdir
            );
        }

        // If ancestor is set, check if it's a direct parent
        if let Some(ref and) = ancestor {
            let (_, parents_str) = try_run_git(
                &["show", "-s", "--pretty=format:%P", commit],
                &ctx.repo_root,
            );
            if !parents_str.split_whitespace().any(|p| p == and.as_str()) {
                continue;
            }
        }

        ancestor = Some(commit.to_string());

        // Build parent args
        let mut parent_args: Vec<String> = Vec::new();
        if let Some(ref pc) = prev_commit {
            parent_args.push("-p".to_string());
            parent_args.push(pc.clone());
        }

        if first_gitrepo_commit.is_none() {
            first_gitrepo_commit = Some(gitrepo_commit.clone());
            parent_args.push("-p".to_string());
            parent_args.push(gitrepo_commit.clone());
        } else if join_method != "rebase" && gitrepo_commit != last_gitrepo_commit {
            parent_args.push("-p".to_string());
            parent_args.push(gitrepo_commit.clone());
        }
        last_gitrepo_commit = gitrepo_commit.clone();

        // Get author info
        let (_, author_info_str) = try_run_git(
            &[
                "log",
                "-1",
                "--date=default",
                "--format=%ad%n%ae%n%an",
                commit,
            ],
            &ctx.repo_root,
        );
        let author_lines: Vec<&str> = author_info_str.lines().collect();
        let author_date = author_lines.first().copied().unwrap_or("");
        let author_email = author_lines.get(1).copied().unwrap_or("");
        let author_name = author_lines.get(2).copied().unwrap_or("");

        // Check if commit has content in subdir
        let subdir_ref = format!("{commit}:{subdir}");
        let (has_content, _) = try_run_git(&["cat-file", "-e", &subdir_ref], &ctx.repo_root);

        let new_commit = if has_content {
            // Get commit message
            let (_, commit_msg) = try_run_git(
                &["log", "-n", "1", "--date=default", "--format=%B", commit],
                &ctx.repo_root,
            );

            // Build commit-tree args
            let mut args: Vec<&str> = vec!["commit-tree", "-F", "-"];
            let parent_refs: Vec<&str> = parent_args.iter().map(|s| s.as_str()).collect();
            args.extend_from_slice(&parent_refs);
            let tree_ref = format!("{commit}:{subdir}");
            args.push(&tree_ref);

            // Run commit-tree with author env and stdin
            let mut cmd = std::process::Command::new("git");
            cmd.args(&args)
                .current_dir(&ctx.repo_root)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_AUTHOR_DATE", author_date)
                .env("GIT_AUTHOR_EMAIL", author_email)
                .env("GIT_AUTHOR_NAME", author_name)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            let mut child = cmd.spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                stdin.write_all(commit_msg.as_bytes())?;
            }
            let output = child.wait_with_output()?;
            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("commit-tree failed: {err}");
            }
            String::from_utf8_lossy(&output.stdout)
                .trim_end_matches('\n')
                .to_string()
        } else {
            // Empty commit
            let empty_tree = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
            let mut args: Vec<&str> = vec!["commit-tree", "-m", "EMPTY"];
            let parent_refs: Vec<&str> = parent_args.iter().map(|s| s.as_str()).collect();
            args.extend_from_slice(&parent_refs);
            args.push(empty_tree);
            run_git(&args, &ctx.repo_root)?
        };

        prev_commit = Some(new_commit);
    }

    let prev = match prev_commit {
        Some(pc) => pc,
        None => anyhow::bail!("no_commits"),
    };

    // Create branch
    run_git(&["branch", branch_name, &prev], &ctx.repo_root)?;

    // Remove .gitrepo from the branch
    let filter = match &first_gitrepo_commit {
        Some(fgc) => format!("{fgc}..{branch_name}"),
        None => branch_name.to_string(),
    };

    crate::git_utils::try_run_git_env(
        &[
            "filter-branch",
            "-f",
            "--prune-empty",
            "--tree-filter",
            "rm -f .gitrepo",
            "--",
            &filter,
            "--first-parent",
        ],
        &ctx.repo_root,
        &[("FILTER_BRANCH_SQUELCH_WARNING", "1")],
    );

    Ok(())
}

/// Delete a subrepo branch and its worktree.
pub fn delete_branch_and_worktree(ctx: &Context, subdir: &str, subref: &str) -> Result<()> {
    let branch_name = format!("subrepo/{subref}");
    let worktree = ctx.worktree_path(subdir);
    let worktree_str = worktree.to_string_lossy().to_string();

    // Step 1: Try `git worktree remove --force` by path.
    try_run_git(
        &["worktree", "remove", "--force", &worktree_str],
        &ctx.repo_root,
    );

    // Step 2: Scan `git worktree list --porcelain` for any entry on our branch and remove it.
    // This handles cases where the registered path differs from what we compute.
    {
        let (ok, out) = try_run_git(&["worktree", "list", "--porcelain"], &ctx.repo_root);
        if ok {
            let mut current_path: Option<String> = None;
            let mut current_branch: Option<String> = None;
            for line in out.lines() {
                if let Some(path) = line.strip_prefix("worktree ") {
                    current_path = Some(path.to_string());
                    current_branch = None;
                } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                    current_branch = Some(b.to_string());
                } else if line.is_empty() {
                    if current_branch.as_deref() == Some(&branch_name)
                        && let Some(ref p) = current_path
                    {
                        try_run_git(&["worktree", "remove", "--force", p], &ctx.repo_root);
                        let _ = std::fs::remove_dir_all(p);
                    }
                    current_path = None;
                    current_branch = None;
                }
            }
            // Handle last stanza (no trailing blank line)
            if current_branch.as_deref() == Some(&branch_name)
                && let Some(ref p) = current_path
            {
                try_run_git(&["worktree", "remove", "--force", p], &ctx.repo_root);
                let _ = std::fs::remove_dir_all(p);
            }
        }
    }

    // Step 3: Remove the actual directory if still present.
    if worktree.exists() {
        let _ = std::fs::remove_dir_all(&worktree);
    }

    // Step 4: Blast any lock files inside .git/worktrees/*/  that reference our branch,
    // so that `worktree prune` can clean them up (prune skips locked entries).
    let git_worktrees_meta = ctx.repo_root.join(".git").join("worktrees");
    if git_worktrees_meta.is_dir()
        && let Ok(entries) = std::fs::read_dir(&git_worktrees_meta)
    {
        for entry in entries.flatten() {
            let head_file = entry.path().join("HEAD");
            if let Ok(contents) = std::fs::read_to_string(&head_file) {
                let expected = format!("ref: refs/heads/{branch_name}");
                if contents.trim() == expected || contents.trim() == branch_name {
                    // Remove the lock file so prune can remove this stanza.
                    let _ = std::fs::remove_file(entry.path().join("locked"));
                    // Also remove the whole metadata dir outright.
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
    }

    // Step 5: Prune any remaining stale registrations.
    try_run_git(&["worktree", "prune"], &ctx.repo_root);

    // Step 6: Force-delete the branch ref (update-ref -d works even if git thinks it's
    // checked out, unlike `git branch -D`).
    if branch_exists(&branch_name, &ctx.repo_root) {
        let branch_ref = format!("refs/heads/{branch_name}");
        run_git(&["update-ref", "-d", &branch_ref], &ctx.repo_root)?;
    }

    Ok(())
}

/// Perform the subrepo:commit operation.
#[allow(clippy::too_many_arguments)]
pub fn subrepo_commit(
    ctx: &Context,
    subdir: &str,
    subref: &str,
    subrepo_commit_ref: &str,
    remote: &str,
    branch: &str,
    upstream_head_commit: &str,
    join_method: &str,
    force: bool,
    message: Option<&str>,
    verify: bool,
) -> Result<()> {
    // Check that subrepo_commit_ref exists
    if !rev_exists(subrepo_commit_ref, &ctx.repo_root) {
        anyhow::bail!("Commit ref '{subrepo_commit_ref}' does not exist.");
    }

    // Unless force: check upstream_head is in rev-list of subrepo_commit_ref
    if !force
        && !upstream_head_commit.is_empty()
        && !crate::git_utils::commit_in_rev_list(
            upstream_head_commit,
            subrepo_commit_ref,
            &ctx.repo_root,
        )
    {
        anyhow::bail!("Can't commit: '{subrepo_commit_ref}' doesn't contain upstream HEAD.");
    }

    // Remove existing subdir from index
    let (_, ls_out) = try_run_git(&["ls-files", "--", subdir], &ctx.repo_root);
    if !ls_out.trim().is_empty() {
        run_git(&["rm", "-r", "--", subdir], &ctx.repo_root)?;
    }

    // Read in the subrepo content
    let prefix = format!("{subdir}/");
    run_git(
        &["read-tree", "--prefix", &prefix, "-u", subrepo_commit_ref],
        &ctx.repo_root,
    )?;

    // Write .gitrepo file
    let gitrepo_path = ctx.repo_root.join(subdir).join(".gitrepo");
    let gitrepo_path_str = gitrepo_path.to_string_lossy().into_owned();
    let gitrepo_rel = format!("{subdir}/.gitrepo");

    // Determine if parent should be written
    // Parent is written when upstream_head_commit == git rev-parse(subrepo_commit_ref)
    let resolved_ref = crate::git_utils::rev_parse(subrepo_commit_ref, &ctx.repo_root);
    let parent = if !upstream_head_commit.is_empty() && !subrepo_commit_ref.is_empty() {
        if resolved_ref.as_deref() == Some(upstream_head_commit) {
            Some(ctx.original_head_commit.as_str())
        } else {
            None
        }
    } else {
        None
    };

    if gitrepo_path.exists() {
        crate::gitrepo::update_gitrepo(
            &gitrepo_path,
            None, // don't update remote
            None, // don't update branch
            upstream_head_commit,
            parent,
            join_method,
            VERSION,
            &ctx.repo_root,
        )?;
    } else {
        // Try to restore .gitrepo from original_head_commit (bash behavior)
        let (cat_ok, cat_out) = try_run_git(
            &[
                "cat-file",
                "-p",
                &format!("{}:{}", ctx.original_head_commit, gitrepo_rel),
            ],
            &ctx.repo_root,
        );
        if cat_ok && !cat_out.is_empty() {
            std::fs::write(&gitrepo_path, cat_out + "\n")?;
            crate::gitrepo::update_gitrepo(
                &gitrepo_path,
                None,
                None,
                upstream_head_commit,
                parent,
                join_method,
                VERSION,
                &ctx.repo_root,
            )?;
        } else {
            crate::gitrepo::write_new_gitrepo(
                &gitrepo_path,
                remote,
                branch,
                upstream_head_commit,
                parent,
                join_method,
                VERSION,
                &ctx.repo_root,
            )?;
        }
    }

    run_git(&["add", "-f", "--", &gitrepo_path_str], &ctx.repo_root)?;

    let commit_msg = match message {
        Some(m) => m.to_string(),
        None => build_commit_message(
            // extract command from subrepo_commit_ref to guess - use generic
            "pull",
            subdir,
            subrepo_commit_ref,
            remote,
            branch,
            upstream_head_commit,
            &ctx.repo_root,
        ),
    };

    let mut commit_args = vec!["commit"];
    if !verify {
        commit_args.push("--no-verify");
    }
    commit_args.extend(["-m", &commit_msg]);
    run_git_interactive(&commit_args, &ctx.repo_root)?;
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/commit"),
            subrepo_commit_ref,
        ],
        &ctx.repo_root,
    )?;

    // Remove worktree
    delete_branch_and_worktree(ctx, subdir, subref)?;

    Ok(())
}
