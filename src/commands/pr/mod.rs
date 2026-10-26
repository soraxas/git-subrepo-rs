//! Explicit PR sessions: preparation/publication are separate from accepted upstream sync.
pub(crate) mod discovery;
mod git;
mod menu;
mod reconcile;
mod state;

use super::Context;
use crate::cli::PrArgs;
use crate::git_utils::{run_git, try_run_git};
use crate::gitrepo::read_gitrepo;
use anyhow::{Context as _, Result, bail, ensure};
use state::{Lock, Session};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// Execute one PR session operation from its owning checkout.
pub fn run(mut args: PrArgs, quiet: bool) -> Result<()> {
    validate_options(&args)?;
    let Some(action) = args.action.as_deref() else {
        if !quiet && std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
            return menu::run();
        }
        println!(
            "Run 'git subrepo pr' in a terminal for guided PR actions, or 'git subrepo pr --help' for explicit commands."
        );
        return Ok(());
    };
    let subdir = args
        .subdir
        .as_deref()
        .context("PR operation requires <subdir>; run 'git subrepo pr' for guided actions")?;
    let ctx = Context::new()?;
    git::relative(subdir)?;
    // Equivalent spellings must share one session and mutation lock.
    let subdir = Path::new(subdir)
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .into_owned();
    let root = discovery::session_root(&ctx.repo_root, &subdir)?;
    args.subdir = Some(subdir);
    if action == "status" {
        return status(&root);
    }
    let _lock = Lock::acquire(&root)?;
    match action {
        "prepare" => prepare(&ctx, &root, &args, quiet),
        "publish" => publish(&root, &args, quiet),
        "reconcile" => reconcile::run(&ctx, &root, &args, quiet),
        "close" => {
            let mut session = Session::load(&root)?;
            session.set("phase", "closed");
            session.save()?;
            if !quiet {
                println!(
                    "Closed PR session; export repository retained at {}",
                    session.repo()?.display()
                );
            }
            Ok(())
        }
        _ => bail!("Unknown PR operation"),
    }
}

/// Execute a menu-confirmed operation only while the selected session and destination still match.
pub(crate) fn confirmed(
    export: &discovery::Export,
    subdir: &str,
    operation: &str,
    revision: &str,
) -> Result<()> {
    ensure!(
        ["publish", "reconcile"].contains(&operation),
        "Unsupported confirmed operation"
    );
    let ctx = Context::at(&export.owner)?;
    let root = discovery::session_root(&ctx.repo_root, subdir)?;
    let _lock = Lock::acquire(&root)?;
    let session = Session::load(&root)?;
    ensure!(
        session.repo()? == export.repository
            && session.get("subdir")? == subdir
            && session.get("branch")? == export.branch
            && session.get("remote")? == export.remote
            && session.get("upstream")? == export.upstream
            && session.get("base")? == export.base
            && session.get("base-branch")? == export.base_branch,
        "Export session changed while confirming. Review the current session and its destination before continuing."
    );
    let args = PrArgs {
        action: Some(operation.to_owned()),
        subdir: Some(subdir.to_owned()),
        branch: None,
        base: None,
        remote: None,
        upstream: None,
        message: None,
        path: Vec::new(),
        expected_head: (operation == "publish").then(|| revision.to_owned()),
        merged: (operation == "reconcile").then(|| revision.to_owned()),
    };
    match operation {
        "publish" => publish(&root, &args, false),
        "reconcile" => reconcile::run(&ctx, &root, &args, false),
        _ => unreachable!(),
    }
}

fn validate_options(args: &PrArgs) -> Result<()> {
    if args.action.as_deref() != Some("prepare") {
        ensure!(
            args.branch.is_none()
                && args.base.is_none()
                && args.remote.is_none()
                && args.upstream.is_none()
                && args.message.is_none()
                && args.path.is_empty(),
            "Branch, remote, message and path options belong to 'pr prepare'"
        );
    }
    ensure!(
        args.expected_head.is_none() || args.action.as_deref() == Some("publish"),
        "--expected-head belongs to 'pr publish'"
    );
    ensure!(
        args.merged.is_none() || args.action.as_deref() == Some("reconcile"),
        "--merged belongs to 'pr reconcile'"
    );
    Ok(())
}

fn prepare(ctx: &Context, root: &Path, args: &PrArgs, quiet: bool) -> Result<()> {
    let subdir = args
        .subdir
        .as_deref()
        .context("prepare requires <subdir>")?;
    discovery::require_committed_source(ctx, subdir)?;
    let config_path = ctx.repo_root.join(subdir).join(".gitrepo");
    ensure!(config_path.is_file(), "Not a subrepo: {subdir}");
    let cfg = read_gitrepo(&config_path, &ctx.repo_root)?;
    if root.join("session").exists() {
        let previous = Session::load(root)?;
        let phase = previous.get("phase")?;
        ensure!(
            phase == "closed" || (phase == "staged" && previous.get("accepted")? == cfg.commit),
            "A PR session is active; use 'pr status', finish reconciliation, or explicitly 'pr close' it"
        );
        // Staged reconciliation must have been committed, not just left in the index.
        if phase == "staged" {
            git::clean(&ctx.repo_root, Some(subdir))?;
        }
    }
    let branch = args
        .branch
        .as_deref()
        .context("prepare requires --branch <topic>")?;
    let base = args.base.as_deref().unwrap_or(&cfg.branch);
    for name in [branch, base] {
        run_git(
            &["check-ref-format", &format!("refs/heads/{name}")],
            &ctx.repo_root,
        )?;
    }
    ensure!(
        branch != base,
        "PR topic branch must differ from the upstream base branch"
    );
    let message = args
        .message
        .as_deref()
        .filter(|m| !m.trim().is_empty())
        .context("prepare requires --message <public commit message>")?;
    let upstream = state::remote_url(
        args.upstream.as_deref().unwrap_or(&cfg.remote),
        &ctx.repo_root,
    )?;
    let remote = state::remote_url(
        args.remote.as_deref().unwrap_or(&cfg.remote),
        &ctx.repo_root,
    )?;
    ensure!(
        !cfg.commit.is_empty(),
        "Subrepo has no imported upstream revision; pull or clone before preparing a PR"
    );
    for path in &args.path {
        git::relative(path)?;
        ensure!(path != ".gitrepo", "Root .gitrepo is not exportable");
    }

    let index = root.join("snapshot-index");
    let source_tree = git::subtree(&ctx.repo_root, subdir, &index)?;
    let source_tree =
        crate::exclusions::restore(&ctx.repo_root, &source_tree, &cfg.commit, &cfg.exclude)?;
    let proposal = if args.path.is_empty() {
        source_tree
    } else {
        git::index_git(&ctx.repo_root, &index, &["read-tree", &cfg.commit])?;
        let paths: Vec<_> = args.path.iter().map(|p| format!(":(literal){p}")).collect();
        let mut diff = vec![
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            &cfg.commit,
            &source_tree,
            "--",
        ];
        diff.extend(paths.iter().map(String::as_str));
        let patch = git::bytes(&ctx.repo_root, &diff, None, None)?;
        ensure!(
            !patch.is_empty(),
            "Selected paths have no changes to export"
        );
        git::bytes(
            &ctx.repo_root,
            &["apply", "--cached", "--binary"],
            Some(&patch),
            Some(&index),
        )?;
        git::index_git(&ctx.repo_root, &index, &["write-tree"])?
    };
    ensure!(
        proposal != git::tree(&ctx.repo_root, &cfg.commit)?,
        "No changes to export"
    );
    let snapshot = git::commit(&ctx.repo_root, &proposal, &cfg.commit, message)?;
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
        .to_string();
    let work = root.join("sessions").join(id).join("repository");
    std::fs::create_dir_all(work.parent().context("Missing export parent")?)?;
    run_git(
        &[
            "clone",
            "--no-local",
            "--no-tags",
            "--single-branch",
            "--origin",
            "upstream",
            "--branch",
            base,
            &upstream,
            &work.to_string_lossy(),
        ],
        &ctx.repo_root,
    )?;
    git::identity(&ctx.repo_root, &work)?;
    let upstream_base = git::head(&work)?;
    ensure!(
        git::ancestor(&work, &cfg.commit, &upstream_base),
        "Imported revision is not an ancestor of the upstream base; reconcile the upstream history before exporting"
    );
    run_git(&["remote", "add", "publish", &remote], &work)?;
    run_git(&["switch", "-c", branch], &work)?;
    run_git(
        &[
            "fetch",
            "--no-tags",
            &ctx.repo_root.to_string_lossy(),
            &format!("{snapshot}:refs/subrepo-pr/source"),
        ],
        &work,
    )?;
    let mut session = Session::new(root);
    for (key, value) in [
        ("version", "1"),
        ("phase", "prepared"),
        ("subdir", subdir),
        ("source", &ctx.original_head_commit),
        ("imported", &cfg.commit),
        ("proposal", &snapshot),
        ("base", &upstream_base),
        ("base-branch", base),
        ("branch", branch),
        ("upstream", &upstream),
        ("remote", &remote),
    ] {
        session.set(key, value);
    }
    session.set("owner", ctx.repo_root.to_string_lossy());
    session.set("repository", work.to_string_lossy());
    session.set_exclusions(&cfg.exclude);
    session.save()?;
    run_git(&["cherry-pick", &snapshot], &work).map_err(|error| anyhow::anyhow!("{error}\nExport requires resolution in {}. Resolve and run git cherry-pick --continue there; the owning checkout is unchanged.", work.display()))?;
    if !quiet {
        for path in &cfg.exclude {
            eprintln!("Excluded local changes: {path:?}");
        }
        eprintln!(
            "Prepared PR branch '{branch}' against '{base}'. Review and test this standalone repository before publishing."
        );
    }
    println!("{}", work.display());
    Ok(())
}

fn status(root: &Path) -> Result<()> {
    let session = Session::load(root)?;
    for (key, value) in &session.values {
        println!("{key}: {value}");
    }
    let work = session.repo()?;
    let head = git::head(&work)?;
    println!("current-head: {head}");
    println!(
        "unpublished-changes: {}",
        session.optional("published") != head
    );
    let dirty = run_git(&["status", "--short"], &work)?;
    if !dirty.is_empty() {
        println!("working-changes:\n{dirty}");
    }
    Ok(())
}

/// Verify the exact clean, linear candidate; every outgoing commit excludes root metadata.
fn candidate(session: &Session) -> Result<String> {
    let work = session.repo()?;
    git::clean(&work, None)?;
    let head = git::head(&work)?;
    let base = session.get("base")?;
    ensure!(
        head != base && git::ancestor(&work, base, &head),
        "Candidate must descend from its recorded upstream base and contain an export"
    );
    let range = format!("{base}..{head}");
    ensure!(
        run_git(&["rev-list", "--merges", &range], &work)?.is_empty(),
        "Candidate must be a linear series of public commits; review and resolve merged history before publishing"
    );
    for commit in run_git(&["rev-list", &range], &work)?.lines() {
        ensure!(
            !try_run_git(&["cat-file", "-e", &format!("{commit}:.gitrepo")], &work).0,
            "Outgoing commit {commit} contains root .gitrepo metadata"
        );
        let tree = git::tree(&work, commit)?;
        ensure!(
            crate::exclusions::restore(&work, &tree, base, &session.exclusions()?)? == tree,
            "Outgoing commit {commit} changes an excluded path"
        );
    }
    Ok(head)
}

fn publish(root: &Path, args: &PrArgs, quiet: bool) -> Result<()> {
    let expected = args
        .expected_head
        .as_deref()
        .context("publish requires --expected-head <full reviewed SHA>")?;
    let mut session = Session::load(root)?;
    ensure!(
        session.get("phase")? == "prepared",
        "Only an active prepared PR can be published"
    );
    let head = candidate(&session)?;
    ensure!(
        head == expected,
        "Candidate changed since review; audit and test its current HEAD before publishing"
    );
    let work = session.repo()?;
    let published = session.optional("published");
    if !published.is_empty() {
        ensure!(
            git::ancestor(&work, published, &head),
            "Review updates must append to the published history; this command never force-pushes"
        );
    }
    let remote = session.get("remote")?;
    let branch = session.get("branch")?;
    let remote_ref = format!("refs/heads/{branch}");
    let advertised = run_git(&["ls-remote", "--heads", remote, &remote_ref], &work)?;
    let remote_head = advertised.split_whitespace().next().unwrap_or("");
    ensure!(
        remote_head == published
            || remote_head == head
            || (!published.is_empty()
                && !remote_head.is_empty()
                && git::ancestor(&work, published, remote_head)
                && git::ancestor(&work, remote_head, &head)),
        "Remote topic branch changed outside this PR session; fetch and review its commits before updating it"
    );
    if remote_head != head {
        // The explicit lease also rejects races creating a new topic branch.
        // Ancestry was checked above; rewriting published history is never allowed.
        run_git(
            &[
                "-c",
                "push.followTags=false",
                "push",
                &format!("--force-with-lease={remote_ref}:{remote_head}"),
                remote,
                &format!("{head}:{remote_ref}"),
            ],
            &work,
        )?;
    }
    session.set("published", &head);
    session.save()?;
    if !quiet {
        println!(
            "Published reviewed commit {head}. Sync metadata is unchanged; create/update the upstream PR explicitly."
        );
    }
    Ok(())
}
