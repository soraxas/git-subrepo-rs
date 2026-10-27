use super::{candidate, discovery, git, state::Session};
use crate::cli::PrArgs;
use crate::commands::Context;
use crate::encode::encode_subdir;
use crate::git_utils::{run_git, try_run_git};
use crate::gitrepo::{read_gitrepo, update_gitrepo};
use anyhow::{Context as _, Result, bail, ensure};
use std::path::{Path, PathBuf};

/// Stage an accepted PR plus retained local work; never commit the owning checkout.
pub(super) fn run(ctx: &Context, root: &Path, args: &PrArgs, quiet: bool) -> Result<()> {
    let merged = args
        .merged
        .as_deref()
        .context("reconcile requires --merged <accepted upstream SHA>")?;
    ensure!(
        [40, 64].contains(&merged.len()) && merged.bytes().all(|b| b.is_ascii_hexdigit()),
        "--merged must be a full commit SHA"
    );
    let mut session = Session::load(root)?;
    ensure!(
        ["prepared", "resolving"].contains(&session.get("phase")?),
        "PR session is not awaiting reconciliation"
    );
    let owner = &ctx.repo_root;
    ensure!(
        git::ancestor(owner, session.get("source")?, &ctx.original_head_commit),
        "Owning checkout no longer descends from the exported source revision"
    );
    let subdir = args
        .subdir
        .as_deref()
        .context("reconcile requires <subdir>")?;
    require_clean_owner(ctx, subdir)?;
    let metadata_snapshot = MetadataSnapshot::read(owner, subdir)?;
    let cfg = read_gitrepo(&owner.join(subdir).join(".gitrepo"), owner)?;
    ensure!(
        cfg.commit == session.get("imported")?,
        "Subrepo sync baseline changed while this PR was open; reconcile that change before importing this session"
    );
    let published = session.get("published")?.to_owned();
    ensure!(
        candidate(&session)? == published,
        "Candidate has unpublished changes; publish its reviewed HEAD before reconciling"
    );
    let work = session.repo()?;
    let upstream = git::fetch(&work, session.get("upstream")?, session.get("base-branch")?)?;
    ensure!(
        git::ancestor(&work, merged, &upstream),
        "Accepted commit is not on the configured upstream base branch"
    );
    if !git::ancestor(&work, &published, merged) {
        // Squash/rebase acceptance must reproduce the reviewed candidate, allowing
        // independent upstream changes made before the merge.
        let parent = run_git(&["rev-parse", &format!("{merged}^1")], &work)?;
        let (ok, combined) =
            try_run_git(&["merge-tree", "--write-tree", &published, &parent], &work);
        ensure!(
            ok && combined.lines().next() == Some(git::tree(&work, merged)?.as_str()),
            "Accepted commit does not match the reviewed candidate (including upstream changes); inspect the PR result before reconciling"
        );
    }

    let merged_tree = if session.get("phase")? == "resolving" {
        ensure!(
            session.get("accepted")? == merged,
            "Resume with the same accepted commit"
        );
        ensure!(
            session.get("local-head")? == ctx.original_head_commit,
            "Local HEAD changed during conflict resolution; preserve the resolution, close this session and prepare a fresh reconciliation"
        );
        resolved_tree(&session)?
    } else {
        run_git(
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                &work.to_string_lossy(),
                merged,
                session.get("proposal")?,
            ],
            owner,
        )?;
        let local_tree = git::subtree(owner, subdir, &root.join("reconcile-index"))?;
        let mut exclusions = session.exclusions()?;
        exclusions.extend(cfg.exclude.iter().cloned());
        let incoming_tree = crate::exclusions::restore(
            owner,
            &git::tree(owner, merged)?,
            &local_tree,
            &exclusions,
        )?;
        let proposal = session.get("proposal")?;
        let local = git::commit(owner, &local_tree, proposal, "Retained local changes")?;
        let incoming = git::commit(owner, &incoming_tree, proposal, "Accepted upstream changes")?;
        let (ok, result) = try_run_git(&["merge-tree", "--write-tree", &local, &incoming], owner);
        if !ok {
            begin_resolution(ctx, &mut session, &local, &incoming, merged)?;
            bail!(
                "Reconciliation needs conflict resolution in {}. Resolve and stage files there, then repeat this reconcile command. The owning checkout is unchanged.",
                session.get("integration")?
            );
        }
        result
            .lines()
            .next()
            .context("Git returned no reconciliation tree")?
            .to_owned()
    };
    stage(
        ctx,
        &session,
        &merged_tree,
        merged,
        &cfg.method,
        &metadata_snapshot,
    )?;
    session.set("phase", "staged");
    session.set("accepted", merged);
    session.set("integration-tree", &merged_tree);
    session.save()?;
    if !quiet {
        println!(
            "Staged accepted PR and retained local changes in {}. Review the staged diff and commit it before preparing another export.",
            subdir
        );
    }
    Ok(())
}

#[derive(PartialEq)]
struct MetadataSnapshot {
    contents: Vec<u8>,
    index_entry: String,
}

impl MetadataSnapshot {
    fn read(owner: &Path, subdir: &str) -> Result<Self> {
        ensure!(
            run_git(
                &[
                    "ls-files",
                    "--unmerged",
                    "--",
                    &format!(":(literal){subdir}/.gitrepo")
                ],
                owner
            )?
            .is_empty(),
            "Resolve root .gitrepo merge conflicts before reconciliation"
        );
        Ok(Self {
            contents: std::fs::read(owner.join(subdir).join(".gitrepo"))?,
            index_entry: run_git(
                &[
                    "ls-files",
                    "--stage",
                    "--",
                    &format!(":(literal){subdir}/.gitrepo"),
                ],
                owner,
            )?,
        })
    }
}

fn require_clean_owner(ctx: &Context, subdir: &str) -> Result<()> {
    let changes = run_git(
        &[
            "status",
            "--porcelain",
            "--untracked-files=no",
            "--",
            ".",
            &format!(":(exclude,literal){subdir}/.gitrepo"),
        ],
        &ctx.repo_root,
    )?;
    ensure!(
        changes.is_empty(),
        "Owning checkout has tracked/index changes outside the root .gitrepo:\n{changes}\nCommit or reconcile those source changes first"
    );
    discovery::require_committed_source(ctx, subdir)
}

fn begin_resolution(
    ctx: &Context,
    session: &mut Session,
    local: &str,
    incoming: &str,
    accepted: &str,
) -> Result<()> {
    let integration = session
        .repo()?
        .parent()
        .context("Missing session directory")?
        .join("integration");
    std::fs::create_dir(&integration)?;
    run_git(&["init", "-q"], &integration)?;
    git::identity(&ctx.repo_root, &integration)?;
    run_git(
        &[
            "fetch",
            "--no-tags",
            &ctx.repo_root.to_string_lossy(),
            local,
            incoming,
        ],
        &integration,
    )?;
    run_git(&["switch", "-c", "reconcile", local], &integration)?;
    session.set("integration", integration.to_string_lossy());
    session.set("local-head", &ctx.original_head_commit);
    session.set("local-commit", local);
    session.set("incoming-commit", incoming);
    session.set("accepted", accepted);
    session.set("phase", "resolving");
    session.save()?;
    // A conflict is the expected outcome; preserve its index for explicit resolution.
    let (ok, message) = try_run_git(&["merge", "--no-commit", "--no-ff", incoming], &integration);
    ensure!(
        ok || try_run_git(&["rev-parse", "--verify", "MERGE_HEAD"], &integration).0,
        "Could not start integration merge: {message}"
    );
    Ok(())
}

fn resolved_tree(session: &Session) -> Result<String> {
    let work = PathBuf::from(session.get("integration")?);
    ensure!(
        run_git(&["ls-files", "--unmerged"], &work)?.is_empty(),
        "Integration still has unresolved conflicts"
    );
    ensure!(
        try_run_git(&["diff", "--quiet"], &work).0,
        "Stage all integration resolutions before continuing"
    );
    ensure!(
        run_git(&["ls-files", "--others", "--exclude-standard"], &work)?.is_empty(),
        "Integration contains untracked files; account for them before continuing"
    );
    let head = git::head(&work)?;
    let local = session.get("local-commit")?;
    let incoming = session.get("incoming-commit")?;
    let merging = try_run_git(&["rev-parse", "--verify", "MERGE_HEAD"], &work);
    ensure!(
        (head == local && merging.0 && merging.1 == incoming)
            || (git::ancestor(&work, local, &head) && git::ancestor(&work, incoming, &head)),
        "Integration no longer contains the recorded local and accepted changes"
    );
    let tree = run_git(&["write-tree"], &work)?;
    // Transfer the resolution tree through a commit containing only subtree history.
    let resolution = git::commit(&work, &tree, local, "Resolved PR integration")?;
    let owner = Path::new(session.get("owner")?);
    run_git(
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            &work.to_string_lossy(),
            &resolution,
        ],
        owner,
    )?;
    Ok(tree)
}

fn stage(
    ctx: &Context,
    session: &Session,
    tree: &str,
    accepted: &str,
    method: &str,
    snapshot: &MetadataSnapshot,
) -> Result<()> {
    let owner = &ctx.repo_root;
    let subdir = session.get("subdir")?;
    let index = session.root.join("stage-index");
    git::index_git(owner, &index, &["read-tree", &ctx.original_head_commit])?;
    git::index_git(
        owner,
        &index,
        &[
            "rm",
            "-r",
            "--cached",
            "--ignore-unmatch",
            "--",
            &format!(":(literal){subdir}"),
        ],
    )?;
    git::index_git(
        owner,
        &index,
        &["read-tree", &format!("--prefix={subdir}/"), tree],
    )?;
    let metadata = session.root.join("accepted.gitrepo");
    std::fs::write(&metadata, &snapshot.contents)?;
    update_gitrepo(
        &metadata,
        Some(session.get("upstream")?),
        Some(session.get("base-branch")?),
        accepted,
        Some(&ctx.original_head_commit),
        method,
        env!("CARGO_PKG_VERSION"),
        owner,
    )?;
    let blob = run_git(&["hash-object", "-w", &metadata.to_string_lossy()], owner)?;
    // Leave metadata out of the source checkout transaction. Its local staged
    // and working contents may differ; Git must not overwrite them as source.
    let previous_blob = run_git(
        &[
            "rev-parse",
            &format!("{}:{subdir}/.gitrepo", ctx.original_head_commit),
        ],
        owner,
    )?;
    git::index_git(
        owner,
        &index,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{previous_blob},{subdir}/.gitrepo"),
        ],
    )?;
    let checkout_tree = git::index_git(owner, &index, &["write-tree"])?;
    ensure!(
        git::head(owner)? == ctx.original_head_commit,
        "Owning HEAD changed during reconciliation"
    );
    require_clean_owner(ctx, subdir)?;
    ensure!(
        MetadataSnapshot::read(owner, subdir)? == *snapshot,
        "Root .gitrepo changed during reconciliation; retry with its current settings"
    );
    // Git checks for conflicting local/untracked files before updating the real
    // index and working tree. Nothing outside the subtree differs in checkout_tree.
    // Anchor the old tree: a concurrent commit outside the subtree must not
    // become a moving baseline whose changes this update would reverse.
    run_git(
        &[
            "read-tree",
            "-m",
            "-u",
            &ctx.original_head_commit,
            &checkout_tree,
        ],
        owner,
    )?;
    // Source checkout succeeded without touching local metadata. Stage the
    // accepted baseline together with the snapshotted working-copy settings.
    ensure!(
        MetadataSnapshot::read(owner, subdir)? == *snapshot,
        "Root .gitrepo changed during checkout; local metadata was preserved. Review the staged source import before continuing"
    );
    std::fs::copy(&metadata, owner.join(subdir).join(".gitrepo"))?;
    run_git(
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{blob},{subdir}/.gitrepo"),
        ],
        owner,
    )?;
    let subref = encode_subdir(subdir);
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/fetch"),
            accepted,
        ],
        owner,
    )?;
    Ok(())
}
