pub mod branch_cmd;
pub mod clean;
pub mod clone;
pub mod commit_cmd;
pub mod config;
pub mod fetch;
pub mod init;
pub mod pull;
pub mod push;
pub mod status;

use crate::git_utils::{branch_exists, commit_in_rev_list, rev_exists, run_git, try_run_git};
use anyhow::Result;
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

/// Assert that the working copy is clean.
pub fn assert_clean_for(cmd: &str, ctx: &Context) -> Result<()> {
    run_git(
        &["update-index", "-q", "--ignore-submodules", "--refresh"],
        &ctx.repo_root,
    )?;

    let (ok1, _) = try_run_git(
        &["diff-files", "--quiet", "--ignore-submodules"],
        &ctx.repo_root,
    );
    if !ok1 {
        anyhow::bail!("Can't {cmd} subrepo. Unstaged changes.");
    }

    if !ctx.original_head_commit.is_empty() {
        let (ok2, _) = try_run_git(
            &["diff-index", "--quiet", "--ignore-submodules", "HEAD"],
            &ctx.repo_root,
        );
        if !ok2 {
            anyhow::bail!("Can't {cmd} subrepo. Working tree has changes.");
        }

        let (ok3, _) = try_run_git(
            &[
                "diff-index",
                "--quiet",
                "--cached",
                "--ignore-submodules",
                "HEAD",
            ],
            &ctx.repo_root,
        );
        if !ok3 {
            anyhow::bail!("Can't {cmd} subrepo. Index has changes.");
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
        "git subrepo {command}{merge_suffix} {subdir}\n\nsubrepo:\n  subdir:   \"{subdir}\"\n  merged:   \"{merged}\"\nupstream:\n  origin:   \"{remote}\"\n  branch:   \"{branch}\"\n  commit:   \"{commit}\"\ngit-subrepo:\n  version:  \"{VERSION}\"\n  origin:   \"???\"\n  commit:   \"???\"\n"
    )
}

/// Perform the subrepo:fetch operation.
/// Returns the upstream HEAD commit SHA.
pub fn subrepo_fetch(ctx: &Context, remote: &str, branch: &str, subref: &str) -> Result<String> {
    run_git(
        &["fetch", "--no-tags", "--quiet", remote, branch],
        &ctx.repo_root,
    )?;

    let upstream_head = run_git(&["rev-parse", "FETCH_HEAD^0"], &ctx.repo_root)?;
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/fetch"),
            &upstream_head,
        ],
        &ctx.repo_root,
    )?;

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
) -> Result<PathBuf> {
    let branch_name = format!("subrepo/{subref}");

    if branch_exists(&branch_name, &ctx.repo_root) {
        return Err(anyhow::anyhow!(
            "Branch '{branch_name}' already exists. Use '--force' to override."
        ));
    }

    if subrepo_parent.is_empty() {
        // No parent: use subdirectory filter
        subrepo_branch_no_parent(ctx, subdir, subref, &branch_name)?;
    } else {
        // Has parent: build commit chain
        subrepo_branch_with_parent(
            ctx,
            subdir,
            subref,
            subrepo_parent,
            join_method,
            &branch_name,
        )?;
    }

    let worktree = ctx.worktree_path(subdir);
    run_git(
        &["worktree", "add", &worktree.to_string_lossy(), &branch_name],
        &ctx.repo_root,
    )?;

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

fn subrepo_branch_with_parent(
    ctx: &Context,
    subdir: &str,
    subref: &str,
    subrepo_parent: &str,
    join_method: &str,
    branch_name: &str,
) -> Result<()> {
    // Check if subrepo_parent is an ancestor of HEAD (handles rebase case)
    let (is_ancestor, _) = try_run_git(
        &["merge-base", "--is-ancestor", subrepo_parent, "HEAD"],
        &ctx.repo_root,
    );

    if !is_ancestor {
        // Parent is not an ancestor - likely caused by rebase
        // Find the previous merge point from the .gitrepo file in history
        let gitrepo_rel = format!("{subdir}/.gitrepo");
        let (_, merge_log) = try_run_git(
            &[
                "log",
                "-1",
                "-G",
                "commit =",
                "--format=%H",
                "--",
                &gitrepo_rel,
            ],
            &ctx.repo_root,
        );
        let merge_point = if !merge_log.trim().is_empty() {
            let (_, parent_of_merge) = try_run_git(
                &["rev-parse", &format!("{}^", merge_log.trim())],
                &ctx.repo_root,
            );
            parent_of_merge
        } else {
            String::new()
        };
        anyhow::bail!(
            "The last sync point (where upstream and the subrepo were equal) is not an ancestor of the current HEAD.\nThis was probably caused by a rebase. Previous merge point was: {}",
            merge_point.trim()
        );
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

        // Check that gitrepo_commit is reachable from the fetch ref
        if rev_exists(&refs_subrepo_fetch, &ctx.repo_root)
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

    // Remove worktree if it exists
    if worktree.exists() {
        std::fs::remove_dir_all(&worktree)?;
        try_run_git(&["worktree", "prune"], &ctx.repo_root);
    }

    // Delete branch if it exists
    if branch_exists(&branch_name, &ctx.repo_root) {
        try_run_git(&["branch", "-D", &branch_name], &ctx.repo_root);
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

    run_git(&["commit", "-m", &commit_msg], &ctx.repo_root)?;

    // Update commit ref
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/commit"),
            subrepo_commit_ref,
        ],
        &ctx.repo_root,
    )?;

    // Remove worktree
    let worktree = ctx.worktree_path(subdir);
    if worktree.exists() {
        std::fs::remove_dir_all(&worktree)?;
        try_run_git(&["worktree", "prune"], &ctx.repo_root);
    }

    Ok(())
}
