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

    /// Display the recorded creation time, or recover it from a legacy session ID.
    pub fn created_at(&self) -> Result<String> {
        let recorded = self.optional("created-at");
        if !recorded.is_empty() {
            return Ok(recorded.to_owned());
        }
        let repository = self.repo()?;
        let seconds = repository
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<u128>().ok())
            .and_then(|nanos| u64::try_from(nanos / 1_000_000_000).ok());
        Ok(seconds
            .and_then(format_time)
            .map(|time| format!("{time} (from legacy session ID)"))
            .unwrap_or_else(|| "unknown (older export)".to_owned()))
    }

    /// Record the exclusion policy used to prepare this export, preserving literal values.
    pub fn set_exclusions(&mut self, paths: &[String]) {
        self.set("exclude-count", paths.len().to_string());
        for (index, path) in paths.iter().enumerate() {
            self.set(&format!("exclude-{index}"), path);
        }
    }

    /// Read the prepared policy; older sessions have no configured exclusions.
    pub fn exclusions(&self) -> Result<Vec<String>> {
        let count: usize = self
            .values
            .get("exclude-count")
            .map(String::as_str)
            .unwrap_or("0")
            .parse()?;
        (0..count)
            .map(|index| {
                let path = self.get(&format!("exclude-{index}"))?;
                crate::exclusions::validate(path)?;
                Ok(path.to_owned())
            })
            .collect()
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

/// Format an export event as UTC without relying on locale or external date tools.
pub(super) fn format_time(seconds: u64) -> Option<String> {
    if seconds > 253_402_300_799 {
        return None;
    }
    let leap = |year: u64| {
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    };
    let mut days = seconds / 86_400;
    let mut year = 1970;
    loop {
        let length = if leap(year) { 366 } else { 365 };
        if days < length {
            break;
        }
        days -= length;
        year += 1;
    }
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0;
    while days >= months[month] {
        days -= months[month];
        month += 1;
    }
    let clock = seconds % 86_400;
    Some(format!(
        "{year:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        month + 1,
        days + 1,
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    ))
}

/// Record the wall-clock time of a new export event.
pub(super) fn now() -> Result<String> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    format_time(seconds).context("Export timestamp is outside the supported calendar range")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_dates_handle_leap_days_centuries_and_calendar_bounds() {
        for (seconds, expected) in [
            (0, "1970-01-01 00:00:00 UTC"),
            (951_868_799, "2000-02-29 23:59:59 UTC"),
            (1_791_967_500, "2026-10-14 08:45:00 UTC"),
            (4_107_542_400, "2100-03-01 00:00:00 UTC"),
            (253_402_300_799, "9999-12-31 23:59:59 UTC"),
        ] {
            assert_eq!(format_time(seconds).as_deref(), Some(expected));
        }
        assert!(format_time(253_402_300_800).is_none());
        assert!(format_time(u64::MAX).is_none());
    }

    #[test]
    fn legacy_creation_time_is_recovered_without_using_source_commit_dates() {
        let mut session = Session::new(Path::new("unused"));
        session.set("repository", "/sessions/1791967500123456789/repository");
        assert_eq!(
            session.created_at().unwrap(),
            "2026-10-14 08:45:00 UTC (from legacy session ID)"
        );
        session.set("repository", "/sessions/unrecognized/repository");
        assert_eq!(session.created_at().unwrap(), "unknown (older export)");
    }
}
