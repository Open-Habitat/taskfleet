# Result

`run salvage` now aborts the rebase it started when rebasing the preserved
branch onto the source conflicts. `run_git_merge` takes an `OnRebaseConflict`
policy: salvage (native and caller-owned) uses `Abort`; `run merge` and
`run land` keep `LeaveForWorker`. The abort fires only when no rebase was in
progress before the call and one is in progress after it, and it is verified
(HEAD back on the branch, branch at its pre-rebase OID, no rebase state). The
`merge_failed` error carries `details.rebase` (`aborted` or `in-progress` if
the abort could not be verified) with worktree, branch, source and
`branch_oid`, and the message says how to rebase by hand and re-run
`taskfleet run salvage <run-id>`.

## Verified

- New integration test `run_salvage::rebase_conflict_aborts_and_leaves_branch_as_found`:
  branch tip, HEAD, clean status, no rebase dirs, main unchanged, no report.
- New unit test `caller_salvage_aborts_the_rebase_it_started_on_conflict` for
  the caller-owned driver.
- `run_merge::rust_driver_conflict_preserves_unmerged_work` now also asserts a
  live-worker `run merge` still leaves the rebase in progress.
- `scripts/validate-local-release.sh` passed on the implementation commit.

## Not verified

- The caller-salvage CLI path that merges `details.rebase` into the
  settlement-phase details is covered only by reading the code, not by an
  end-to-end test.
- The `in-progress` branch (an abort that fails) is untested; it needs a
  failure injected into `git rebase --abort`.
