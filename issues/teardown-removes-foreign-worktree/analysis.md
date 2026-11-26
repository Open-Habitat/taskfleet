# Analysis: teardown-removes-foreign-worktree

Read-only analysis. Examined taskfleet `v0.20.0` (the reported release) and
`v0.21.0` (`7d1f83a9`, current `main`); workmux `v0.1.284` (`748997d`) from its
public repository; Git 2.43 behaviour, checked empirically.

## 1. What taskfleet's teardown actually runs

`supervise/cleanup.rs` (identical in this respect from v0.17.0 through v0.21.0)
issues only commands scoped to the node's own resources:

- `git -C <repo> worktree remove [--force] <own worktree path>`
  (`Git::worktree_remove`, `git/repo.rs`), force only after a confirmed
  explicit `run merge`;
- `git -C <repo> branch -d|-D <own branch>` (`Git::branch_delete`);
- for an already unregistered checkout, `remove_unregistered_checkout`, which
  deletes only a directory whose `.git` pointer names this repository's
  `worktrees/` admin dir and which no registration still claims;
- tmux window close (`session::close_original_workmux_window`), which talks to
  tmux only.

It never calls `workmux`, never runs `git worktree prune`, and never edits
`.git/config` directly. `git grep` over `crates/` at v0.20.0 finds
`worktree prune` only in `run/resume.rs` (named-resident resume, not a spinoff
teardown) and `workmux remove` only in the spawn rollback paths
(`run/spawn.rs` `Rollback::drop`, `run/create.rs`
`rollback_reclaimed_materialization`), which run when a `run create` fails,
not at teardown.

## 2. The two observations, explained separately

**`.git/config` rewritten.** `git branch -D wt/<id>-<slug>` deletes the
branch's config section, which carries workmux's
`branch.<name>.workmux-base` key, by writing `config.lock` and renaming it, so
the file gets a new inode. That is the teardown's own section and is ordinary
Git behaviour. Every spinoff that lands in a repository changes the shared
config this way, so a gate that requires the shared config to stay
byte-identical will trip on any concurrent landing. Taskfleet never removes
workmux's own `[workmux "worktree.<handle>"]` section (that leaks: a busy
checkout examined here had 11 stale handles), so if that section disappeared
for the torn-down spinoff, something other than taskfleet ran (workmux's
`remove_worktree_meta_at` is the only known remover).

**`.git/worktrees/` deleted.** Git removes that directory only through
`delete_worktrees_dir_if_empty()`, a plain `rmdir` after `worktree remove` or
`worktree prune`, which fails while any entry remains. So the directory
vanishing means the teardown removed the *last* registration: the foreign
`oh-gate-…` entry was already gone. Its removal needs a separate actor.

## 3. What can remove a registration whose checkout exists

`git worktree prune` (default expiry: immediate) discards any entry whose
`gitdir` file points at a `.git` that does not exist *from the pruning
process's point of view*. `git gc --auto` also prunes worktrees but only past
`gc.worktreePruneExpire` (3 months), so it does not apply to a fresh entry.
That leaves a repo-wide prune run from a context where the foreign checkout
was not visible, for example:

- a process in a different mount namespace whose `/var/tmp` is private
  (a service unit with a private temporary directory, a sandbox or
  container), where `/var/tmp/agent-scratch/<gate>/.git` does not exist;
- a gitdir path the pruning Git cannot resolve (for instance a relative
  gitdir written by a newer Git with relative worktree paths enabled and read
  by an older one).

Repo-wide prune callers that could have run in the window:

- **workmux** `workflow/cleanup.rs`: every `workmux remove`/merge cleanup
  quarantines its own worktree and then runs a repo-wide `git worktree prune`
  (`git::prune_worktrees_in`), then deletes the branch and the
  `workmux.worktree.<handle>` config section. This matches both symptoms in
  one call when the prune sees the foreign path as missing. Reached from
  taskfleet only through the spawn rollback paths above (a concurrent failed
  `run create` or re-fire in the same repository), or from any operator or
  agent running `workmux remove` directly.
- **taskfleet** `run/resume.rs:113` (`run message` to a dormant resident).
- The gate itself or anything it spawns, if it runs Git from a sandbox.

## 4. Reproduction (disposable repository, Git 2.43)

1. Main repo, `git worktree add --detach <scratch>/foreign-gate`, a spinoff-like
   `git worktree add -b wt/spin <scratch>/spin`, plus `workmux-base` and
   `workmux.worktree.spin` config. Ran taskfleet's teardown sequence
   (`worktree remove --force` + `branch -D`): the foreign registration and
   `.git/worktrees/` survive; config inode changes; the
   `workmux.worktree.spin` section remains. **Teardown alone does not
   reproduce the report.**
2. Same setup, but `git worktree prune` run while the foreign checkout is not
   visible at its path (simulated by renaming it, since `unshare` is not
   permitted on the analysis host), then the teardown sequence: the foreign
   entry is pruned, `worktree remove` takes the last entry and Git `rmdir`s
   `.git/worktrees/`, and the foreign checkout is left with a `.git` file
   pointing at a deleted admin dir. **This reproduces the observed end
   state exactly.**

A full taskfleet run was not used: the code path in §1 is fully explicit and
exercising it needs a live harness, tmux and workmux.

## 5. What would decide it on the affected host

- Whether `workmux.worktree.<spinoff handle>` is still in `.git/config`
  (present: workmux remove did not run; absent: it did).
- `readlink /proc/<pid>/ns/mnt` for the conductor shell, the tmux server that
  hosted the headless worker, and the supervisor: a mismatch with a private
  `/var/tmp` makes the foreign checkout invisible to Git in the other.
- The run's `events.jsonl` around the teardown, and any concurrent
  `run create` failures (`workmux_add_failed`, rollback) or resident
  `run message` in the same repository at that minute.
