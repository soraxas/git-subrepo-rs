use std::process::{Command, Stdio};
use usage::complete::{Candidate, CompleteCtx};

pub(super) fn request(args: &[std::ffi::OsString]) -> Option<String> {
    use usage::complete::{CompletionRequest, complete, render_request, walk};

    let request = CompletionRequest::parse(args)?;
    let position = walk(super::Cli::command(), request.split.argv());
    if request.candidates_for.is_none()
        && position.cmd.name == "workon"
        && position.next_arg.is_some()
        && position.awaiting_value.is_none()
        && !(position.flags_possible && request.split.prefix.starts_with('-'))
    {
        let mut answer = complete(super::Cli::spec(), &request.split);
        // Usage 6.12 falls back to files when a dynamic callback has no
        // matches. Workon accepts only existing subrepos at this position.
        answer.files = None;
        Some(render_request(&answer, &request))
    } else {
        super::Cli::completion_request(args)
    }
}

/// Completion is read-only and local. Git failures (including no repository)
/// simply yield no candidates and must never interrupt the shell prompt.
fn git_output(args: &[&str]) -> Vec<u8> {
    Command::new("git")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .unwrap_or_default()
}

fn subrepo_paths(files: &[u8]) -> Vec<String> {
    let mut paths: Vec<_> = files
        .split(|byte| *byte == 0)
        .filter_map(|file| std::str::from_utf8(file).ok())
        .filter_map(|file| file.strip_suffix("/.gitrepo"))
        // Shell completion protocols use tabs/newlines as record separators.
        .filter(|path| !path.contains(['\t', '\n', '\r']))
        .map(str::to_owned)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn subrepos<T>(_: &T, _: &CompleteCtx<'_>) -> Vec<Candidate<'static>> {
    subrepo_paths(&git_output(&[
        "ls-files",
        "--full-name",
        "-z",
        "--",
        ":(top,glob)**/.gitrepo",
    ]))
    .into_iter()
    .map(|path| Candidate::described(path, "Subrepo"))
    .collect()
}

pub(super) fn methods<T>(_: &T, _: &CompleteCtx<'_>) -> Vec<Candidate<'static>> {
    ["merge", "rebase"]
        .into_iter()
        .map(Candidate::new)
        .collect()
}

pub(super) fn config_keys<T>(_: &T, _: &CompleteCtx<'_>) -> Vec<Candidate<'static>> {
    crate::commands::config::VALID_KEYS
        .iter()
        .map(|key| Candidate::new(*key))
        .collect()
}

pub(super) fn config_values(
    _: &<super::ConfigArgs as usage::spec::CommandArgs>::Partial,
    ctx: &CompleteCtx<'_>,
) -> Vec<Candidate<'static>> {
    use usage::spec::CommandArgs;

    // Usage 6.12 builds a callback's partial from the child command alone,
    // which does not know inherited flags such as `config -f`. Walk the full
    // CLI tables so global flags never get mistaken for config positionals.
    let words: Vec<_> = ctx
        .words
        .get(1..ctx.cword)
        .unwrap_or_default()
        .iter()
        .map(std::ffi::OsStr::new)
        .collect();
    let mut parser = usage::Parser::new(super::Cli::command(), &words);
    let mut partial = super::ConfigArgs::start();
    while let Some(Ok(event)) = parser.next_event() {
        let _ = super::ConfigArgs::apply(&mut partial, &event);
    }
    match partial.key.as_slice() {
        b"method" => methods(&partial, ctx),
        _ => Vec::new(),
    }
}

pub(super) fn refs<T>(_: &T, _: &CompleteCtx<'_>) -> Vec<Candidate<'static>> {
    String::from_utf8_lossy(&git_output(&[
        "for-each-ref",
        "--format=%(refname:short)",
        "refs/heads",
        "refs/tags",
        "refs/subrepo",
    ]))
    .lines()
    .map(|name| Candidate::described(name, "Commit ref"))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::subrepo_paths;

    #[test]
    fn paths_preserve_spaces_and_nested_subrepos_and_remove_duplicates() {
        assert_eq!(
            subrepo_paths(
                b"space dir/.gitrepo\0lib/a/.gitrepo\0lib/a/nested/.gitrepo\0lib/a/.gitrepo\0"
            ),
            ["lib/a", "lib/a/nested", "space dir"]
        );
    }

    #[test]
    fn paths_ignore_empty_non_state_and_unrepresentable_entries() {
        assert!(
            subrepo_paths(b"\0.gitrepo\0lib/file\0bad\tname/.gitrepo\0\xff/.gitrepo\0").is_empty()
        );
        assert!(subrepo_paths(b"").is_empty());
    }
}
