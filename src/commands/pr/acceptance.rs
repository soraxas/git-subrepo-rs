//! Read-only discovery of merged GitHub PRs for a published export.
use super::discovery::Export;
use anyhow::{Context as _, Result, bail, ensure};
use std::process::Command;

/// An accepted upstream PR whose head matches the published export revision.
pub(super) struct MergedPr {
    pub url: String,
    pub commit: String,
}

/// Find merged PRs on the recorded base branch using the optional GitHub CLI.
pub(super) fn merged_prs(export: &Export) -> Result<Vec<MergedPr>> {
    let repo = github_repo(&export.upstream).context(
        "This upstream is not a supported github.com URL; enter the accepted SHA manually",
    )?;
    let output = Command::new("gh")
        .args([
            "pr", "list", "--repo", &repo, "--state", "merged", "--base", &export.base_branch,
            "--head", &export.branch, "--limit", "100", "--json", "url,headRefOid,mergeCommit",
            "--jq", ".[] | [.url, .headRefOid, .mergeCommit.oid] | @tsv",
        ])
        .env("GH_PROMPT_DISABLED", "1")
        .current_dir(&export.repository)
        .output()
        .context("Could not run 'gh'; install/authenticate GitHub CLI or enter the accepted SHA manually")?;
    ensure!(
        output.status.success(),
        "GitHub CLI lookup failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    parse_prs(&String::from_utf8(output.stdout)?, &repo, &export.published)
}

fn github_repo(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let path = path
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_end_matches('/'));
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || [".", ".."].contains(part)
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
    {
        return None;
    }
    Some(path.to_owned())
}

fn parse_prs(text: &str, repo: &str, published: &str) -> Result<Vec<MergedPr>> {
    let mut found = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split('\t').collect();
        ensure!(
            fields.len() == 3,
            "Unexpected merged PR response from GitHub CLI"
        );
        let [url, head, commit] = fields[..] else {
            unreachable!()
        };
        if head != published {
            continue;
        }
        let prefix = format!("https://github.com/{repo}/pull/");
        let number = url.strip_prefix(&prefix).unwrap_or("");
        if number.is_empty()
            || !number.bytes().all(|b| b.is_ascii_digit())
            || ![40, 64].contains(&commit.len())
            || !commit.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("Merged PR response has an invalid URL or accepted commit");
        }
        if !found.iter().any(|pr: &MergedPr| pr.url == url) {
            found.push(MergedPr {
                url: url.to_owned(),
                commit: commit.to_owned(),
            });
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_upstreams_support_https_and_ssh_without_accepting_other_hosts() {
        for url in [
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git/",
            "git@github.com:owner/repo.git",
            "ssh://git@github.com/owner/repo.git",
        ] {
            assert_eq!(github_repo(url).as_deref(), Some("owner/repo"));
        }
        for url in [
            "/local/repo",
            "https://github.com.evil/owner/repo",
            "https://github.com/owner/repo/extra",
            "https://github.com/owner/..",
            "https://github.com//repo",
            "https://github.com/owner/repo?x=y",
        ] {
            assert!(github_repo(url).is_none(), "{url}");
        }
    }

    #[test]
    fn lookup_matches_the_published_tip_and_rejects_invalid_acceptance_data() {
        let head = "a".repeat(40);
        let commit = "b".repeat(40);
        let text = format!(
            "https://github.com/o/r/pull/1\t{}\t{commit}\nhttps://github.com/o/r/pull/2\t{head}\t{commit}\n",
            "c".repeat(40)
        );
        let found = parse_prs(&text, "o/r", &head).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://github.com/o/r/pull/2");
        assert_eq!(found[0].commit, commit);
        assert!(parse_prs("", "o/r", &head).unwrap().is_empty());
        assert!(parse_prs("malformed", "o/r", &head).is_err());
        for row in [
            format!("https://evil/pull/1\t{head}\t{commit}"),
            format!("https://github.com/o/r/pull/2\t{head}\tmissing"),
        ] {
            assert!(parse_prs(&row, "o/r", &head).is_err());
        }
    }
}
