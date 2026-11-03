#!/usr/bin/env bash

set -e

source test/setup

use Test::More

clone-foo-and-bar

(
  cd "$OWNER/foo"
  git subrepo clone "$UPSTREAM/bar"
  add-new-files bar/FooBar
) &> /dev/null || die

# A pre-push hook in the owning checkout that rejects every push
{
  hook_file="$OWNER/foo/.git/hooks/pre-push"
  cat > "$hook_file" <<'HOOK'
#!/bin/bash
echo "Pre-push hook triggered and failing" >&2
exit 1
HOOK
  chmod +x "$hook_file"
}

# Default: the hook runs, and the failure names it
{
  push_status=0
  push_output=$(
    cd "$OWNER/foo"
    git subrepo push bar 2>&1
  ) || push_status=$?

  isnt "$push_status" "0" \
    'subrepo push fails when the pre-push hook fails'
  like "$push_output" "Pre-push hook triggered and failing" \
    'pre-push hook output is shown'
  like "$push_output" "stopped by the owning checkout's pre-push hook" \
    'failure names the owning checkout pre-push hook'
  like "$push_output" "rerun with --no-verify" \
    'failure suggests --no-verify'
}

# --no-verify: the hook is skipped and the push succeeds
{
  push_status=0
  push_output=$(
    cd "$OWNER/foo"
    git subrepo push bar --no-verify 2>&1
  ) || push_status=$?

  is "$push_status" "0" \
    'subrepo push --no-verify succeeds despite the failing hook'
  unlike "$push_output" "Pre-push hook triggered" \
    'pre-push hook was skipped with --no-verify'
  is "$push_output" \
    "Subrepo 'bar' pushed to '$UPSTREAM/bar' (master)." \
    'push output is correct'
}

done_testing
teardown
