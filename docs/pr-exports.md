# Reviewed upstream exports

`git subrepo pr` separates a proposed upstream contribution from the accepted
sync baseline. Existing `push` remains the direct-sync command; do not use
`push -b topic` as a substitute, because it repins `.gitrepo` after publication.

## Guided menu

In a terminal, run `git subrepo pr`. Use ↑/↓ and Enter to select; Esc goes back.
Bare `git subrepo` still prints help. Choose the subrepo; if an
export already exists, **Open existing export** comes first. The menu searches
registered owner worktrees and identifies each export's owner and destination.
It also works from inside a retained export. Opening a checkout starts your
shell there; `exit` returns to the menu without rebuilding the checkout.

**Prepare new contribution** prompts for an editable topic, public message,
upstream/base and push destination. It exports committed source only. If the
current subtree is dirty, the menu shows the changed files and existing exports;
review/commit the intended source changes explicitly, or resume an existing
export. It does not silently stash, commit, or switch to another source checkout.

**Publish reviewed export** displays the candidate history and diff, destination
and revision. Confirm only after reviewing and testing that revision; confirmation
defaults to no. Both the SHA and selected session/destination are checked before
publication. Updating another session while confirmation is open cannot redirect
the push. This publishes a branch; creating the GitHub PR is still a separate step.

After acceptance, **Bring accepted upstream changes back** asks for the merged
commit from the PR and stages reconciliation in its displayed owning checkout.
Review and commit that import separately. Closing or abandoning local tracking
remains available through `pr close`; it retains the export and does not close
a remote PR. Commands below remain available for automation and detailed control.

Interactive Unix regression tests use Python 3's standard-library PTY support,
real local Git fixtures, and no network. The application has no new dependencies.

## Prepare

Run from the owning checkout root with committed changes in the subrepo:

```bash
git subrepo pr prepare lib/mylib \
  --branch contribute/robot-viewer \
  --message 'feat: add standalone robot viewer'
```

The command prints a standalone repository path. It snapshots the committed
subtree, excludes its root `.gitrepo`, and creates one public commit based on
upstream history. It does not reuse/rebuild a `workon` checkout or copy monorepo
ancestry. It fetches the upstream base and incorporates independent upstream
changes before review. Git 2.38 or newer is required for `merge-tree --write-tree`.

Preparation creates local export commits; it does not commit the owner, push,
create a PR, or grant an agent permission to perform those external actions.
Respect any additional owning-repository authorization requirements.

Optional arguments:

- `--path src/viewer --path tests/viewer`: select literal paths relative to the
  subtree. Excluded local changes remain available after reconciliation.
- `--base main`: select the accepted upstream branch, defaulting to `.gitrepo`.
- `--upstream <url>`: fetch from a different upstream repository.
- `--remote <fork-url>`: publish to a fork while keeping the upstream base separate.

Use path selection during preparation. Removing initially selected changes
from the PR later is a change to the proposal: reconciliation brings that removal
back too. If the intention is to defer changes locally, close the session and
prepare a narrower selection instead. Split within-file changes in the source
before preparation; path selection does not interactively select hunks.

One session can be active per subrepo per owning checkout. Records and isolated
repositories live under that checkout's Git directory, not the exported tree.
They include source revision, imported revision, selected proposal, upstream
base, topic/push destination, published tip and reconciliation state. They are
local records, not team-wide PR tracking. `pr close` allows a new session while
retaining the previous repository and its archived session record. It never
deletes a branch or closes a remote PR.

If preparation conflicts with upstream, resolve in the printed repository and
run `git cherry-pick --continue` there. The owning checkout is unchanged. For a
non-conflict failure, inspect the underlying Git error before retrying; close
the session and prepare again if no cherry-pick is in progress.

## Persistent exclusions

Store owner-only paths as repeated `exclude` entries in the source `.gitrepo`:

```ini
[subrepo]
    exclude = LOCAL_PATCHES.md
    exclude = docs/local-notes/
```

Paths are literal and relative to the subtree root. Files and whole directories
are supported, including spaces and wildcard characters in their actual names.
Absolute paths, traversal, `.git` components, empty values and pathspec magic
are rejected. No glob or negation syntax is interpreted. Root `.gitrepo` is
always omitted; nested metadata and similarly named vendor ledgers are retained.

The guided preparation screen and normal `pr prepare` output display the
configured exclusions. They also apply when using explicit `--path` selections;
an exclusion wins if a path is both selected and excluded. Excluded local
changes are replaced with their imported upstream baseline, or omitted if they
did not exist upstream. This does not delete files already published upstream.
If nothing exportable remains, preparation reports no changes.

The session snapshots its policy; `pr status` lists it. Publication checks every
outgoing commit against that session's upstream base and rejects changes to
excluded paths, even if removed again before the tip. Changing the source policy
does not change an existing session: close it and prepare again to use the new
policy. Manual `git push` from an export bypasses these checks.

Direct `git subrepo push` also filters excluded paths from every outgoing commit,
including the first push after `init`. Existing upstream history is preserved.
The owner's files remain intact. Pull/import retains the owner's committed
excluded content and deletions; PR reconciliation preserves the union of its
recorded exclusions and the owner's current policy. If an upstream change to
an ancestor conflicts structurally with a retained path, resolve the structural
conflict explicitly rather than removing local content.

These settings apply to git-subrepo-rs exports, not ordinary pushes of the owning
repository. Commit messages and other non-excluded files still need review.
Other git-subrepo implementations may ignore this extension.

## Audit and publish

Audit the full candidate against the recorded upstream base: behavior, public
APIs, examples, dependencies, fixtures, documentation, licensing/attribution and
every new commit's metadata and file content. Removed-at-tip content can still
exist in earlier commits. The tool checks root `.gitrepo` and configured exclusions in each outgoing commit
and requires a linear public series; it is not a branding or secret scanner.

Run the library's standalone checks in the candidate repository. Record the
candidate SHA, commands/results and runtime limitations outside the exported
source. Passing downstream application checks does not establish standalone
compatibility. Private audit records must not be placed in the PR description.

```bash
git subrepo pr status lib/mylib
git subrepo pr publish lib/mylib --expected-head <full-reviewed-sha>
```

Publish verifies a clean candidate and sends exactly that SHA to the recorded
topic branch. It never rebuilds the extraction and never repins `.gitrepo`.
Subsequent updates must append to published history. An explicit lease protects
against concurrent changes to the destination; it is not permission to rewrite
the branch. Initial publication refuses an unrelated existing branch.

After publication is authorized, create the PR separately with explicit
repository/base/head and a reviewed body file, for example:

```bash
gh pr create --repo OWNER/LIBRARY --base main \
  --head FORK_OWNER:contribute/robot-viewer \
  --title 'Add standalone robot viewer' --body-file /path/to/public-pr.md
```

PR creation and branch push are external publication, including draft PRs. This
workflow does not imply permission to merge, deploy, or publish packages.

## Review fixes

Append fixes as ordinary commits in the same standalone repository. Review the
delta since the last published tip, check the complete resulting candidate, and
rerun affected tests. Publish again with the new full `--expected-head`.
The previous verification applies to the previous SHA; it is not approval of new
commits. Never regenerate the export over the only copy of review fixes.

If somebody else updates the remote topic branch, fetch it explicitly in the
candidate repository and review it. Fast-forward to the reviewed remote tip;
append any additional fixes locally, then publish the newly reviewed SHA.
Diverged histories need deliberate resolution;
the tool does not silently force-push them.

Default recommendation: clean up the initial export, append meaningful review
fixes, then use GitHub's normal merge commit. This retains reviewed commit
identities. Squash merge is also supported for short-lived PRs; always reconcile
the actual accepted revision and start a fresh export for the next PR. GitHub's
rebase merge is not a dedicated acceptance mode; it is accepted only when the
same ancestry or content-equivalence checks can prove the resulting contribution.

## Reconcile accepted code

```bash
git subrepo pr reconcile lib/mylib --merged <full-accepted-sha>
git diff --cached
# Run affected downstream checks, then commit the staged import when authorized.
```

The candidate must be clean and equal to its recorded published tip. The supplied
commit must be reachable from the configured upstream base branch. For a normal
merge it must contain the published tip. For a squash merge, the accepted tree
must equal the reviewed candidate merged with the accepted commit's first parent.
Closing a PR or supplying an unrelated upstream SHA is insufficient.

Reconciliation uses three trees: the initially selected proposal, today's local
subtree, and accepted upstream content. Review fixes and concurrent upstream
changes arrive; unselected local changes and later local development survive.
Overlapping changes require resolution. It refuses a changed local sync baseline
or an owner branch that no longer descends from the original source revision.

Conflicts are materialized in a separate integration repository. Resolve and
stage the files there, then repeat the same reconcile command. The owner must
remain at the recorded local revision while resolving; if it changes, preserve
the resolution before closing/restarting the session.

On success, the owner receives a staged import and updated `.gitrepo`, including
the selected accepted upstream URL/base branch and commit. The owner HEAD is
unchanged until the user commits. Inspect and test before committing. The PR
repository and local record are retained; a subsequent prepare is allowed after
the staged import is committed. Start the next PR from that new sync baseline.

## Verification contract

Tests exercise real temporary Git remotes without GitHub/network access. Required
round-trip coverage includes normal and squash merges, review-fix commits,
upstream movement, path-scoped exports, retained local changes, binary files,
metadata in intermediate history, stale reviewed heads and conflict recovery.
A second export must contain only residual/new changes. Live GitHub permissions,
branch protections and required CI checks remain the repository's responsibility.
