#!/usr/bin/env bash

set -e

source test/setup

use Test::More

{
  # Check the Rust binary is on PATH and runs
  git subrepo --version &>/dev/null
  pass 'git-subrepo binary is present and runs'

  # Git intercepts `git subrepo --help` to open a man page; -h reaches the CLI.
  git subrepo -h &>/dev/null
  pass 'git-subrepo -h succeeds'
}

done_testing 2

teardown
