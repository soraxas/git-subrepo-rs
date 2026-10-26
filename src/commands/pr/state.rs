use crate::git_utils::{run_git, try_run_git};
use anyhow::{Context as _, Result, ensure};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Checkout-local PR metadata, separate from the accepted `.gitrepo` baseline.
pub(super) struct Session {
    pub root: PathBuf,
    pub values: BTreeMap<String, String>,
}

impl Session {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            values: BTreeMap::new(),
        }
    }

    pub fn load(root: &Path) -> Result<Self> {
        let raw = run_git(
            &[
                "config",
                "--null",
                "--list",
                "--file",
                &root.join("session").to_string_lossy(),
            ],
            root,
        )
        .context("No PR session; run 'git subrepo pr prepare' first")?;
        let mut session = Self::new(root);
        for item in raw.split('\0').filter(|s| !s.is_empty()) {
            let (key, value) = item.split_once('\n').context("Invalid PR session record")?;
            session.values.insert(
                key.trim_start_matches("export.").to_owned(),
                value.to_owned(),
            );
        }
        ensure!(
            session.get("version")? == "1",
            "Unsupported PR session version"
        );
        Ok(session)
    }

    pub fn get(&self, key: &str) -> Result<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .with_context(|| format!("Missing PR session field: {key}"))
    }

    pub fn optional(&self, key: &str) -> &str {
        self.values.get(key).map(String::as_str).unwrap_or("")
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        self.values.insert(key.to_owned(), value.into());
    }

    pub fn repo(&self) -> Result<PathBuf> {
        Ok(PathBuf::from(self.get("repository")?))
    }

    pub fn save(&self) -> Result<()> {
        let temp = self.root.join("session.new");
        std::fs::write(&temp, "")?;
        for (key, value) in &self.values {
            run_git(
                &[
                    "config",
                    "--file",
                    &temp.to_string_lossy(),
                    &format!("export.{key}"),
                    value,
                ],
                &self.root,
            )?;
        }
        // Keep the audit record with its repository after another session starts.
        let repo = self.repo()?;
        let archive = repo.parent().context("Missing session directory")?;
        let archive_temp = archive.join("session.new");
        std::fs::copy(&temp, &archive_temp)?;
        std::fs::rename(archive_temp, archive.join("session"))?;
        std::fs::rename(temp, self.root.join("session"))?;
        Ok(())
    }
}

/// Serialize session mutations without touching the owning checkout's index.
pub(super) struct Lock(PathBuf);

impl Lock {
    pub fn acquire(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let path = root.join("lock");
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
            .with_context(|| format!("PR session is locked: {}. Check for an active command before removing a stale lock.", path.display()))?;
        use std::io::Write;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Resolve a configured remote name or a local repository path into a stable URL/path.
pub(super) fn remote_url(value: &str, cwd: &Path) -> Result<String> {
    ensure!(
        !value.is_empty() && value != "none" && !value.starts_with('-'),
        "A valid repository URL is required"
    );
    let (found, url) = try_run_git(&["remote", "get-url", value], cwd);
    let value = if found { url.as_str() } else { value };
    let local = cwd.join(value);
    if local.exists() {
        Ok(local.canonicalize()?.to_string_lossy().into_owned())
    } else {
        Ok(value.to_owned())
    }
}
