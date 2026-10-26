# Guided PR actions

**Goal:** `git subrepo pr` offers guided PR actions in a terminal. Bare
`git subrepo` always prints help. Explicit PR operations remain available for
scripts and agents.

**Design:** Use the existing `dialoguer` dependency, matching the project's
other interactive commands: arrow-key selection, Enter to select, Esc to leave
a selector, editable text prompts, and default-deny publication confirmation.
Keep the UI inside `commands/pr/menu.rs`; do not add a general-purpose root menu
or a new UI framework.

PR discovery finds retained exports across registered owner worktrees. It displays
the owner and destination, and lets users reopen review work even when the current
source is dirty. Opening an export starts a shell; `exit` returns to the menu.
Preparation requires committed source. Publication verifies the displayed revision
and session identity under the session lock. Reconciliation stages the accepted
import in the owning checkout. GitHub PR creation remains a separate action.

## Tasks

- [x] Add regression coverage for bare root help in a terminal and bare `pr`
  guidance without a terminal; test real arrow-key selection and Escape.
- [x] Move the menu under `pr` and replace numbered stdin prompts with `dialoguer`.
- [x] Keep dirty-source diagnostics, cross-worktree discovery and reviewed-revision
  and session-replacement publication guards.
- [x] Update README and the owning PR workflow documentation.
- [x] Verify focused CLI, PR, import and PTY tests; formatting and Clippy.
- [x] Reinstall and exercise the actual Dimensify export read-only.
- [x] Verify the pre-existing index remains unchanged; leave changes uncommitted.

## Evidence

The revised PTY tests failed against the earlier root-menu implementation:
bare `pr` required an action, while bare root invocation opened a menu.
Publication tests use temporary local remotes; no real remote publication is
authorized by this task.

Focused verification passed: 59 Rust tests across the CLI, PR export, import,
binary and guided-menu suites, including ten Python PTY scenarios. Formatting
and `git diff --check` passed. All-target Clippy passed with warnings denied
except the previously recorded baseline categories `useless_borrows_in_formatting`
and `items_after_test_module`. The narrow read-only review found no remaining
issues. Source HEAD remains `edebbef11a600065269b2b5c3d1ede41f2501abb`.

Reinstalled with `cargo install --path . --force --offline`. An installed-binary
PTY check from the dirty Dimensify subtree opened `contribute/bevy-019-robosim`,
verified its HEAD, returned to the selector and cancelled with Escape. A separate
terminal check confirmed bare root invocation prints help and exits. Candidate
HEAD and working status were unchanged. Transcript:
`/tmp/git-subrepo-pr-selector-installed.log`. No live publication or CI was tested.

The pre-existing staged diff is preserved in
`/tmp/git-subrepo-guided-menu-index.patch`, SHA-256
`b41055180ad876d136e588706b8b2a40512fdfde0c059f73b2763b19e5cd79ae`.
The pre-menu installed binary backup is
`/Users/tinlai/.cargo/backups/git-subrepo/20261015-002514`.
