use super::completion;

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct CompletionArgs {
    #[usage(choices("bash", "elvish", "fish", "nushell", "powershell", "zsh"))]
    pub shell: String,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct CloneArgs {
    #[usage(value_hint = usage::ValueHint::DirPath)]
    pub remote: String,
    #[usage(value_hint = usage::ValueHint::DirPath)]
    pub subdir: Option<String>,
    #[usage(short = 'b', long, value_hint = usage::ValueHint::Other)]
    pub branch: Option<String>,
    #[usage(short = 'M', long, complete = completion::methods)]
    pub method: Option<String>,
    #[usage(short = 'm', long, value_hint = usage::ValueHint::Other)]
    pub message: Option<String>,
    /// Stage changes but do not commit (skip commit step entirely)
    #[usage(long)]
    pub stage_only: bool,
    #[usage(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub extra: Vec<String>,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct InitArgs {
    #[usage(value_hint = usage::ValueHint::DirPath)]
    pub subdir: String,
    #[usage(short = 'r', long, value_hint = usage::ValueHint::Url)]
    pub remote: Option<String>,
    #[usage(short = 'b', long, value_hint = usage::ValueHint::Other)]
    pub branch: Option<String>,
    #[usage(short = 'M', long, complete = completion::methods)]
    pub method: Option<String>,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct PullArgs {
    #[usage(complete = completion::subrepos)]
    pub subdir: Option<String>,
    #[usage(short = 'b', long, value_hint = usage::ValueHint::Other)]
    pub branch: Option<String>,
    #[usage(short = 'r', long, value_hint = usage::ValueHint::Url)]
    pub remote: Option<String>,
    #[usage(short = 'M', long, complete = completion::methods)]
    pub method: Option<String>,
    #[usage(short = 'u', long)]
    pub update: bool,
    #[usage(short = 'm', long, value_hint = usage::ValueHint::Other)]
    pub message: Option<String>,
    /// Stage changes but do not commit (skip commit step entirely)
    #[usage(long)]
    pub stage_only: bool,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct PushArgs {
    #[usage(complete = completion::subrepos)]
    pub subdir: Option<String>,
    #[usage(short = 'b', long, value_hint = usage::ValueHint::Other)]
    pub branch: Option<String>,
    #[usage(short = 'r', long, value_hint = usage::ValueHint::Url)]
    pub remote: Option<String>,
    #[usage(short = 'M', long, complete = completion::methods)]
    pub method: Option<String>,
    #[usage(short = 's', long)]
    pub squash: bool,
    #[usage(short = 'u', long)]
    pub update: bool,
    #[usage(short = 'm', long, value_hint = usage::ValueHint::Other)]
    pub message: Option<String>,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct FetchArgs {
    #[usage(complete = completion::subrepos)]
    pub subdir: Option<String>,
    #[usage(short = 'b', long, value_hint = usage::ValueHint::Other)]
    pub branch: Option<String>,
    #[usage(short = 'r', long, value_hint = usage::ValueHint::Url)]
    pub remote: Option<String>,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct PrArgs {
    #[usage(choices("prepare", "status", "publish", "reconcile", "close"))]
    pub action: Option<String>,
    #[usage(complete = completion::subrepos)]
    pub subdir: Option<String>,
    /// Topic branch to publish; must differ from the upstream base branch
    #[usage(long)]
    pub branch: Option<String>,
    /// Upstream target branch (default: tracked .gitrepo branch)
    #[usage(long)]
    pub base: Option<String>,
    /// Push URL, such as a fork (default: tracked remote)
    #[usage(long)]
    pub remote: Option<String>,
    /// Fetch URL to reconcile from (default: tracked remote)
    #[usage(long)]
    pub upstream: Option<String>,
    /// Public export commit message, required for prepare
    #[usage(short = 'm', long)]
    pub message: Option<String>,
    /// Literal file/directory inside the subrepo to export; repeat for multiple paths
    #[usage(long)]
    pub path: Vec<String>,
    /// Full reviewed candidate SHA, required for publish
    #[usage(long)]
    pub expected_head: Option<String>,
    /// Accepted upstream commit SHA, required for reconcile
    #[usage(long)]
    pub merged: Option<String>,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct BranchArgs {
    #[usage(complete = completion::subrepos)]
    pub subdir: Option<String>,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct WorkonArgs {
    #[usage(complete = completion::subrepos)]
    pub subdir: String,
    /// The original project's URL. Overrides .gitrepo's `upstream` key for this run.
    #[usage(short = 'u', long, value_hint = usage::ValueHint::Url)]
    pub upstream: Option<String>,
    /// Don't spawn an interactive shell; just print the worktree path.
    #[usage(long)]
    pub no_shell: bool,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct CommitArgs {
    /// Subrepo to import; inferred inside a subrepo directory or workon tree
    #[usage(complete = completion::subrepos)]
    pub subdir: Option<String>,
    #[usage(complete = completion::refs)]
    pub subrepo_commit_ref: Option<String>,
    #[usage(short = 'm', long, value_hint = usage::ValueHint::Other)]
    pub message: Option<String>,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct StatusArgs {
    #[usage(complete = completion::subrepos)]
    pub subdir: Option<String>,
    /// Skip showing unpushed commits and upstream diff (shown by default)
    #[usage(long)]
    pub no_dirty: bool,
    /// Skip fetching upstream before showing status (fetch is done by default)
    #[usage(long)]
    pub no_fetch: bool,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct CleanArgs {
    #[usage(complete = completion::subrepos)]
    pub subdir: Option<String>,
}

#[derive(usage::Args)]
#[usage(args_override_self = false)]
pub struct ConfigArgs {
    #[usage(complete = completion::subrepos)]
    pub subdir: String,
    #[usage(complete = completion::config_keys)]
    pub key: String,
    #[usage(complete = completion::config_values)]
    pub value: Option<String>,
}
