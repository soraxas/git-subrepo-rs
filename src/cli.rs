use usage::{Cli as UsageCli, Subcommands};

mod args;
mod compat;
mod completion;
pub use args::*;

#[derive(UsageCli)]
#[usage(
    bin = "git-subrepo",
    version = env!("CARGO_PKG_VERSION"),
    completion,
    unknown_flags = "error",
    args_override_self = false,
    disable_version_flag = true,
    disable_help_flag = true
)]
pub struct Cli {
    #[usage(subcommand)]
    pub command: Option<Commands>,

    /// Print the version number
    #[usage(long = "version", action = usage::ArgAction::SetTrue, global = true)]
    pub version: bool,

    /// Suppress output
    #[usage(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Show verbose output
    #[usage(short = 'v', long, global = true)]
    pub verbose: bool,

    /// Operate on all top-level subrepos
    #[usage(short = 'a', long, global = true)]
    pub all: bool,

    /// Operate on all subrepos and sub-subrepos
    #[usage(short = 'A', long = "ALL", global = true)]
    pub all_all: bool,

    /// Force operation
    #[usage(short = 'f', long, global = true)]
    pub force: bool,

    /// Fetch upstream before operation
    #[usage(short = 'F', long = "fetch", global = true)]
    pub fetch: bool,

    /// Use the auto-generated commit message without opening an editor
    #[usage(short = 'n', long = "no-edit", global = true)]
    pub no_edit: bool,

    /// Run git hooks during commit (default: hooks are bypassed with --no-verify)
    #[usage(short = 'V', long = "verify", global = true)]
    pub verify: bool,

    /// Print this help message
    #[usage(short = 'h', long, global = true, action = usage::ArgAction::Help)]
    pub help: bool,
}

#[derive(Subcommands)]
pub enum Commands {
    /// Generate a shell completion script
    Completion(CompletionArgs),
    /// Clone a remote into a subdirectory
    Clone(CloneArgs),
    /// Turn an existing directory into a subrepo
    Init(InitArgs),
    /// Pull upstream changes
    Pull(PullArgs),
    /// Push local commits upstream
    Push(PushArgs),
    /// Fetch upstream without merging
    Fetch(FetchArgs),
    /// Create a branch of local subrepo commits
    Branch(BranchArgs),
    /// Open a worktree for a subrepo with remotes wired up: bare `git push`/`pull`/
    /// `fetch` always reach `remote`, exactly as `git subrepo push`/`pull` already do.
    /// If `upstream` is configured (the original project, when `remote` is your own
    /// fork of it), it's added as a plain named `upstream` remote for explicit use
    /// (`git fetch upstream`, ...) — never the default. Pass `--fetch`/`-F` to also
    /// check (network required) whether `remote`/`upstream` have moved past the
    /// pinned commit in a way that looks like a rebase, before opening the worktree.
    Workon(WorkonArgs),
    /// Commit a merged subrepo branch into mainline
    Commit(CommitArgs),
    /// Show subrepo status
    Status(StatusArgs),
    /// Remove subrepo branches, refs and worktrees
    Clean(CleanArgs),
    /// Read or write subrepo configuration
    Config(ConfigArgs),
    /// Find subrepos sharing the same remote+branch but with diverged commits and sync them
    Sync,
    /// Scan all subrepos for issues (stale refs, rebase parent drift, missing fields) and offer to fix them
    Fix,
}

impl Cli {
    /// Handle informational requests before command dispatch or repository access.
    pub fn parse_process() -> Self {
        let mut args: Vec<_> = std::env::args_os().skip(1).collect();
        let words: Vec<_> = args.iter().map(|arg| arg.as_os_str()).collect();
        if let Some(answer) = completion::request(&args).or_else(|| Self::spec_request(&words)) {
            print!("{answer}");
            std::process::exit(0);
        }
        if let Err(message) = compat::prepare_args(&mut args) {
            eprintln!("git-subrepo: {message}");
            std::process::exit(1);
        }
        let words: Vec<_> = args.iter().map(|arg| arg.as_os_str()).collect();
        match Self::parse_from(&words) {
            Ok(cli) => cli,
            Err(usage::Error::Help { cmd, long }) => {
                print!(
                    "{}",
                    Self::render_help(cmd, long).expect("command has help metadata")
                );
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("git-subrepo: {}", Self::compatibility_error(&words, &error));
                std::process::exit(1);
            }
        }
    }

    fn compatibility_error(
        words: &[&std::ffi::OsStr],
        error: &usage::Error<'static, '_>,
    ) -> String {
        let command = words
            .iter()
            .find(|word| !word.as_encoded_bytes().starts_with(b"-"));
        let command = command
            .map(|word| word.to_string_lossy())
            .unwrap_or_default();
        match error {
            usage::Error::UnknownFlag { token } if token.starts_with(b"--") => {
                let name = token[2..]
                    .split(|byte| *byte == b'=')
                    .next()
                    .unwrap_or_default();
                format!("error: unknown option `{}'", String::from_utf8_lossy(name))
            }
            usage::Error::UnexpectedArg { token }
                if !Self::command()
                    .subcommands
                    .iter()
                    .any(|cmd| cmd.name == command)
                    && command != "help" =>
            {
                format!(
                    "'{}' is not a command. See 'git subrepo help'.",
                    String::from_utf8_lossy(token)
                )
            }
            usage::Error::MissingRequired { name }
                if name.eq_ignore_ascii_case("subdir") || name.eq_ignore_ascii_case("key") =>
            {
                format!(
                    "Command '{command}' requires arg '{}'.",
                    name.to_ascii_lowercase()
                )
            }
            usage::Error::MissingRequired { name }
                if name.eq_ignore_ascii_case("remote")
                    && command == "clone"
                    && words.iter().any(|word| {
                        ["--all", "-a", "--ALL", "-A"]
                            .iter()
                            .any(|flag| word == flag)
                    }) =>
            {
                "Invalid option '--all' for 'clone'.".to_string()
            }
            _ => Self::render_failure(words, error).trim_end().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use std::ffi::OsStr;

    fn parse<'a>(args: &[&'a str]) -> Result<Cli, usage::Error<'static, 'a>> {
        Cli::parse_from(&args.iter().copied().map(OsStr::new).collect::<Vec<_>>())
    }

    #[test]
    fn global_flags_and_command_specific_u_keep_their_meaning() {
        let cli = parse(&["-q", "pull", "--branch=main", "-u", "lib", "-AF"]).unwrap();
        assert!(cli.quiet && cli.all_all && cli.fetch);
        assert!(
            matches!(cli.command, Some(Commands::Pull(super::PullArgs { subdir: Some(subdir), branch: Some(branch), update: true, .. })) if subdir == "lib" && branch == "main")
        );
        let cli = parse(&[
            "workon",
            "-u",
            "https://example.com/project",
            "space dir",
            "--no-shell",
        ])
        .unwrap();
        assert!(
            matches!(cli.command, Some(Commands::Workon(super::WorkonArgs { subdir, upstream: Some(upstream), no_shell: true })) if subdir == "space dir" && upstream == "https://example.com/project")
        );
    }

    #[test]
    fn unknown_flags_and_duplicate_options_are_rejected_in_subcommands() {
        for args in [
            vec!["pull", "--unknown"],
            vec!["pull", "--branch", "main", "-b", "other"],
            vec!["pull", "-q", "-q"],
            vec!["config", "lib", "method", "merge", "--unknown"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn clone_preserves_trailing_arguments_for_legacy_error_reporting() {
        let cli = parse(&["clone", "remote", "lib", "extra", "--unknown"]).unwrap();
        assert!(
            matches!(cli.command, Some(Commands::Clone(super::CloneArgs { extra, .. })) if extra == ["extra", "--unknown"])
        );
    }

    #[test]
    fn double_dash_allows_a_subdir_that_looks_like_a_flag() {
        let cli = parse(&["pull", "--", "--directory"]).unwrap();
        assert!(
            matches!(cli.command, Some(Commands::Pull(super::PullArgs { subdir: Some(subdir), .. })) if subdir == "--directory")
        );
    }
}
