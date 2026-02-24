#!/usr/bin/env bash

set -e

source test/setup

use Test::More

{
  # Check the Rust binary is on PATH and runs
  git subrepo --version &>/dev/null
  pass 'git-subrepo binary is present and runs'

  # Check --help works
  git subrepo --help &>/dev/null
  pass 'git-subrepo --help succeeds'
}

done_testing 2

teardown
