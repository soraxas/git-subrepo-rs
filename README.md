# git-subrepo-rs

A fast, concurrent, Rust-based reimplementation of [git-subrepo](https://github.com/ingydotnet/git-subrepo) — a Git submodule alternative that keeps your history clean and your workflow simple.

**Fully compatible** with the original `.gitrepo` format. Drop-in replacement, but with extra features.

## Why git-subrepo?

| | git-submodule | git-subtree | **git-subrepo** |
|---|:---:|:---:|:---:|
| Users don't need extra steps after clone | ✗ | ✓ | ✓ |
| Collaborators need the tool | ✗ | ✓ | only to push/pull |
| Clean main history | ✗ | ✗ | ✓ |
| Works across branches | ✗ | ✗ | ✓ |
| Move/rename subdir JustWorks™ | ✗ | ✓ | ✓ |
| Async parallel operations | ✗ | ✗ | ✓ |
| Interactive conflict resolution | ✗ | ✗ | ✓ |

### Key benefits

- **Users** just `git clone` your repo — subrepo content is already there, no extra steps.
- **Collaborators** see a `.gitrepo` file in any subrepo directory — it's self-documenting.
- **Owners** get a clean, linear history. Upstream changes are squashed to one commit.
- Branching, renaming, moving subdirs all Just Work™.
- Parallel fetch/pull across all subrepos (`-a`/`--all`).
- Interactive prompts for conflicts, merge strategy, and rebase recovery.

## Installation

```bash
cargo install --path .
# or
cargo build --release && cp target/release/git-subrepo ~/.local/bin/
```

Requires: `git` ≥ 2.36, Rust ≥ 1.91. Context detection uses Git's
NUL-delimited worktree listing to handle paths containing spaces or newlines.

For Fish tab completion, run this after installing the binary:

```fish
mkdir -p ~/.config/fish/completions
git subrepo completion fish > ~/.config/fish/completions/git-subrepo.fish
```

Restart Fish after installation. Completions support both `git subrepo` and
`git-subrepo`, including command options, tracked subrepo paths, and config keys.
Suggestions are generated from the Usage CLI definitions in Rust. Subrepo paths
come from the local Git index; completion never fetches or changes the repository.
The installed script calls the binary, so suggestions stay current after upgrades.
No separate Usage CLI installation is needed.

`git subrepo completion <shell>` also supports `bash`, `zsh`, `elvish`, `nushell`,
and `powershell`.

## Quick start

Run `git subrepo pr` in a terminal for guided PR actions. Use ↑/↓ and Enter to
choose a subrepo and action; Esc goes back. Open an existing export, review changes, or
prepare a contribution. Existing exports are found across the repository's
worktrees, so uncommitted source changes do not prevent resuming review work
prepared from another checkout. Every export shows its owning checkout, creation
time in UTC, source revision/branch, recorded exclusions and publication time.
The menu flags exclusions that differ from the current checkout. Opening resumes
the saved snapshot; new source or settings require preparing a new export.
Source files must be committed; root `.gitrepo` settings may remain uncommitted
and are recorded in the export. Close an active session in the same checkout
before replacing it. Closing retains the export repository and remote branch.

Opening an export starts your shell there; type `exit` to return to
the menu. Preparation prompts for a topic, public message and destinations.
Publishing displays the exact commit history/diff and asks for confirmation;
it does not run tests or an AI audit for you. Acceptance stages the import in
the export's owning checkout for review and a separate commit.

Bare `git subrepo` always prints help. Without a terminal, bare `git subrepo pr`
prints instructions instead of prompting. Explicit PR commands remain available
to scripts and agents.

```bash
# Add an external repo as a subrepo
git subrepo clone https://github.com/you/mylib lib/mylib

# Pull upstream changes
git subrepo pull lib/mylib

# Push your local changes back upstream
git subrepo push lib/mylib

# Status of all subrepos (fetches first, shows dirty info)
git subrepo status
```

## Commands

### Running from a subdirectory

Commands discover the current checkout root automatically. Explicit subrepo paths
resolve from your current directory, like ordinary filesystem paths:

```bash
cd lib/mylib
git subrepo config . exclude LOCAL_PATCHES.md
git subrepo pull
git subrepo status --no-fetch
```

`pull`, `push`, `fetch`, `branch`, `clean`, `status`, explicit `pr` operations and
`commit` infer the nearest enclosing tracked subrepo when their path is omitted.
`config` and `workon` accept `.` for the current subrepo. At the checkout root,
commands retain their existing explicit-path or all-subrepo behavior.

Interactive writes show the resolved checkout and target, then ask whether to
execute there, defaulting to no. Read operations do not prompt. `--yes` (or `-y`)
skips this location confirmation; it does not bypass publication, overwrite or
conflict checks. Scripts execute unambiguous cwd-relative paths without prompting.

For compatibility, a path such as `lib/mylib` entered from a nested directory can
fall back to that existing subrepo at the checkout root only when its cwd-relative
path does not exist. The CLI reports the fallback. Writes require terminal
confirmation or `--yes`; `--quiet` and `--force` do not approve it. An explicit
`./` or `../` path never falls back. If both locations exist, the cwd-relative
location wins. Paths outside the selected checkout, including external symlinks,
are rejected. Linked checkouts retain their own root and branch.

Clone destinations and explicit local remote paths also resolve from cwd. Named
Git remotes and remote URLs keep their usual meaning. Shell completion suggests
cwd-relative paths, including `.` for the current subrepo. Execution preserves
the caller's working directory; an import that would replace it with a file is
rejected before removing tracked content.

### `clone` — Add a subrepo

```text
git subrepo clone <url> [<subdir>] [-b <branch>] [-M <merge|rebase>] [-m <msg>]
```

Clones an external repo into `<subdir>`. The entire upstream history is squashed into one commit. A `.gitrepo` file is created in the subdir to track the remote, branch, and pinned commit.

If `<subdir>` is omitted, it is inferred from the URL (like `git clone`).

```bash
git subrepo clone https://github.com/org/lib          # → lib/
git subrepo clone https://github.com/org/lib ext/lib  # → ext/lib/
git subrepo clone https://github.com/org/lib -b dev   # track 'dev' branch
```

### `pull` — Update from upstream

```text
git subrepo pull [<subdir>] [-b <branch>] [-r <remote>] [-M <merge|rebase>] [-m <msg>]
```

Fetches the latest upstream commits and merges (or rebases) them with your local changes in the subdir. The result is squashed into a single commit on your mainline.

```bash
git subrepo pull lib/mylib             # pull for one subrepo
git subrepo pull -a                    # pull all subrepos in parallel
git subrepo pull lib/mylib -M rebase   # use rebase strategy
```

On merge conflict, the worktree is left open for manual resolution:

```text
Resolve conflicts in the worktree, then finalise with:
  cd '/path/to/.git/tmp/subrepos/lib/mylib'
  # fix conflicts, stage them, then:
  git merge --continue
  git subrepo commit lib/mylib
```

### `push` — Push local changes upstream

```text
git subrepo push [<subdir>] [-b <branch>] [-r <remote>] [-s] [-f]
```

Pushes the local subrepo branch back to the upstream remote. The branch must already include the upstream HEAD (i.e. you must pull before you can push, unless it's a first push after `init`).

```bash
git subrepo push lib/mylib
git subrepo push lib/mylib -b feature-branch   # push to a different branch
git subrepo push -a                             # push all subrepos
```

### `pr` — Export changes through a reviewed upstream PR

Use this workflow when contributing a vendored change through a pull request.
It keeps the accepted upstream sync baseline separate from an open PR and gives
review fixes a persistent standalone repository. This workflow requires Git 2.38
or newer.

```bash
git subrepo pr prepare lib/mylib --branch contribute/feature --message 'feat: add shared capability'
git subrepo pr status lib/mylib
# Review/test the printed repository; append review fixes there as normal commits.
git subrepo pr publish lib/mylib --expected-head <full-reviewed-sha>
# Create the PR explicitly with gh or GitHub; after it is merged:
git subrepo pr reconcile lib/mylib --merged <accepted-upstream-sha>
# Inspect/test the staged import, then commit it in the owning repository.
```

Preparation and publication do not change the owner's HEAD, index or `.gitrepo`.
Reconciliation stages the accepted result together with unexported and later
local changes. Both normal PR merges and content-equivalent squash merges are
supported. Root `.gitrepo` metadata and monorepo commit messages are excluded
from the initial export.

Use repeated `--path <literal-path>` arguments on prepare for a focused export;
paths are relative to the subrepo. `--base` selects the upstream target branch,
`--upstream` selects its fetch URL, and `--remote` selects a separate push URL
(for example a fork). Topic and base branches must differ. `pr close` closes
only the local session and retains its repository; it does not close a GitHub PR.

See [the PR export workflow](docs/pr-exports.md) for review fixes, conflict
recovery, authorization and merge policies. The reusable agent skill is
[upstream-export](.agents/skills/upstream-export/SKILL.md).

### `fetch` — Fetch upstream (without merging)

```text
git subrepo fetch [<subdir>] [-b <branch>] [-r <remote>]
```

Fetches upstream commits into `refs/subrepo/<subdir>/fetch`. Does not modify your working tree.

### `branch` — Create a local branch of subrepo commits

```text
git subrepo branch [<subdir>] [-f]
```

Scans mainline history and creates a branch `subrepo/<subdir>` containing only commits that touched `<subdir>`. Useful for manual merge/rebase workflows.

### `workon` — Enter a subrepo worktree with real remotes wired up

```text
git subrepo workon <subdir> [-u <upstream-url>] [--no-shell]
```

Opens the same kind of worktree as `branch` (at `.git/tmp/subrepo/<subdir>`,
on branch `subrepo/<subdir>`), but wires it up with real, named remotes so
`git log --decorate` shows exactly where things stand — nothing is left as
an opaque raw URL you can't see anywhere:

- Plain `git push`/`git pull`/`git fetch` (no arguments) always reach
  `remote` — exactly like `git subrepo push`/`pull` already do. This never
  changes, regardless of `upstream`. It's added as a real remote named
  `<subdir>-remote`, so `git log` shows `(<subdir>-remote/<branch>)`
  wherever it currently sits — e.g. right on `HEAD` if you're fully synced.
- If `upstream` is configured (via `-u/--upstream`, or a persisted
  `upstream` key — see below), it's *also* added as a remote named
  `<subdir>-upstream` — `git fetch <subdir>-upstream`,
  `git push <subdir>-upstream ...` — for explicit, deliberate use. It is
  never the default target for a bare push/pull.

Remote names are scoped per-subdir (`<subdir>-remote`/`<subdir>-upstream`,
not bare "remote"/"upstream") because remotes are repo-global: a second
subrepo's `workon` would otherwise collide with the first — most
concretely, a nested subrepo and its parent (e.g. `embodx` and
`embodx/crates/potree-rs`) both need their own. If that exact scoped name
is somehow already taken by something unrelated, `workon` leaves it alone
and falls back to a raw URL for `remote` — still fully functional, just
without the log decoration.

Use `upstream` when `remote` is your own fork of someone else's project,
and you occasionally want to pull in or push to the original directly, by
name, without disturbing the default `remote` target.

Whenever `upstream` is configured, `workon` also fetches its tracked branch
automatically and tags the exact commit where your local history diverges
from it — no manual `git fetch`/`git merge-base` needed to see the
boundary:

```text
$ git subrepo workon embodx/crates/potree-rs
...
  remote:   ...(default push/pull, pinned @ 8756900, as remote 'embodx/crates/potree-rs-remote' — see it with `git log`)
  Local work diverges from upstream at tag 'embodx/crates/potree-rs-upstream-base' — see it with `git log`.

$ git log --oneline --decorate
5b4257bb (HEAD -> subrepo/..., embodx/crates/potree-rs-remote/feat/converter) test: add CLI tests   <- fully synced with your fork
...
4da3014e (tag: embodx/crates/potree-rs-upstream-base) chore: fix other fns   <- the fork point, tagged
e4b15601 chore: rename `new` to `load`                                       <- upstream's own history
```

The tag stays put even if the real `upstream` remote moves further later;
re-running `workon` refreshes both the tracking ref and the tag together.

If `remote`'s tracked branch doesn't exist on `upstream` at all — common
when a feature branch was created only after forking — `workon` falls back
to `upstream`'s own default branch automatically.

On a terminal, `workon` drops you into a subshell with its working directory
set to the worktree; exit the shell to return. In scripts/CI (non-TTY), or
with `--no-shell`, it just prints the path.

```bash
git subrepo workon lib/mylib                                   # own repo, plain push/pull — no upstream needed
git subrepo workon lib/mylib -u git@github.com:someone/mylib   # your fork of someone else's repo
```

**Bring workon changes back into the main repository**: commits made inside
the workon worktree stay on its subrepo branch until you import them. Exiting
the shell alone does not update the main repository's files.

```bash
# Start in the main repository with a clean working tree.
git subrepo workon lib/mylib

# Inside the workon shell: edit files, then commit your changes.
git add -A
git commit -m "Improve mylib"

# Still inside workon: import those commits into the owning repository.
git subrepo commit

# Keep working here, or return to the main repository.
exit
```

Inside a workon tree or any of its subdirectories, `git subrepo commit`
detects the subrepo and owning checkout. It copies the current worktree HEAD
into the owner's subrepo directory, updates `.gitrepo`, and creates a single
commit on the owner's checked-out branch. This also works after switching
to a feature branch inside workon. It prints the destination checkout and
branch and preserves your workon tree so you can continue working.

Both checkouts must be clean, including untracked files: use ordinary
`git commit` to save your work first, or stash changes you are not importing.
Detection uses Git's worktree metadata and tracked `.gitrepo` files, so it
also works when you entered manually with `--no-shell`. If multiple
checkouts could own the subrepo, the command stops and asks you to run it
from the intended checkout instead.

You can also exit workon first and run `git subrepo commit lib/mylib` from
the main repository root. That form imports `subrepo/lib/mylib` and removes
the temporary worktree and subrepo branch after success.

No remote push is required for this local workflow. To publish the imported
changes afterward, run `git subrepo push lib/mylib` from the main repository.

Persist an upstream so you don't have to pass `-u` every time:

```bash
git subrepo config lib/mylib upstream git@github.com:someone/mylib --force
```

**Curating what goes public** (own repo with private "secret sauce"
commits mixed in): `workon` gives you the full local history for the
subdir, unfiltered — hand-pick what to publish with `git rebase -i` /
cherry-pick onto a clean branch, then push *that*. Secret commits stay
local; nothing is pushed unless you push it.

**Fork workflow, end to end**: someone else's project, with `remote` set to
your own fork and `upstream` set to the original project.

1. `git subrepo workon <subdir>` → make changes, plain `git push` (lands on
   your fork, `remote`, as always).
2. Open a PR from your fork to the original project by hand.
3. Once it's merged: still inside the `workon` session,
   `git fetch upstream && git merge upstream/<branch>` (or `git pull
   upstream <branch>`), then `git push` — pulls the merged result from the
   original project and pushes it to your own fork, keeping the two in sync.
4. Back in the main repo, `git subrepo pull <subdir>` re-fetches from
   `remote` (now up to date) and re-pins `.gitrepo` — this also correctly
   keeps any local-only commits you never pushed.

**Caution**: `git subrepo push`, `git subrepo pull`, and
`git subrepo branch --force` can rebuild the workon worktree from mainline
history. Before running them on the same subdir, commit your workon changes
and either import them with `git subrepo commit` as above or publish them
with plain `git push` from inside workon. Uncommitted changes and commits
that exist only in the workon branch can otherwise be lost.

**When `remote`/`upstream` gets rebased out from under you**: `workon`
always rebuilds faithfully from whatever `.gitrepo` currently has pinned —
if the real remote's branch has since been force-pushed/rebased, the
worktree's reconstructed history won't align with the remote's *current*
tip, and a merge there will surface confusing, large-scale conflicts from
that rewrite. This isn't something `workon` can safely auto-fix — that
requires actual human judgement, which is exactly what `git subrepo pull
<subdir>` is for (fetch, merge/rebase, resolve, re-pin `.gitrepo`). Run
`workon` with `-F/--fetch` to check for this *before* it drops you into the
worktree — it fetches `remote` (and `upstream`, if configured) and warns if
the pinned commit is no longer an ancestor of their current tip, pointing
you at `git subrepo pull` instead of guessing:

```bash
git subrepo workon lib/mylib -F
# ⚠ remote's main has moved past the pinned commit (8756900) — it looks like
#   it was rebased/force-pushed since (remote is now at a1b2c3d). Merging here
#   will likely hit real conflicts from that rewrite.
#   Recommended: run `git subrepo pull lib/mylib` first to re-sync.
```

### `commit` — Import workon changes or finalise a manual merge

```text
git subrepo commit [<subdir>] [<ref>] [-m <msg>]
```

Import committed subrepo content after working in `workon` or resolving a
manual merge. This updates the destination's `<subdir>` and `.gitrepo` and
creates a single commit there; it does not stage or commit unfinished edits
in the source worktree.

| Where you run it | Subrepo selection | Default source | Worktree cleanup |
|------------------|-------------------|----------------|------------------|
| Workon tree or a directory inside it | Inferred | Current worktree HEAD | Preserved |
| Inside a subrepo directory in the main checkout | Nearest enclosing tracked `.gitrepo` | `subrepo/<subdir>` | Removed after success |
| Main checkout root | Explicit `<subdir>` required | `subrepo/<subdir>` | Removed after success |

The destination working tree must be clean. When importing from workon,
both checkouts must be clean and the destination must have a branch checked
out. A bare import from inside a main-checkout subrepo also refuses to
remove a source worktree containing unfinished changes. If ownership is
ambiguous, exit workon and run the command from the
intended checkout root with an explicit subdir and source ref.

To select a different source from the main repository, supply both
`<subdir>` and `<ref>`:

```bash
git subrepo commit lib/mylib my-feature -m "Update mylib"
```

### `status` — Show subrepo state

```text
git subrepo status [<subdir>] [--no-fetch] [--no-dirty]
```

Shows the state of all subrepos. By default:

- Fetches upstream first (skip with `--no-fetch`)
- Shows unpushed local commits and new upstream commits (skip with `--no-dirty`)

Example output:

```text
Git subrepo 'lib/mylib':
  Remote URL:       https://github.com/org/mylib
  Tracking Branch:  main (a1b2c3d4) [remote HEAD: e5f6g7h8 (2 commits ahead)]
  Pull Parent:      deadbeef (0 commits ago)
  Unpushed:         3 unpushed commits  →  git subrepo push lib/mylib
                      a1b2c3d add feature
                      ...
  Upstream:         2 new upstream commits  →  git subrepo pull lib/mylib
                      e5f6g7h fix bug
                      ...
```

### `sync` — Sync diverged subrepos sharing the same remote

```text
git subrepo sync
```

Finds groups of subrepos that track the same remote URL and branch but have different pinned commits. Shows a diff of the content between them and offers to sync them up interactively.

### `fix` — Diagnose and repair subrepo issues

```text
git subrepo fix
```

Scans all top-level subrepos for common issues:

| Issue | Auto-fixable? |
|---|:---:|
| `parent=` not in HEAD history (caused by rebase) | ✓ |
| Missing required `.gitrepo` fields | ✗ |
| Stale `refs/subrepo/*/` refs pointing to missing objects | ✓ |
| Pinned `commit=` not reachable locally | ✗ |

Presents a multi-select prompt to choose which fixes to apply.

### `init` — Turn an existing subdir into a subrepo

```text
git subrepo init <subdir> [-r <remote>] [-b <branch>] [-M <merge|rebase>]
```

Promotes an existing subdirectory into a tracked subrepo. The content stays as-is; a `.gitrepo` file is added.

### `clean` — Remove subrepo artifacts

```text
git subrepo clean [<subdir>] [-f]
```

Removes temporary branches, refs, and remotes created by `fetch` and `branch`. Use `--force` to also remove refs. Use `-a` to clean all subrepos.

### `config` — Read/write `.gitrepo` values

```text
git subrepo config <subdir> <key> [<value>]
```

```bash
git subrepo config lib/mylib method rebase    # change merge strategy
git subrepo config lib/mylib parent <sha>     # manually repair parent after rebase
git subrepo config lib/mylib remote <url>     # update remote URL
```

## Global options

| Option | Short | Description |
|---|---|---|
| `--all` | `-a` | Operate on all top-level subrepos |
| `--ALL` | `-A` | Operate on all subrepos including nested |
| `--force` | `-f` | Force the operation (meaning varies per command) |
| `--quiet` | `-q` | Suppress output |
| `--verbose` | `-v` | More output |
| `--no-edit` | `-n` | Use auto-generated commit message (no editor) |
| `--verify` | `-V` | Run git hooks on commit (default: `--no-verify`) |
| `--yes` | `-y` | Confirm execution in the discovered checkout; other confirmations remain active |
| `--fetch` | `-F` | Fetch before operation |

## The `.gitrepo` file

Every subrepo directory contains a `.gitrepo` file that tracks its state:

```ini
; DO NOT EDIT (unless you know what you are doing)
;
; This subdirectory is a git "subrepo", and this file is maintained by the
; git-subrepo command. See https://github.com/soraxas/git-subrepo-rs
;
[subrepo]
    remote = https://github.com/org/mylib
    branch = main
    commit = a1b2c3d4e5f6...   # upstream commit currently pinned
    parent = deadbeef1234...   # main-repo HEAD at time of last pull
    method = merge
    cmdver = 0.0.1
    exclude = LOCAL_PATCHES.md
    exclude = docs/local-notes/
```

- `commit` — the upstream commit that is currently checked in
- `parent` — the main repo commit that was HEAD when this subrepo was last pulled/cloned. Used to find local-only commits when pushing.
- `method` — `merge` (default) or `rebase`
- `upstream` — optional; the original project's URL (when `remote` is your own fork of it). `git subrepo workon` adds it as a plain named `upstream` remote for explicit use — `remote` stays the default push/pull target either way. Not written by `clone`/`init`, only via `config`.
- `exclude` — optional and repeatable; literal file or directory paths relative
  to the subrepo root. Keep their local changes out of PR exports and direct
  subrepo pushes, and preserve local content on pull/import. A trailing `/` is
  accepted for directories; wildcard characters are literal. This is a
  git-subrepo-rs extension; other implementations may ignore it.

The `.gitrepo` file is committed to **your** repo but is **not** pushed to the upstream subrepo remote.

Configure one exclusion, or append several, then commit the metadata in your
owning repository:

```bash
git subrepo config lib/mylib exclude LOCAL_PATCHES.md
git config --file lib/mylib/.gitrepo --add subrepo.exclude docs/local-notes/
git subrepo config lib/mylib exclude
```

`git subrepo config ... exclude <path>` replaces the exclusion list. To remove
the list, use `git config --file lib/mylib/.gitrepo --unset-all subrepo.exclude`.
Excluding an existing upstream file preserves its upstream version; it does not
delete that file from the remote. New local-only files are omitted entirely.
The root `.gitrepo` is always omitted automatically. Nested `.gitrepo` files and
vendor patch ledgers remain included unless explicitly excluded. See
[export policy and session behavior](docs/pr-exports.md#persistent-exclusions).

## Nested subrepos

Subrepos can themselves contain subrepos. For example:

```text
app/
  ├── lib/barlib/         ← subrepo of app
  │     └── baz/bazlib/  ← subrepo of barlib (nested)
```

**Important**: there is no special handling at the `app` level for `barlib`'s nested subrepos. From `app`'s perspective, `lib/barlib/baz/bazlib/` is just regular files inside `lib/barlib/`.

To upstream changes to a nested subrepo:

1. `git subrepo push lib/barlib` — push your changes to barlib's upstream
2. In your local clone of `barlib`, `git pull` then `git subrepo push baz/bazlib`

When pulling a nested subrepo directly (e.g. `git subrepo pull lib/barlib/baz/bazlib`), a warning is shown because the `parent=` field will reference the main repo's HEAD, not barlib's commit history. Use `git subrepo fix` if this causes issues after a rebase.

## Conflict resolution

When a pull or push fails due to merge conflicts, the subrepo worktree is left open at:

```text
.git/tmp/subrepos/<subdir>/
```

Resolve conflicts there, then:

```bash
cd .git/tmp/subrepos/lib/mylib
# edit conflicting files
git add .
git merge --continue        # or: git rebase --continue
git subrepo commit lib/mylib
```

## Rebase recovery

If you rebase your main branch, the `parent=` SHA stored in `.gitrepo` may no longer be in HEAD history. When this happens, `git subrepo pull` will:

1. Detect the problem and show a diagnosis
2. Search for the correct new parent (via `.gitrepo` log history)
3. Show how many commits behind HEAD the candidate is, and how many subdir files differ
4. Prompt: **Auto-repair** / **Force** / **Abort**

You can also run `git subrepo fix` to scan all subrepos for this and other issues at once.

## Differences from the original bash `git-subrepo`

| Feature | bash git-subrepo | git-subrepo-rs |
|---|---|---|
| Implementation | Bash | Rust |
| Parallel `--all` operations | Sequential | Async parallel (DFS-ordered) |
| Rebase parent recovery | Manual | Interactive auto-repair |
| Merge conflict UX | Error + instructions | Worktree left open + step-by-step |
| `sync` command | ✗ | ✓ |
| `fix` command | ✗ | ✓ |
| `.gitrepo` format | Same | Same (fully compatible) |
| `cmdver` field | `0.4.x` | `0.0.x` |
