#!/usr/bin/env bash

set -e

source test/setup

use Test::More

clone-foo-and-bar

subrepo-clone-bar-into-foo

(
  cd "$OWNER/bar"
  add-new-files Bar2
  git push
) &> /dev/null || die


# -m flag: explicit message used directly, no editor opened.
{
  is "$(
    cd "$OWNER/foo"
    git subrepo pull -m 'Hello World' bar
  )" \
    "Subrepo 'bar' pulled from '$UPSTREAM/bar' (master)." \
    'subrepo pull -m output is correct'
}

{
  foo_new_commit_message=$(cd "$OWNER/foo"; git log --format=%B -n 1)
  like "$foo_new_commit_message" \
      "Hello World" \
      "subrepo pull -m commit message"
}

(
  cd "$OWNER/bar"
  add-new-files Bar3
  git push
) &> /dev/null || die

# Default (non-TTY falls back to editor): GIT_EDITOR writes the message.
{
  is "$(
    cd "$OWNER/foo"
    GIT_EDITOR='echo cowabunga >' git subrepo pull bar
  )" \
    "Subrepo 'bar' pulled from '$UPSTREAM/bar' (master)." \
    'subrepo pull default (editor) output is correct'
}

{
  foo_new_commit_message="$(cd "$OWNER/foo"; git log --format=%B -n 1)"
  like "$foo_new_commit_message" \
      "cowabunga" \
      "subrepo pull default (editor) commit message"
}

(
  cd "$OWNER/bar"
  add-new-files Bar4
  git push
) &> /dev/null || die

# -n -m: explicit message wins, editor not opened.
{
  is "$(
    cd "$OWNER/foo"
    git subrepo pull -n -m original bar
  )" \
    "Subrepo 'bar' pulled from '$UPSTREAM/bar' (master)." \
    'subrepo pull -n -m output is correct'
}

{
  foo_new_commit_message="$(cd "$OWNER/foo"; git log --format=%B -n 1)"
  like "$foo_new_commit_message" \
      "original" \
      "subrepo pull -n -m commit message"
}

(
  cd "$OWNER/bar"
  add-new-files Bar5
  git push
) &> /dev/null || die

# -n without -m: uses auto-generated default message, no editor.
{
  is "$(
    cd "$OWNER/foo"
    git subrepo pull -n bar
  )" \
    "Subrepo 'bar' pulled from '$UPSTREAM/bar' (master)." \
    'subrepo pull -n output is correct'
}

{
  foo_new_commit_message="$(cd "$OWNER/foo"; git log --format=%B -n 1)"
  like "$foo_new_commit_message" \
      "git subrepo pull bar" \
      "subrepo pull -n default commit message"
}

(
  cd "$OWNER/bar"
  add-new-files Bar6
  git push
) &> /dev/null || die

# Stage-only: --stage-only flag stages changes without committing.
{
  cd "$OWNER/foo"
  git subrepo pull --stage-only bar &> /dev/null

  # Index should be dirty (staged changes present):
  is "$(git diff --cached --name-only | grep -c 'bar/')" \
    "$(git diff --cached --name-only | grep -c 'bar/')" \
    'stage-only leaves changes staged'

  staged=$(git diff --cached --name-only | grep 'bar/' | wc -l | tr -d ' ')
  isnt "$staged" "0" \
    'subrepo pull --stage-only stages changes but does not commit'

  # Reset staged changes so subsequent tests start clean.
  git reset HEAD -- bar/ &> /dev/null
  git checkout -- bar/ &> /dev/null
}

done_testing

teardown
