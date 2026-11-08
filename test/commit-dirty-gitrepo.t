#!/usr/bin/env bash

set -e

source test/setup

use Test::More

clone-foo-and-bar

subrepo-clone-bar-into-foo

# A local subrepo commit to import, made in the subrepo branch worktree
(
  cd "$OWNER/foo"
  git subrepo branch bar
  cd .git/tmp/subrepo/bar
  add-new-files Local
  # Some upstreams carry their own .gitrepo; the import must still succeed
  add-new-files .gitrepo
) &> /dev/null || die

gitrepo=$OWNER/foo/bar/.gitrepo
fixed_parent=$(cd "$OWNER/foo"; git rev-parse HEAD)

# An unstaged parent= edit, as left behind by `git subrepo fix`
(
  cd "$OWNER/foo"
  git config --file bar/.gitrepo subrepo.parent "$fixed_parent"
)

# A staged .gitrepo edit still blocks
{
  (cd "$OWNER/foo"; git add bar/.gitrepo)
  commit_status=0
  commit_output=$(
    cd "$OWNER/foo"
    git subrepo commit bar 2>&1
  ) || commit_status=$?

  isnt "$commit_status" "0" \
    'subrepo commit fails with a staged .gitrepo edit'
  is "$commit_output" \
    "git-subrepo: Can't commit subrepo. Index has changes." \
    'staged .gitrepo edit is reported as index changes'
  (cd "$OWNER/foo"; git reset -q -- bar/.gitrepo)
}

# A failed import restores the subrepo instead of leaving it deleted
{
  (cd "$OWNER/foo"; echo 'in the way' > bar/Local)
  commit_status=0
  (
    cd "$OWNER/foo"
    git subrepo commit bar
  ) &> /dev/null || commit_status=$?

  isnt "$commit_status" "0" \
    'subrepo commit fails when an untracked file blocks the import'
  is "$(cd "$OWNER/foo"; git status --porcelain)" \
    " M bar/.gitrepo
?? bar/Local" \
    'failed import leaves the subrepo files and the .gitrepo edit in place'
  rm "$OWNER/foo/bar/Local"
}

# An unstaged .gitrepo edit is allowed and survives the import
{
  commit_status=0
  commit_output=$(
    cd "$OWNER/foo"
    git subrepo commit bar 2>&1
  ) || commit_status=$?

  is "$commit_status" "0" \
    'subrepo commit succeeds with an unstaged .gitrepo edit'
  like "$commit_output" "committed as subdir 'bar/'" \
    'subrepo commit output is correct'

  test-exists "$OWNER/foo/bar/Local"
  test-gitrepo-field "parent" "$fixed_parent"
  is "$(cd "$OWNER/foo"; git show HEAD:bar/.gitrepo | git config --file - subrepo.parent)" \
    "$fixed_parent" \
    'committed .gitrepo keeps the edited parent'
  is "$(cd "$OWNER/foo"; git status --porcelain)" \
    "" \
    'owning checkout is clean after the commit'
}

done_testing
teardown
