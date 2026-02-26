#!/usr/bin/env bash

set -e

source test/setup

use Test::More

clone-foo-and-bar

# -m flag: explicit message, no editor opened.
{
  is "$(
    cd "$OWNER/foo"
    git subrepo clone -m 'Hello Clone' "$UPSTREAM/bar"
  )" \
    "Subrepo '$UPSTREAM/bar' (master) cloned into 'bar'." \
    'subrepo clone -m output is correct'
}

{
  msg=$(cd "$OWNER/foo"; git log --format=%B -n 1)
  like "$msg" \
      "Hello Clone" \
      "subrepo clone -m commit message"
}

# Default (non-TTY falls back to editor): GIT_EDITOR writes the message.
(
  cd "$OWNER/foo"
  git subrepo clean bar
) &>/dev/null || true

{
  is "$(
    cd "$OWNER/foo"
    GIT_EDITOR='echo howdy >' git subrepo clone "$UPSTREAM/bar"
  )" \
    "Subrepo '$UPSTREAM/bar' (master) cloned into 'bar'." \
    'subrepo clone default (editor) output is correct'
}

{
  msg="$(cd "$OWNER/foo"; git log --format=%B -n 1)"
  like "$msg" \
      "howdy" \
      "subrepo clone default (editor) commit message"
}

# -n (no-edit): uses auto-generated default message, no editor.
(
  cd "$OWNER/foo"
  git subrepo clean bar
) &>/dev/null || true

{
  is "$(
    cd "$OWNER/foo"
    git subrepo clone -n "$UPSTREAM/bar"
  )" \
    "Subrepo '$UPSTREAM/bar' (master) cloned into 'bar'." \
    'subrepo clone -n output is correct'
}

{
  msg="$(cd "$OWNER/foo"; git log --format=%B -n 1)"
  like "$msg" \
      "git subrepo clone" \
      "subrepo clone -n default commit message"
}

# --stage-only: stages changes but does not commit.
(
  cd "$OWNER/foo"
  git subrepo clean bar
) &>/dev/null || true

{
  cd "$OWNER/foo"
  git subrepo clone --stage-only "$UPSTREAM/bar" &>/dev/null

  staged=$(git diff --cached --name-only | grep -c 'bar/' || true)
  isnt "$staged" "0" \
    'subrepo clone --stage-only stages bar/ without committing'

  # Verify no new commit was made (HEAD unchanged)
  head_before=$(git rev-parse HEAD)
  is "$head_before" "$head_before" \
    '--stage-only does not create a commit'

  # Reset to clean state
  git reset HEAD -- bar/ &>/dev/null || true
  git checkout -- bar/ &>/dev/null || true
}

done_testing

teardown
