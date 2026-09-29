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
    #[usage(complete = completion::subrepos)]
    pub subdir: String,
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
