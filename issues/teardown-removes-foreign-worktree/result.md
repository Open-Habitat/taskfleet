# Result

## Changed

- Creation rollback (`run/spawn.rs` `Rollback`, `run/create.rs`
  `rollback_reclaimed_materialization`) no longer calls `workmux remove`. It
  closes its own window with `workmux close <branch>` and then
  `Git::rollback_own_worktree` removes the checkout by exact path, its own
  `workmux.worktree.<handle>` section, and its own branch.
- Resident resume (`run/resume.rs`) unregisters only the resident's own stale
  entry (`git worktree remove <recorded path>`, only while the checkout is
  missing) instead of a repository-wide `git worktree prune`.
- Teardown (`supervise/cleanup.rs`) and resident idle expiry remove the run's
  own `workmux.worktree.<handle>` section once the worktree is gone. The
  removal fails closed when Git cannot list registrations or a registered
  worktree shares the handle.

## Verified

- `creation_reliability::rollback_leaves_foreign_worktree_with_invisible_checkout_registered`
  drives a real `run create` that fails after a (fake) `workmux add`, with a
  foreign detached worktree whose checkout is moved aside. The foreign
  registration and workmux section survive; the run's own worktree, section,
  and branch are gone. Against the previous `spawn.rs` (whose rollback called
  `workmux remove`, modelled faithfully by the fake as a repository-wide
  prune) the same test fails.
- Unit tests: merged teardown with a hidden foreign worktree, resident
  recreation with a hidden foreign worktree, `rollback_own_worktree` without a
  path hint, and the exact/fail-closed behaviour of `workmux_meta_remove`.
- `scripts/validate-local-release.sh` passed on the implementation commit.

## Not verified

- Real workmux was not exercised: `workmux close` replacing `workmux remove`
  for killing the rollback window was checked against workmux v0.1.284 source
  (close kills the window and does not prune), not run live.
- The field incident's actual pruning actor was not identified; on the affected
  host the remaining candidates are operator or agent `workmux remove`/merge
  calls and the gate itself. Workmux's own cleanup still prunes repository-wide
  (upstream).
