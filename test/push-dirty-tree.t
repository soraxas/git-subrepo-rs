#!/usr/bin/env bash

set -e

source test/setup

use Test::More

clone-foo-and-bar

(
  cd "$OWNER/foo"
  git subrepo clone "$UPSTREAM/bar"
  add-new-files bar/FooBar
  add-new-files Staged
) &> /dev/null || die

# Changes inside the pushed subrepo would not be pushed, so they block
{
  (
    cd "$OWNER/foo"
    echo 'not committed' >> bar/Bar
  )
  push_status=0
  push_output=$(
    cd "$OWNER/foo"
    git subrepo push bar 2>&1
  ) || push_status=$?

  isnt "$push_status" "0" \
    'subrepo push fails with uncommitted changes inside the subrepo'
  like "$push_output" "'bar' has uncommitted changes that would not be pushed" \
    'failure explains that subrepo changes would not be pushed'
  (
    cd "$OWNER/foo"
    git checkout -- bar/Bar
  )
}

# Unstaged and staged changes outside the subrepo do not block, and stay put
{
  (
    cd "$OWNER/foo"
    echo 'unstaged' >> Foo
    echo 'staged' >> Staged
    git add Staged
  )
  push_status=0
  push_output=$(
    cd "$OWNER/foo"
    git subrepo push bar 2>&1
  ) || push_status=$?

  is "$push_status" "0" \
    'subrepo push succeeds with changes outside the subrepo'
  is "$push_output" \
    "Subrepo 'bar' pushed to '$UPSTREAM/bar' (master)." \
    'push output is correct'
  is "$(cd "$OWNER/foo"; git show --name-only --format= HEAD)" \
    "bar/.gitrepo" \
    'push commit contains only bar/.gitrepo'
  is "$(cd "$OWNER/foo"; git diff --cached --name-only)" \
    "Staged" \
    'staged change outside the subrepo stays staged'
  is "$(cd "$OWNER/foo"; git diff --name-only)" \
    "Foo" \
    'unstaged change outside the subrepo stays unstaged'
}

done_testing
teardown
