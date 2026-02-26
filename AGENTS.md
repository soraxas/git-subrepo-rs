# git-subrepo-rs — Agent Guide

## Project Overview

This is a Rust rewrite of [git-subrepo](https://github.com/ingydotnet/git-subrepo), a Git subcommand that provides a clean alternative to `git submodules` and `git subtree`.

The binary is named `git-subrepo` (installed as a `git` subcommand callable via `git subrepo <command>`).

Full compatibility with the original git-subrepo `.gitrepo` state file format is required.

---

## Architecture

```text
src/
├── main.rs           — Entry point: parse CLI, build tokio runtime, dispatch
├── cli.rs            — clap derive-based CLI definitions
├── error.rs          — SubrepoError (thiserror) + SubrepoResult
├── context.rs        — SubrepoContext struct (repo, subdir, options)
├── gitrepo.rs        — .gitrepo file reading / writing (via git2::Config)
├── encode.rs         — subdir → subref encoding (git ref-safe encoding)
├── git_utils.rs      — libgit2 + git CLI helpers
├── progress.rs       — indicatif progress bar helpers
├── scheduler.rs      — DFS async scheduler for --all operations
└── commands/
    ├── mod.rs
    ├── clone.rs
    ├── init.rs
    ├── pull.rs
    ├── push.rs
    ├── fetch.rs
    ├── branch_cmd.rs
    ├── commit_cmd.rs
    ├── status.rs
    ├── clean.rs
    └── config.rs
```

---

## Crate Choices

| Purpose | Crate |
|---------|-------|
| CLI parsing | `clap` (derive) |
| Git operations | `git2` (libgit2 bindings) |
| Async runtime | `tokio` (full features) |
| Progress bars | `indicatif` |
| Colored output | `colored` |
| Internal errors | `thiserror` |
| User-facing errors | `anyhow` |

---

## Commands

All commands follow `git subrepo <command> [<subdir>] [options]`.

### Core Commands

| Command | Description |
|---------|-------------|
| `clone <url> [<subdir>]` | Clone remote into a local subdirectory |
| `init <subdir>` | Turn existing subdir into a subrepo |
| `pull <subdir>` | Pull upstream changes into subdir |
| `push <subdir>` | Push local subrepo commits upstream |
| `fetch <subdir>` | Fetch upstream without merging |
| `branch <subdir>` | Create branch containing local subrepo commits |
| `commit <subdir>` | Commit a merged subrepo branch back to mainline |
| `status [<subdir>]` | Show subrepo status |
| `clean <subdir>` | Remove branches/refs/worktrees for subrepo |
| `config <subdir> <key> [<value>]` | Read/write .gitrepo config values |

### Global Options

| Flag | Description |
|------|-------------|
| `-a, --all` | Operate on all subrepos |
| `-A, --ALL` | Operate on all subrepos including sub-subrepos |
| `-b, --branch <branch>` | Specify upstream branch |
| `-r, --remote <url>` | Specify upstream remote URL |
| `-m, --message <msg>` | Custom commit message |
| `--file <path>` | Commit message from file |
| `-e, --edit` | Open editor for commit message |
| `-f, --force` | Force certain operations |
| `-F, --fetch` | Fetch before the command |
| `-M, --method <merge\|rebase>` | Join method (default: merge) |
| `-s, --squash` | Squash commits on push |
| `-u, --update` | Save --branch/--remote overrides to .gitrepo |
| `-q, --quiet` | Minimal output |
| `-v, --verbose` | Verbose output |

---

## `.gitrepo` File Format

The state file lives at `<subdir>/.gitrepo`. It is a git-config file with a specific comment header.

```ini
; DO NOT EDIT (unless you know what you are doing)
;
; This subdirectory is a git "subrepo", and this file is maintained by the
; git-subrepo command. See https://github.com/ingydotnet/git-subrepo#readme
;
[subrepo]
	remote = git@github.com:user/repo.git
	branch = master
	commit = <upstream HEAD commit sha>
	parent = <local commit sha before this operation>
	method = merge
	cmdver = 0.1.0
```

Fields:

- `remote` — upstream URL
- `branch` — upstream branch being tracked
- `commit` — upstream HEAD commit at last clone/pull/push
- `parent` — local commit (in mainline repo) just before the last subrepo operation
- `method` — join method: `merge` (default) or `rebase`
- `cmdver` — version of git-subrepo that last modified this file

---

## Git Refs Structure

For a subrepo at path `foo/bar` (subref = `foo/bar`):

| Ref | Purpose |
|-----|---------|
| `refs/subrepo/foo/bar/fetch` | Points to last fetched upstream HEAD |
| `refs/subrepo/foo/bar/branch` | Points to HEAD of last created subrepo branch |
| `refs/subrepo/foo/bar/commit` | Points to last merged upstream commit |
| `refs/subrepo/foo/bar/push` | Points to last pushed commit |

---

## Worktree Convention

During branch/pull/push operations, a git worktree is created at:

```text
$(git rev-parse --git-common-dir)/tmp/subrepo/<subdir>
```

This worktree is cleaned up after a successful commit operation.

---

## Async DFS Scheduling (--all)

When `--all` is specified, the command runs on all subrepos. To avoid conflicts, subrepos are processed in a dependency-respecting order:

**Rule**: A parent subrepo (e.g., `lib/`) must be processed AFTER all its child subrepos (e.g., `lib/ui/`, `lib/core/`) are complete.

**Algorithm**:

1. Collect all subrepos from `git ls-files | grep '\.gitrepo$'`
2. Build a DAG: subrepo `s` depends on all subrepos whose path starts with `s/`
3. Process using tokio `JoinSet` — each generation of independent subrepos runs in parallel
4. Wait for each generation before starting dependent parents

With `--ALL`, sub-subrepos (subrepos inside subrepos) are also included. Without `--ALL`, only top-level subrepos are processed.

---

## Implementation Guidelines

### Error Handling

- Use `SubrepoError` (thiserror) for internal logic errors with structured variants
- Use `anyhow::Result` / `anyhow::Context` for user-facing error propagation
- Print errors to stderr prefixed with `git-subrepo:` (matching bash version)

### Git Operations Split

- **libgit2** (`git2` crate): repo opening, rev walking, tree manipulation, index operations, commit creation, ref management, status checks
- **git CLI** (`std::process::Command` or `tokio::process::Command`): `git fetch`, `git push`, `git worktree add/prune`, `git ls-remote --symref` (for default branch detection)

Reason: git CLI handles credential helpers automatically; libgit2 needs explicit callback setup. For local test repos this doesn't matter, but for real usage git CLI is more user-friendly.

### Output Format

All user-visible output must match the bash version exactly (the test suite checks this):

- Success: `"Subrepo '<subdir>' pulled from '<remote>' (<branch>)."`
- Up to date: `"Subrepo '<subdir>' is up to date."`
- Error: printed to stderr with `git-subrepo: <message>`
- Use `colored` crate for colored output in TTY contexts

### Commit Messages

Commit messages follow this format:

```text
git subrepo <command> <subdir>

subrepo:
  subdir:   "<subdir>"
  merged:   "<short-sha>"
upstream:
  origin:   "<remote>"
  branch:   "<branch>"
  commit:   "<short-sha>"
git-subrepo:
  version:  "<VERSION>"
```

### Version

Binary version is stored in `Cargo.toml`. The `--version` flag outputs just the version number (e.g., `0.1.0`).

---

## Testing

There are two complementary test layers:

### 1. Rust unit tests (in `src/`)

Pure-function logic is covered by `#[cfg(test)] mod tests { … }` blocks inside the relevant
source files. No git repo is required. Run with:

```bash
cargo test
```

When adding new pure helpers (parsers, formatters, decision logic), extract them into a
standalone function and add a `#[test]` for each interesting case (empty input, only-current,
mixed, edge cases). This keeps the integration tests lean.

### 2. Integration / TAP tests (in `test/`)

These use the `prove` TAP runner with `Test::More` from bash+.
Tests call `git subrepo <command>` which invokes the compiled Rust binary.

### Build before testing

```bash
cargo build
prove test/
```

### Run specific test

```bash
cargo build && prove test/clone.t
```

### Individual test (verbose)

```bash
cargo build && bash test/clone.t
```

### Run cargo tests (unit + integration)

```bash
cargo nextest run
```

### Test setup (`test/setup`)

- Adds `target/debug/` to front of `$PATH` so `git subrepo` finds the Rust binary
- Sets up temporary git config (user.name, user.email, etc.)
- Creates `$TMP/upstream`, `$TMP/owner`, `$TMP/collab` directories
- Initializes bare test repos from `test/repo/{foo,bar,init}`

---

## `subrepo:branch` Algorithm

This is the most complex operation. It creates a branch containing only local commits to the subrepo subdir, rebased onto upstream history.

```text
Inputs: subdir, subrepo_parent, refs/subrepo/<subref>/fetch

1. Walk commits from subrepo_parent..HEAD in ancestry-path + topo order (reversed)
2. For each commit:
   a. Try to read <commit>:<subdir>/.gitrepo as a blob
   b. If missing: skip (commit doesn't touch subrepo)
   c. Parse subrepo.commit from that blob → gitrepo_commit
   d. Check if this commit is a direct child of the previous ancestor (single path)
   e. Get the tree at <commit>:<subdir>/
   f. Remove .gitrepo from that tree → create new tree object
   g. Build parent list: [prev_commit (if exists), gitrepo_commit (merge method)]
   h. Create new commit preserving author info (committer = current user)
3. Set prev_commit = last created commit
4. Create branch `subrepo/<subref>` at prev_commit
5. Create worktree at $GIT_TMP/subrepo/<subdir>
6. Create refs/subrepo/<subref>/branch → branch HEAD
```

When `subrepo_parent` is not set (first push after init):

- Filter all commits in the branch by subdir using subdirectory-filter logic
- Remove .gitrepo from each commit

---

## `subrepo:commit` Algorithm

```text
Inputs: subrepo_commit_ref (branch or commit sha), upstream_head_commit

1. Verify subrepo_commit_ref exists
2. Verify upstream_head_commit is in rev-list of subrepo_commit_ref (unless --force)
3. Remove all subdir/** entries from git index
4. Delete subdir/** files from working tree
5. Read tree from subrepo_commit_ref into index at prefix <subdir>/
6. Checkout those files to working tree
7. Write/update <subdir>/.gitrepo with new state
8. Stage <subdir>/.gitrepo
9. git commit with formatted message
10. Create refs/subrepo/<subref>/commit → subrepo_commit_ref
11. Clean up worktree
```
