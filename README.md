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

Requires: `git` ≥ 2.23, Rust ≥ 1.91.

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
and `powershell`. To regenerate the bundled Fish script during development:

```bash
cargo run -- completion fish > etc/git-subrepo.fish
```

## Quick start

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

**Caution**: `workon`'s worktree is not protected from `push`/`pull`/
`branch --force` run elsewhere — those rebuild it from mainline history and
will discard anything you haven't pushed yet. Finish and push from the
`workon` session before running those on the same subdir.

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

### `commit` — Finalise a manual merge

```text
git subrepo commit <subdir> [<ref>] [-m <msg>]
```

After resolving a conflict by hand, use this to take the HEAD of `subrepo/<subdir>`, put its content into the subdir, and add a single commit to your mainline.

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
```

- `commit` — the upstream commit that is currently checked in
- `parent` — the main repo commit that was HEAD when this subrepo was last pulled/cloned. Used to find local-only commits when pushing.
- `method` — `merge` (default) or `rebase`
- `upstream` — optional; the original project's URL (when `remote` is your own fork of it). `git subrepo workon` adds it as a plain named `upstream` remote for explicit use — `remote` stays the default push/pull target either way. Not written by `clone`/`init`, only via `config`.

The `.gitrepo` file is committed to **your** repo but is **not** pushed to the upstream subrepo remote.

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
