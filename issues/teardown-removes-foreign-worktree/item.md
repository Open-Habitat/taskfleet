---
created: 2026-11-13
updated: 2026-11-13
type: bug
status: untriaged
priority: normal
provenance: chat
source_ref: conductor:2026-11-13/open-habitat-gate-worktree
---

# Spinoff teardown removed a foreign worktree's registration (.git/worktrees deleted)

## Description

## Observed (13.11.2026, Trude, taskfleet 0.20.0)

In `~/src/open-habitat`, a conductor ran the product's clean gate in a
manually added, detached worktree (`git worktree add --detach
/var/tmp/agent-scratch/oh-gate-604b03d1 <sha>`). While it ran, a headless
spinoff in the same repository (run `01m7akhreacxskhfg58sfby70d`) landed and
was torn down at about 16:18 local time. At that moment `.git/config` was
rewritten (its workmux sections changed) and the whole `.git/worktrees/`
directory disappeared, including the foreign gate worktree's registration;
its checkout stayed on disk with a `.git` file pointing to the deleted
`.git/worktrees/oh-gate-604b03d1`. The gate then failed with "fatal: not a
git repository" and "the gate changed the shared repository config".

`git worktree prune` alone would not remove a registration whose checkout
exists, so something in teardown (Taskfleet or workmux) removed more than
its own worktree.

## Expected

Teardown removes only its own worktree registration and its own config
section; foreign worktrees and unrelated config stay untouched.

## Triage analysis

**Verdict: cannot tell, not caused by taskfleet's teardown by itself.** A
specific plausible path with material impact remains. Examined taskfleet
v0.20.0 and v0.21.0 (`7d1f83a9`; no relevant change between them) and workmux
v0.1.284 (`748997d`). Full trace: `analysis.md` in this issue directory.

- **What teardown does.** `supervise/cleanup.rs` runs only
  `git worktree remove` on the node's own path and `git branch -d/-D` on its
  own branch. It never calls workmux or `git worktree prune`. `branch -D`
  rewrites `.git/config` by removing the branch's own section, including
  workmux's `workmux-base` key. That is the config change that was seen, and
  it happens on every landing.
- **Why `.git/worktrees/` vanished.** Git `rmdir`s it only when it is empty,
  after removing the last entry. So the foreign registration was already gone
  when the teardown removed its own entry. Removing it takes a repo-wide
  `git worktree prune` that could not see the foreign checkout: a process
  with a different mount namespace or private `/var/tmp`, or a sandbox.
  Candidate callers: workmux's remove/cleanup, which always prunes
  repo-wide and deletes its `workmux.worktree.<handle>` section, matching
  both symptoms; taskfleet reaches it only from spawn rollback
  (`run/spawn.rs`, `run/create.rs`). Also taskfleet's resident resume
  (`run/resume.rs`), and the gate itself.
- **Responsible code.** Not taskfleet's teardown. The dangerous primitive is
  the unscoped `git worktree prune` in workmux, and to a lesser degree in
  taskfleet's resident resume, combined with an environment where callers
  disagree about which paths exist.
- **Severity: medium.** It breaks a concurrent, manually added worktree in
  the same repository: the checkout's data survives, but the checkout must be
  moved aside and re-added. It hits operators who mix their own worktrees
  with taskfleet runs, and only when the pruning process cannot see the
  foreign path.
- **Repro status.** Partly reproduced in a disposable repository. The
  teardown sequence alone leaves the foreign worktree intact. A prune run
  while the foreign path was invisible, followed by the teardown, produces
  exactly the reported end state.
- **Fix sketch.** Taskfleet: replace the `workmux remove` calls in the spawn
  rollback, and the prune in resident resume, with removal scoped to the
  run's own worktree (its own admin dir and its own
  `workmux.worktree.<handle>` section). Teardown should also remove that
  section, which currently leaks. Workmux (upstream): scope cleanup's prune
  to the quarantined worktree. Affected host: compare mount namespaces of the
  conductor, the tmux server and the supervisor, and check whether the
  spinoff's `workmux.worktree.<handle>` section survived. A gate that
  requires a byte-identical shared config conflicts with any concurrent
  landing by design.
