#!/usr/bin/env bash

set -e

source test/setup

use Test::More

clone-foo-and-bar

subrepo-clone-bar-into-foo

(
  cd "$OWNER"/foo
  add-new-files bar/file
)

# The original project bar's remote was forked from — distinct from bar's own
# `remote` (which stays $UPSTREAM/bar, the subrepo's default push/pull target).
ORIGINAL_URL="$UPSTREAM/bar-original"
gitrepo=$OWNER/foo/bar/.gitrepo

PINNED=$(cd "$OWNER"/foo && git config -f bar/.gitrepo subrepo.commit)
PINNED_SHORT=${PINNED:0:7}

is "$(
  cd "$OWNER"/foo
  git subrepo config bar upstream "$ORIGINAL_URL" --force
)" \
  "Subrepo 'bar' option 'upstream' set to '$ORIGINAL_URL'." \
  "upstream config is set"

test-gitrepo-field "upstream" "$ORIGINAL_URL"

is "$(
  cd "$OWNER"/foo
  git subrepo workon bar --no-shell
)" \
  "Opened workon session for 'bar' at '.git/tmp/subrepo/bar'.
  remote:   $UPSTREAM/bar [$DEFAULTBRANCH] (default push/pull, pinned @ $PINNED_SHORT)
  upstream: $ORIGINAL_URL (added as remote 'upstream' — use e.g. \`git fetch upstream\`)
  Note: \`git subrepo push/pull\` on this subdir elsewhere rebuilds this worktree from mainline and discards anything not pushed from here yet." \
  "subrepo workon command output is correct"

test-exists "$OWNER"/foo/.git/tmp/subrepo/bar/

is "$(
  cd "$OWNER"/foo
  git config --get branch.subrepo/bar.remote
)" \
  "$UPSTREAM/bar" \
  "branch.<name>.remote (default push/pull) points at bar's own remote"

is "$(
  cd "$OWNER"/foo
  git config --get branch.subrepo/bar.merge
)" \
  "refs/heads/$DEFAULTBRANCH" \
  "branch.<name>.merge points at the tracked branch"

is "$(
  cd "$OWNER"/foo
  catch git config --get branch.subrepo/bar.pushRemote
)" \
  "" \
  "no pushRemote override — push always follows branch.<name>.remote"

is "$(
  cd "$OWNER"/foo
  git remote get-url upstream
)" \
  "$ORIGINAL_URL" \
  "'upstream' was added as a plain named remote for explicit use"

# Re-running with no new commits touching the subdir should reuse the
# existing worktree/branch rather than rebuilding it, and re-adding the
# 'upstream' remote must be idempotent (not error on the second call).
is "$(
  cd "$OWNER"/foo
  git subrepo workon bar --no-shell
)" \
  "Resumed workon session for 'bar' at '.git/tmp/subrepo/bar'.
  remote:   $UPSTREAM/bar [$DEFAULTBRANCH] (default push/pull, pinned @ $PINNED_SHORT)
  upstream: $ORIGINAL_URL (added as remote 'upstream' — use e.g. \`git fetch upstream\`)
  Note: \`git subrepo push/pull\` on this subdir elsewhere rebuilds this worktree from mainline and discards anything not pushed from here yet." \
  "second workon call reuses the existing worktree"

# If the worktree directory is deleted out from under git (e.g. a manual
# `rm -rf`) while the branch itself is untouched, workon must rebuild the
# worktree rather than blindly reporting "Resumed" for a directory that no
# longer exists.
rm -rf "$OWNER"/foo/.git/tmp/subrepo/bar

is "$(
  cd "$OWNER"/foo
  git subrepo workon bar --no-shell
)" \
  "Opened workon session for 'bar' at '.git/tmp/subrepo/bar'.
  remote:   $UPSTREAM/bar [$DEFAULTBRANCH] (default push/pull, pinned @ $PINNED_SHORT)
  upstream: $ORIGINAL_URL (added as remote 'upstream' — use e.g. \`git fetch upstream\`)
  Note: \`git subrepo push/pull\` on this subdir elsewhere rebuilds this worktree from mainline and discards anything not pushed from here yet." \
  "workon rebuilds the worktree if its directory was deleted, instead of reporting 'Resumed'"

test-exists "$OWNER"/foo/.git/tmp/subrepo/bar/

# Without a configured upstream, no 'upstream' remote should be added at all.
(
  cd "$OWNER"/foo
  git subrepo clean bar
  git remote remove upstream
  git config --file bar/.gitrepo --unset subrepo.upstream
)

is "$(
  cd "$OWNER"/foo
  git subrepo workon bar --no-shell
)" \
  "Opened workon session for 'bar' at '.git/tmp/subrepo/bar'.
  remote:   $UPSTREAM/bar [$DEFAULTBRANCH] (default push/pull, pinned @ $PINNED_SHORT)
  Note: \`git subrepo push/pull\` on this subdir elsewhere rebuilds this worktree from mainline and discards anything not pushed from here yet." \
  "workon without a configured upstream adds no 'upstream' remote"

is "$(
  cd "$OWNER"/foo
  catch git remote get-url upstream
)" \
  "error: No such remote 'upstream'" \
  "no 'upstream' remote exists when none is configured"

# A pre-existing 'upstream' remote pointing elsewhere (unrelated to this
# subrepo) must never be silently overwritten.
(
  cd "$OWNER"/foo
  git remote add upstream https://example.com/unrelated.git
  git subrepo config bar upstream "$ORIGINAL_URL" --force
)

is "$(
  cd "$OWNER"/foo
  git subrepo workon bar --no-shell 2>&1 >/dev/null
)" \
  "git-subrepo: a remote named 'upstream' already exists (→ 'https://example.com/unrelated.git'); not overwriting it. Fetch/push '$ORIGINAL_URL' directly by URL instead, or rename/remove the existing 'upstream' remote first." \
  "a conflicting pre-existing 'upstream' remote is not overwritten"

is "$(
  cd "$OWNER"/foo
  git remote get-url upstream
)" \
  "https://example.com/unrelated.git" \
  "the pre-existing unrelated 'upstream' remote is left untouched"

# `-F/--fetch` on a healthy, aligned remote should stay silent — no drift.
(
  cd "$OWNER"/foo
  git remote remove upstream
  git config --file bar/.gitrepo --unset subrepo.upstream
)

is "$(
  cd "$OWNER"/foo
  git subrepo workon bar --no-shell -F 2>&1 >/dev/null
)" \
  "" \
  "-F prints nothing when remote hasn't drifted from the pinned commit"

# If remote's branch is force-pushed/rebased past the pinned commit, `-F`
# must warn and point at `git subrepo pull`, rather than silently building a
# worktree whose reconstructed history no longer aligns with the real remote.
(
  cd "$OWNER"/bar
  git checkout -q --orphan rewritten-history
  git rm -qrf . 2>/dev/null
  echo "totally rewritten" >file
  git add file
  git commit -qm "rewritten history"
  git push -q --force "$UPSTREAM"/bar HEAD:"$DEFAULTBRANCH"
)
REMOTE_TIP_SHORT=$(cd "$OWNER"/bar && git rev-parse --short HEAD)

is "$(
  cd "$OWNER"/foo
  git subrepo workon bar --no-shell -F 2>&1 >/dev/null
)" \
  "⚠ remote's $DEFAULTBRANCH has moved past the pinned commit ($PINNED_SHORT) — it looks like it was rebased/force-pushed since (remote is now at $REMOTE_TIP_SHORT). Merging here will likely hit real conflicts from that rewrite.
  Recommended: run \`git subrepo pull bar\` first to re-sync." \
  "workon -F warns when remote has been rebased past the pinned commit"

# `workon` should automatically fetch `upstream`'s tracked branch (populating
# the normal `refs/remotes/upstream/<branch>` tracking ref) and tag the exact
# commit where local history diverges from it — no manual fetch/merge-base
# needed to see the boundary in a plain decorated `git log`.
#
# Build a real three-tier fixture (original project -> a fork with one extra
# local commit) so there's genuine shared ancestry to find, independent of
# `bar`'s now-rewritten state above.
(
  mkdir -p "$UPSTREAM"/original-src
  cd "$UPSTREAM"/original-src
  git init -q -b "$DEFAULTBRANCH"
  git config user.name "Original Author"
  git config user.email orig@example.com
  echo "v1" >lib.rs
  git add lib.rs
  git commit -qm "original: initial version"
  echo "v2" >>lib.rs
  git add lib.rs
  git commit -qm "original: add feature"
  git init -q --bare -b "$DEFAULTBRANCH" "$UPSTREAM"/original.git
  git push -q "$UPSTREAM"/original.git HEAD:"$DEFAULTBRANCH"

  git clone -q "$UPSTREAM"/original.git "$UPSTREAM"/fork2-src
  cd "$UPSTREAM"/fork2-src
  git config user.name "Me"
  git config user.email me@example.com
  echo "v3-mine" >>lib.rs
  git add lib.rs
  git commit -qm "me: local change"
  git init -q --bare -b "$DEFAULTBRANCH" "$UPSTREAM"/fork2.git
  git push -q "$UPSTREAM"/fork2.git HEAD:"$DEFAULTBRANCH"
)
DIVERGE_POINT=$(cd "$UPSTREAM"/original-src && git rev-parse HEAD)

(
  cd "$OWNER"/foo
  git subrepo clone "$UPSTREAM"/fork2.git bar2 -b "$DEFAULTBRANCH" >/dev/null
  git subrepo config bar2 upstream "$UPSTREAM"/original.git --force >/dev/null
)
BAR2_PINNED_SHORT=$(cd "$OWNER"/foo && git config -f bar2/.gitrepo subrepo.commit | cut -c1-7)

is "$(
  cd "$OWNER"/foo
  git subrepo workon bar2 --no-shell
)" \
  "Opened workon session for 'bar2' at '.git/tmp/subrepo/bar2'.
  remote:   $UPSTREAM/fork2.git [$DEFAULTBRANCH] (default push/pull, pinned @ $BAR2_PINNED_SHORT)
  upstream: $UPSTREAM/original.git (added as remote 'upstream' — use e.g. \`git fetch upstream\`)
  Local work diverges from upstream at tag 'bar2-upstream-base' — see it with \`git log\`.
  Note: \`git subrepo push/pull\` on this subdir elsewhere rebuilds this worktree from mainline and discards anything not pushed from here yet." \
  "workon auto-fetches upstream and reports the divergence tag in its banner"

is "$(
  cd "$OWNER"/foo/.git/tmp/subrepo/bar2
  git rev-parse upstream/"$DEFAULTBRANCH"
)" \
  "$DIVERGE_POINT" \
  "workon fetched 'upstream', populating the normal tracking ref at the original project's tip"

is "$(
  cd "$OWNER"/foo/.git/tmp/subrepo/bar2
  git rev-parse bar2-upstream-base
)" \
  "$DIVERGE_POINT" \
  "workon auto-tagged the exact commit where local history diverges from upstream"

is "$(
  cd "$OWNER"/foo/.git/tmp/subrepo/bar2
  git log --color=never --decorate --pretty=format:'%d' -1 bar2-upstream-base
)" \
  " (tag: bar2-upstream-base, upstream/$DEFAULTBRANCH)" \
  "the divergence tag and the upstream tracking ref both decorate the same commit in git log"

done_testing

teardown
