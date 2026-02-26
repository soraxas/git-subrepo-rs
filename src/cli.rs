use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "git-subrepo",
    disable_version_flag = true,
    disable_help_flag = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(long = "version", action = clap::ArgAction::SetTrue, global = true)]
    pub version: bool,

    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    #[arg(short = 'a', long, global = true)]
    pub all: bool,

    /// Operate on all subrepos and sub-subrepos
    #[arg(short = 'A', long = "ALL", global = true)]
    pub all_all: bool,

    /// Force operation
    #[arg(short = 'f', long, global = true)]
    pub force: bool,

    /// Fetch upstream before operation
    #[arg(short = 'F', long = "fetch", global = true)]
    pub fetch: bool,

    /// Use the auto-generated commit message without opening an editor
    #[arg(short = 'n', long = "no-edit", global = true)]
    pub no_edit: bool,

    /// Print this help message
    #[arg(short = 'h', long, global = true, action = clap::ArgAction::Help)]
    pub help: Option<bool>,
}

#[derive(Subcommand)]
pub enum Commands {
    Clone {
        remote: String,
        subdir: Option<String>,
        #[arg(short = 'b', long)]
        branch: Option<String>,
        #[arg(short = 'M', long)]
        method: Option<String>,
        #[arg(short = 'q', long)]
        quiet: bool,
        #[arg(short = 'm', long)]
        message: Option<String>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        extra: Vec<String>,
    },
    Init {
        subdir: String,
        #[arg(short = 'r', long)]
        remote: Option<String>,
        #[arg(short = 'b', long)]
        branch: Option<String>,
        #[arg(short = 'M', long)]
        method: Option<String>,
    },
    Pull {
        subdir: Option<String>,
        #[arg(short = 'b', long)]
        branch: Option<String>,
        #[arg(short = 'r', long)]
        remote: Option<String>,
        #[arg(short = 'M', long)]
        method: Option<String>,
        #[arg(short = 'q', long)]
        quiet: bool,
        #[arg(short = 'u', long)]
        update: bool,
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
    Push {
        subdir: Option<String>,
        #[arg(short = 'b', long)]
        branch: Option<String>,
        #[arg(short = 'r', long)]
        remote: Option<String>,
        #[arg(short = 'M', long)]
        method: Option<String>,
        #[arg(short = 's', long)]
        squash: bool,
        #[arg(short = 'q', long)]
        quiet: bool,
        #[arg(short = 'u', long)]
        update: bool,
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
    Fetch {
        subdir: Option<String>,
        #[arg(short = 'b', long)]
        branch: Option<String>,
        #[arg(short = 'r', long)]
        remote: Option<String>,
        #[arg(short = 'q', long)]
        quiet: bool,
    },
    Branch {
        subdir: Option<String>,
        #[arg(short = 'q', long)]
        quiet: bool,
    },
    Commit {
        subdir: String,
        subrepo_commit_ref: Option<String>,
        #[arg(short = 'q', long)]
        quiet: bool,
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
    Status {
        subdir: Option<String>,
        #[arg(short = 'q', long)]
        quiet: bool,
        #[arg(short = 'v', long)]
        verbose: bool,
        /// Show unpushed commit count per subrepo (commits in main repo touching the subdir since last pull)
        #[arg(short = 'd', long)]
        dirty: bool,
    },
    Clean {
        subdir: Option<String>,
        #[arg(short = 'q', long)]
        quiet: bool,
    },
    Config {
        subdir: String,
        key: String,
        value: Option<String>,
    },
}
