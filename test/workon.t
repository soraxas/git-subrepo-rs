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
  remote:   $UPSTREAM/bar [$DEFAULTBRANCH] (default push/pull)
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
  remote:   $UPSTREAM/bar [$DEFAULTBRANCH] (default push/pull)
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
  remote:   $UPSTREAM/bar [$DEFAULTBRANCH] (default push/pull)
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
  remote:   $UPSTREAM/bar [$DEFAULTBRANCH] (default push/pull)
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

done_testing

teardown
