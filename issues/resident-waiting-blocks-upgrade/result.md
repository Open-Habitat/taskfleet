# Result: residents waiting between rounds no longer block upgrades

## What changed

- `run upgrade-lock` classifies a published run as a **waiting resident** when
  it is a non-terminal resident whose only node is non-terminal, whose current
  attempt's round report is open, whose worker has a told exit, with no live
  agent and no pending merge (`run/upgrade_survey.rs`, `read_run`). Such a run
  is not a live run: it needs no flag and is not an override (`quiescent`
  stays true, no receipt). It is listed in `data.waiting_residents[]`
  (`run_id`, `state_schemas`, `supervisor_pid`, `supervisor_stopped`).
- The survey takes the resident operation lock of each candidate without
  waiting, re-reads it under that lock, and holds the lock until the
  activation ends, so no `run land` or idle expiry acts on it meanwhile. A
  held lock (operation in progress) makes it a live run again, with reason
  `resident_operation_in_progress`.
- After every refusal is ruled out and before the inhibit marker is written,
  the gate stops each waiting resident's live supervisor with SIGTERM
  (`stop_waiting_supervisors`), only after re-reading the run (still between
  rounds) and matching the pid's kernel start identity against the record. A
  supervisor that does not exit within 30 s refuses
  `upgrade_supervisor_stop_timeout` with admission unchanged;
  `details.stopped_supervisor_run_ids` names any already stopped (also on a
  failed activation).
- `run message`'s resume path now takes the worker-admission lock shared
  (`run/resume.rs`), like `run create` and `run reattach`. A resume during an
  activation is refused `admission_busy` / `upgrade_inhibited` instead of
  starting a worker mid-swap. That is what makes letting the resident through
  safe: no round can begin while the installation changes.
- The resident's `.creating` control files (round marker, Pi message adapter,
  stale socket) and Pi session were already attributed to the published run by
  `leaked-creating-adapters` (0.23.0); nothing touches them.
- Operator text: `run upgrade-lock --help` long about, a new section in
  `docs/UPGRADE-ADMISSION.md`, the `upgrade_survey` module header, CHANGELOG
  `[Unreleased]` / Changed.

## Does the old supervisor survive a binary swap?

It would survive the swap itself: it keeps its mapped image, and while a
single-node resident waits it starts no Taskfleet executable (child
supervisors are forked only for `child.spawned`, which a resident never has;
the notify hook runs the operator's own `sh -c` command; idle expiry runs git
and tmux only; the Pi adapter is embedded with `include_str!` and written
only by a launch). It reads only run-state files.

It is not safe to leave running, though. After the swap, the next
`run message` from the new CLI starts the round, and the old supervisor would
adjudicate that round's exit and append events with the old reducer beside the
new CLI. That is the mixed fleet `--allow-live-runs` only admits with a
declared, equal state schema, and even then an older same-schema writer drops
newer optional fields. Without a flag the gate cannot know the target schema.

So the gate stops it through an existing supported path and lets the new
binary reattach: SIGTERM is the supervisor's graceful exit (§7.8: finish the
tick, `supervisor.exited {reason: signal}`, remove the pid file; it never
terminalizes or tears down), and a dead supervisor of a waiting resident is
the same state a host restart leaves, which `run message`
(`resume::ensure_supervisor`), `run merge` (`ensure_report_consumer`) and
`run reattach` already restart from their own executable. No `node ask` was
needed.

## Decisions for later review

- **No flag for waiting residents, any state schema.** After the stop no old
  process remains, so the only cross-version reader is the new release reading
  its predecessor's run state, which its own `SUPPORTED_STATE_SCHEMAS` check
  fails closed on (a `run message` error, data untouched). When the operator
  does pass `--allow-live-runs --target-state-schema N`, waiting residents are
  held to `N` like live runs and refuse `upgrade_state_schema_incompatible`
  on a mismatch.
- **Always stop the supervisor**, also under `--allow-live-runs`, rather than
  accept it as a live run: one rule, and strictly fewer old processes.
- **Idle expiry pauses** until the next `run message`, `run merge` or
  `run reattach` starts a supervisor. Expiry only removes a fully landed
  worktree or raises attention, so the delay preserves more, never less. The
  round-reported notification has already fired by the time a resident is
  waiting. The upgrade does not reattach supervisors itself: the gate process
  is the old binary, and re-executing the product name from `PATH` is what
  `self_exec` exists to avoid.
- **Waiting residents are not an override:** no receipt, `quiescent: true`.
  The stop is recorded in each run's own event log (`supervisor.exited`) and
  in `data.waiting_residents[].supervisor_stopped`.
- **A live supervisor without a verifiable start identity** (legacy pid record
  that cannot be matched) keeps the run classified as live, because signalling
  it could reach a recycled pid.
- **Dormant residents** (idle expiry removed the landed worktree, round still
  open) count as waiting; the next message recreates the worktree as before.
- The clean-exit-without-round-report attention case is not "between rounds"
  and still blocks; the issue scoped only waiting residents.
- Residual: a `run cancel` or `run merge` (which take neither admission nor
  the resident lock) landing between the stop's re-read and the signal could
  have its teardown deferred; `run merge` reattaches a supervisor for exactly
  that case, and a cancelled run's teardown resumes on the next reattach.

## Evidence

Production-path tests in `crates/taskfleet/tests/resident.rs` (real
materializer, launcher, run-worker shim, supervisor):

- `waiting_resident_lets_upgrade_through_and_resumes_after_it`: a waiting Pi
  resident lets the activation through; the supervisor is stopped; the round
  marker, adapter, socket path and Pi session bytes are unchanged; a
  `run message` attempted from inside the activation fails `admission_busy`
  and starts no attempt; after release `run message` resumes the same
  `--session`, a second `supervisor.started` appears, and the resident
  mid-round then refuses the upgrade again.
- `mid_round_resident_still_refuses_upgrade`: before the round report, and
  after it while the worker is still running (`agent_live`), the gate refuses
  `upgrade_not_quiescent` and the supervisor stays up.
- `refusals_beside_a_waiting_resident_touch_nothing`: an unpublished creation
  refuses `upgrade_state_unverifiable`, and a declared
  `--target-state-schema 2` refuses `upgrade_state_schema_incompatible`
  naming the resident; in both the supervisor keeps running and no marker is
  left.

Gate: `scripts/validate-local-release.sh` (see the final report for the run).
