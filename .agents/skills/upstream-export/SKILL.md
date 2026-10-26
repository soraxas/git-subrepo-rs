---
name: upstream-export
description: Audit, prepare, review, publish, and reconcile upstream pull requests from git-subrepo-managed code, including public history cleanup and reviewer fixes. Use for exporting vendored changes or auditing a prepared upstream PR; not for ordinary application edits.
---

# Upstream export

Use the custom `git-subrepo-rs` PR session commands. Check `git subrepo help pr`
first; if unavailable, locate the source/build and report the mismatch. Never
substitute direct `push -b`: it advances sync metadata before PR acceptance.
Follow the owning repository's shell conventions and authorization rules.

## Choose the requested operation

- **Audit:** read-only inspection and findings; no extraction regeneration.
- **Prepare:** local candidate, standalone checks and public PR draft. No push.
- **Publish:** push the reviewed SHA and create/update the explicitly authorized PR.
- **Reconcile:** stage accepted upstream content plus retained local work. Commit
  only when authorized; preparation/publication do not authorize merging the PR.

Establish checkout ownership. Inspect branch/HEAD, working and staged changes,
`.gitrepo`, existing worktrees and `pr status`. Preserve existing work. Resolve
the source path, canonical upstream URL/base, push URL/topic and intended scope
from context; ask only when a material choice remains unknown.

## Prepare and audit

`pr prepare` takes committed source, an explicit public `--message`, and a
`--branch` distinct from `--base`. Use repeated literal `--path` selections for
focused exports; `--remote` can select a fork and `--upstream` its canonical base.
Inspect/test the returned standalone repository. Keep private audit records
outside the exported tree. Do not edit root `.gitrepo` manually.

Record: source revision, imported/upstream base, selected scope, candidate SHA,
destination/base/topic, findings, checks/results and runtime limitations.

Audit **every new reachable commit**, including messages, trailers, deleted
content and binaries. Check private names/URLs/paths/secrets, external workspace
dependencies, fixtures, public API/feature/binary removals, examples and documented
commands. Preserve legitimate attribution/licenses; flag changes instead of
automatically deleting them. Keyword matches require contextual review.

Run appropriate standalone checks against the exact candidate. A downstream
build or one passing library target cannot establish all exported features work.

## Review and publication

Append review fixes in the same candidate repository. Review changes since the
published tip, inspect the complete candidate, rerun affected checks and record
the new SHA. Never regenerate extraction over reviewer commits. Fetch/review
remote edits before adopting them: fetch the explicit topic, then use
`git merge --ff-only FETCH_HEAD` before appending local follow-ups. Candidate
history must stay linear; do not silently force-push divergence.

When authorized, `pr publish <subdir> --expected-head <full-reviewed-sha>` sends
that revision. Use explicit `gh pr create --repo ... --base ... --head ...
--title ... --body-file ...` arguments and a reviewed public body. Draft PRs are
publication too. Reuse an existing PR instead of creating duplicates.

Recommend a normal merge to preserve export identity, or squash merge for a
short-lived PR with explicit reconciliation. Neither replaces pre-push auditing.

## Accepted result

Verify the actual accepted revision and CI, then `pr reconcile <subdir> --merged
<full-sha>`. Review/test the staged owner import; preserve later and unexported
local work. Resolve conflicts in the reported integration repository and rerun.
Commit only when authorized, then start the next export from the new baseline.
Verify its diff contains only residual/new work, without replaying accepted changes.
`pr close` closes only local session tracking and retains its repository.

Report findings first, then changed files, inspected verification, remaining
limits and exact Git/publication state. Never equate pushed, PR-open, merged and
reconciled states.
