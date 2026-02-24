use crate::commands::{Context, assert_clean_for, normalize_subdir, subrepo_fetch};
use crate::encode::encode_subdir;
use crate::git_utils::{run_git, try_run_git};
use anyhow::Result;

pub fn run(
    remote: String,
    subdir_opt: Option<String>,
    branch_opt: Option<String>,
    force: bool,
    method: Option<String>,
    quiet: bool,
    message: Option<String>,
) -> Result<()> {
    let mut ctx = Context::new()?;
    ctx.quiet = quiet;

    // Check HEAD exists (can't clone into empty repo)
    let (head_ok, _) = try_run_git(&["rev-parse", "HEAD"], &ctx.repo_root);
    if !head_ok {
        eprintln!("git-subrepo: You can't clone into an empty repository");
        std::process::exit(1);
    }

    assert_clean_for("clone", &ctx)?;

    // Determine subdir
    let subdir = match subdir_opt {
        Some(s) => normalize_subdir(&s),
        None => guess_subdir(&remote)?,
    };
    let subref = encode_subdir(&subdir);

    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");

    // Check subdir non-empty before any network operations (only for new clone)
    let subdir_path = ctx.repo_root.join(&subdir);
    if !gitrepo_path.exists() && subdir_path.exists() {
        let entries: Vec<_> = std::fs::read_dir(&subdir_path)
            .map(|r| r.collect::<Vec<_>>())
            .unwrap_or_default();
        if !entries.is_empty() {
            anyhow::bail!("The subdir '{}' exists and is not empty.", subdir);
        }
    }

    // Determine branch (only for non-force path; reclone will fetch it)
    let subrepo_branch = if let Some(ref b) = branch_opt {
        b.clone()
    } else if !force || !gitrepo_path.exists() {
        get_upstream_head_branch(&remote, &ctx)?
    } else {
        String::new() // will be determined in reclone path
    };

    let join_method = method.as_deref().unwrap_or("merge").to_string();

    // Handle reclone (--force with existing .gitrepo)
    if gitrepo_path.exists() && force {
        // Read current state to check if already up to date
        let existing_cfg = crate::gitrepo::read_gitrepo(&gitrepo_path, &ctx.repo_root).ok();

        // Fetch upstream first
        let fetch_branch = if let Some(b) = branch_opt.as_ref() {
            b.clone()
        } else {
            get_upstream_head_branch(&remote, &ctx)?
        };

        let upstream_head = subrepo_fetch(&ctx, &remote, &fetch_branch, &subref)?;

        // Check if already up to date
        if let Some(ref cfg) = existing_cfg
            && upstream_head == cfg.commit
        {
            if !quiet {
                println!("Subrepo '{subdir}' is up to date.");
            }
            return Ok(());
        }

        // Remove existing subdir for reclone
        let (_, ls) = try_run_git(&["ls-files", "--", &subdir], &ctx.repo_root);
        if !ls.trim().is_empty() {
            run_git(&["rm", "-r", "--", &subdir], &ctx.repo_root)?;
        }

        // Determine join method
        let join_method = method
            .as_deref()
            .unwrap_or(
                existing_cfg
                    .as_ref()
                    .map(|c| c.method.as_str())
                    .unwrap_or("merge"),
            )
            .to_string();

        // Create subdir
        std::fs::create_dir_all(ctx.repo_root.join(&subdir))?;

        // Read in upstream content
        let prefix = format!("{subdir}/");
        run_git(
            &["read-tree", "--prefix", &prefix, "-u", &upstream_head],
            &ctx.repo_root,
        )?;

        // Determine branch to use
        let actual_branch = branch_opt
            .as_deref()
            .map(|b| b.to_string())
            .unwrap_or(fetch_branch.clone());

        // Write .gitrepo file
        let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
        let gitrepo_path_str = gitrepo_path.to_string_lossy().into_owned();

        crate::gitrepo::write_new_gitrepo(
            &gitrepo_path,
            &remote,
            &actual_branch,
            &upstream_head,
            Some(&ctx.original_head_commit),
            &join_method,
            env!("CARGO_PKG_VERSION"),
            &ctx.repo_root,
        )?;

        run_git(&["add", "-f", "--", &gitrepo_path_str], &ctx.repo_root)?;
        let commit_msg =
            build_clone_commit_message(&subdir, &upstream_head, &remote, &actual_branch, &ctx);
        run_git(&["commit", "-m", &commit_msg], &ctx.repo_root)?;

        run_git(
            &[
                "update-ref",
                &format!("refs/subrepo/{subref}/commit"),
                &upstream_head,
            ],
            &ctx.repo_root,
        )?;

        if !quiet {
            println!("Subrepo '{remote}' ({actual_branch}) recloned into '{subdir}'.");
        }
        return Ok(());
    }

    // Fetch upstream
    let upstream_head = subrepo_fetch(&ctx, &remote, &subrepo_branch, &subref)?;

    // Create subdir
    std::fs::create_dir_all(ctx.repo_root.join(&subdir))?;

    // Commit the cloned content
    let commit_msg = match message {
        Some(ref m) => m.clone(),
        None => build_clone_commit_message(&subdir, &upstream_head, &remote, &subrepo_branch, &ctx),
    };

    // Remove subdir from index if it has files
    let (_, ls_out) = try_run_git(&["ls-files", "--", &subdir], &ctx.repo_root);
    if !ls_out.trim().is_empty() {
        run_git(&["rm", "-r", "--", &subdir], &ctx.repo_root)?;
    }

    // Read in upstream content
    let prefix = format!("{subdir}/");
    run_git(
        &["read-tree", "--prefix", &prefix, "-u", &upstream_head],
        &ctx.repo_root,
    )?;

    // Write .gitrepo file
    let gitrepo_path = ctx.repo_root.join(&subdir).join(".gitrepo");
    let gitrepo_path_str = gitrepo_path.to_string_lossy().into_owned();

    crate::gitrepo::write_new_gitrepo(
        &gitrepo_path,
        &remote,
        &subrepo_branch,
        &upstream_head,
        Some(&ctx.original_head_commit), // parent = HEAD before clone
        &join_method,
        env!("CARGO_PKG_VERSION"),
        &ctx.repo_root,
    )?;

    run_git(&["add", "-f", "--", &gitrepo_path_str], &ctx.repo_root)?;

    run_git(&["commit", "-m", &commit_msg], &ctx.repo_root)?;

    // Update refs
    run_git(
        &[
            "update-ref",
            &format!("refs/subrepo/{subref}/commit"),
            &upstream_head,
        ],
        &ctx.repo_root,
    )?;

    if !quiet {
        println!("Subrepo '{remote}' ({subrepo_branch}) cloned into '{subdir}'.");
    }

    Ok(())
}

fn guess_subdir(remote: &str) -> Result<String> {
    let dir = remote.trim_end_matches('/').trim_end_matches(".git");
    let dir = dir.rsplit('/').next().unwrap_or(dir);
    if dir.is_empty() {
        anyhow::bail!("Can't determine subdir from '{remote}'.")
    }
    if dir
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        Ok(dir.to_string())
    } else {
        anyhow::bail!("Can't determine subdir from '{remote}'.")
    }
}

fn get_upstream_head_branch(remote: &str, ctx: &Context) -> Result<String> {
    let (ok, output) = try_run_git(&["ls-remote", "--symref", remote], &ctx.repo_root);
    if !ok || output.is_empty() {
        anyhow::bail!("Command failed: 'git ls-remote --symref {remote}'.");
    }

    for line in output.lines() {
        // Format: "ref: refs/heads/master\tHEAD"
        if line.starts_with("ref:") && (line.ends_with("HEAD") || line.contains("\tHEAD")) {
            // Split on tab to get "ref: refs/heads/master"
            let ref_part = line.split('\t').next().unwrap_or(line);
            let ref_part = ref_part.trim_start_matches("ref:").trim();
            if let Some(branch) = ref_part.strip_prefix("refs/heads/") {
                return Ok(branch.to_string());
            }
        }
    }

    anyhow::bail!("Problem finding remote default head branch.")
}

fn build_clone_commit_message(
    subdir: &str,
    upstream_head: &str,
    remote: &str,
    branch: &str,
    ctx: &Context,
) -> String {
    let short = crate::git_utils::rev_parse_short(upstream_head, &ctx.repo_root)
        .unwrap_or_else(|| "none".to_string());
    format!(
        "git subrepo clone {subdir}\n\nsubrepo:\n  subdir:   \"{subdir}\"\n  merged:   \"{short}\"\nupstream:\n  origin:   \"{remote}\"\n  branch:   \"{branch}\"\n  commit:   \"{short}\"\ngit-subrepo:\n  version:  \"{}\"\n  origin:   \"???\"\n  commit:   \"???\"\n",
        env!("CARGO_PKG_VERSION")
    )
}
