# Reviewed PR exports

## Design

Keep the existing direct push workflow compatible. Add `git subrepo pr
prepare|status|publish|reconcile <subdir>` for upstream contributions. A PR
session belongs to one owning checkout and lives in its Git directory, outside
the exported source. Preparation produces a standalone repository with only
upstream ancestry and a public commit message. Review fixes are ordinary commits
in that repository; publication sends exactly the explicitly reviewed HEAD.

Preparation and publication leave the owning checkout, index and `.gitrepo`
unchanged. Track the base remote/branch independently from the push remote/topic
branch. Only reconciliation imports accepted upstream content and advances the
sync baseline. Keep standard `.gitrepo` format and direct push behavior.

Prepare a committed subtree snapshot (excluding its root `.gitrepo`), optionally
limited to literal paths. Build its proposal against the pinned upstream revision
and merge newer upstream changes before review. Record the selected proposal tree
as the reconciliation baseline. Later three-way reconciliation combines that
baseline, current local subtree, and accepted upstream tree. This preserves
unexported changes and later local edits while bringing back review fixes.

Normal PR merge is the default recommendation. Squash merges are supported by
verifying the accepted commit's tree against the candidate merged with its first
parent; do not infer acceptance from a PR being closed. Conflicts leave the
owning checkout untouched and a separate integration checkout available for
resolution. Reconciliation stages a reviewable import; the user commits it
before starting another export. No command creates a GitHub PR implicitly; the
agent skill uses explicit `gh pr create` arguments after publication authorization.

## Implementation checklist

- [x] Add real Git fixture tests for preparation, review-fix publication,
  stale reviewed heads, protected base branches, clean ancestry and unchanged
  sync metadata. Observe failures before implementation.
- [x] Add CLI routing, checkout-local session records and isolated export
  preparation. Require an explicit public message. Support literal path scope.
- [x] Add exact-HEAD publication with fast-forward review-fix updates and
  separate publication state. Reject unexpected remote branch history.
- [x] Add staged reconciliation for normal and squash merges, including
  review fixes, upstream movement, retained local changes and a second export.
- [x] Add conflict recovery and failed-operation tests. Keep direct commands
  compatible and preserve pre-existing workon/commit changes.
- [x] Update README/runbook and reusable `upstream-export` agent skill. Evaluate
  the skill against a realistic export/review-fix scenario.
- [x] Run focused integration tests, CLI/unit checks, formatting and Clippy.
  Record any remaining limitations; do not commit, install or push this work.

## Verification

Use temporary real local Git remotes, isolated identity/config, and no network.
The critical invariant is that a second export after reconciliation contains
only unexported/new changes. Run `cargo test --test pr_export`, then related
CLI/commit-context tests and the existing regression suite once the candidate is
stable. Audit every new reachable commit, not only the final source tree.

### Inspected results (2026-10-14)

- Full `cargo test`: 97 passed across five suites, including the existing TAP
  regression suite. After the final path-normalization change, the focused
  `cargo test --test pr_export` passed all 15 tests; the full suite preceded that
  final isolated change and was not repeated.
- New regressions were observed failing before their fixes, including session
  path collisions/aliases, archived records, owner GC, externally pushed review
  fixes, literal wildcard paths and an owner commit racing with staged import.
- Independent review exercised a combined scoped-export, external-review-fix,
  local-follow-up, GC, squash-merge and second-export lifecycle successfully.
  Skill scenario review prompted explicit fast-forward adoption and a required
  residual-only check on the next export.
- Rustfmt checks on new Rust files, `git diff --check`, and skill-creator's
  `quick_validate.py` passed.
- Strict Clippy reports existing `useless_borrows_in_formatting` in
  `src/commands/fix.rs:166` and `items_after_test_module` in
  `src/commands/pull.rs:378`. With only those categories exempted, all-target
  Clippy passed with remaining warnings denied. Those files were not changed.
- Live GitHub PR creation, branch protection and remote CI remain unverified.
  The installed binary was not replaced; use the built binary or install when
  desired. The skill is repository-local, not installed in the personal catalog.
- Implementation remains uncommitted/unpushed on `man`, with owner HEAD
  `edebbef11a600065269b2b5c3d1ede41f2501abb` and an empty index. Pre-existing
  context-aware commit/workon changes and unrelated untracked files were retained.
