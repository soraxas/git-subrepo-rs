#!/usr/bin/env bash

set -e

source test/setup

use Test::More

clone-foo-and-bar
subrepo-clone-bar-into-foo

# Push a new upstream commit via $OWNER/bar so pull has something to fetch.
{
  (
    cd "$OWNER/bar"
    add-new-files Bar2
    git push
  ) &> /dev/null || die
}

# Set verify=true in bar/.gitrepo and commit it (bypassing hooks; hook not yet installed).
{
  (
    cd "$OWNER/foo"
    git config --file="bar/.gitrepo" subrepo.verify true
    git add bar/.gitrepo
    git commit --no-verify -m "set verify=true in bar/.gitrepo"
  ) &> /dev/null || die
}

# Now install a pre-commit hook that always fails.
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

# Pull with verify=true from .gitrepo: hooks run → should fail.
{
  pull_status=0
  pull_output=$(
    cd "$OWNER/foo"
    git subrepo pull -n bar 2>&1
  ) || pull_status=$?

  isnt "$pull_status" "0" \
    'subrepo pull respects verify=true from .gitrepo'
  like "$pull_output" "Pre-commit hook triggered and failing" \
    'pre-commit hook message seen when verify=true in .gitrepo'
}

# Reset and set verify=false — pull should bypass hooks.
{
  (
    cd "$OWNER/foo"
    git reset --hard HEAD~1 &>/dev/null || true  # undo the verify=true commit
    git config --file="bar/.gitrepo" subrepo.verify false
  ) &>/dev/null || true
}

{
  pull_status=0
  pull_output=$(
    cd "$OWNER/foo"
    git subrepo pull -n bar 2>&1
  ) || pull_status=$?

  unlike "$pull_output" "Pre-commit hook triggered" \
    'pre-commit hook bypassed when verify=false in .gitrepo'
}

done_testing
teardown
