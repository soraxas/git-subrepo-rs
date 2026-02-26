#!/usr/bin/env bash

set -e

source test/setup

use Test::More

clone-foo-and-bar

# Create a pre-commit hook that always fails
{
  hook_file="$OWNER/foo/.git/hooks/pre-commit"
  mkdir -p "$(dirname "$hook_file")"
  cat > "$hook_file" <<'HOOK'
#!/bin/bash
echo "Pre-commit hook triggered and failing"
exit 1
HOOK
  chmod +x "$hook_file"
}

# Default (no -V): clone bypasses hooks with --no-verify → should succeed
{
  clone_status=0
  clone_output=$(
    cd "$OWNER/foo"
    git subrepo clone -n "$UPSTREAM/bar" 2>&1
  ) || clone_status=$?

  is "$clone_status" "0" \
    'subrepo clone succeeds by default (hooks bypassed)'
  unlike "$clone_output" "Pre-commit hook triggered" \
    'pre-commit hook was bypassed by default'
  is "$clone_output" \
    "Subrepo '$UPSTREAM/bar' (master) cloned into 'bar'." \
    'clone output is correct'
}

{
  test-exists \
    "$OWNER/foo/bar/" \
    "$OWNER/foo/bar/Bar" \
    "$OWNER/foo/bar/.gitrepo"
}

# Clean up for next test
{
  (
    cd "$OWNER/foo"
    git reset --hard HEAD~1
    rm -rf bar
  ) &> /dev/null || true
}

# With -V (--verify): hooks run → commit should fail
{
  clone_status=0
  clone_output=$(
    cd "$OWNER/foo"
    git subrepo clone -V -n "$UPSTREAM/bar" 2>&1
  ) || clone_status=$?

  isnt "$clone_status" "0" \
    'subrepo clone with --verify fails when pre-commit hook fails'
  like "$clone_output" "Pre-commit hook triggered and failing" \
    'pre-commit hook was triggered with --verify'
}

done_testing
teardown
