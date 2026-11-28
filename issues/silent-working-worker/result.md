# Result: silent-working-worker

## What changed

A new computed read-time hint, `no_progress`, in `crates/taskfleet/src/run/no_progress.rs`,
in the same family as `startup_blocked` (`run/startup.rs`) and the stall
verdicts (`run/stalled.rs`).

- **Signal:** the modification time of the current attempt's native Pi
  transcript (`Node::evidence.live_session_path`, through the same
  `checked_live_path` and no-follow open as `startup_blocked`). It applies only
  when the transcript holds more than Taskfleet's header line, i.e. the worker
  passed its first turn.
- **Conditions:** autonomous, Taskfleet-owned, single-node, non-terminal run;
  node pending/running with a PID handshake and no told exit; no open request,
  question, or resident round; supervisor presumed working; no attention, stall,
  or startup-blocked verdict (lowest precedence of all); and the transcript
  unchanged for strictly longer than the interval.
- **Interval:** 2 hours by default, `TASKFLEET_NO_PROGRESS_GRACE_SECS` overrides
  it (whole seconds), mirroring `TASKFLEET_STARTUP_BLOCKED_GRACE_SECS`.
- **Surfaces:** `run show` / `run list` carry `no_progress: bool` and
  `no_progress_detail { reason, last_progress_at, silent_secs, interval_secs,
  worker_pid, resume_hint }`; human output shows `(no-progress)` in `run list`
  and a `no-progress:` line in `run show`. `run wait` outcomes carry the same
  pair, the `--progress` JSONL line carries `no_progress`, and a default wait
  settles on it with `error` = the stable reason, grading as exit 3 under
  `--fail-on-error`. `--terminal` ignores it.
- **Wake anchoring:** identical to `startup::wakes_wait` — the interval is
  measured from the later of the last transcript write and the wait's start, so
  each wait wakes at most once and a re-wait never returns immediately.
- **Informs only:** no event, no projection write, no status change, no kill;
  no event-append, reducer, or schema path is touched.
- Skills: `taskfleet-run-overview` (field list, meaning, remedy, wait
  semantics), `worktree-spinoff` (wait section), `fan-out` (child slot
  handling). CLI help for `run wait --terminal` mentions it. CHANGELOG entry
  under `[Unreleased] / Added`.

## Evidence

Integration tests through the production binary (`crates/taskfleet/tests/run_wait.rs`):

- `silent_live_worker_is_surfaced_once_and_clears_on_progress` — a forged live
  worker (live stand-in supervisor/worker process, transcript past its first
  turn, mtime aged 10 min): no hint with interval 3600 s; hint on `run show`
  and `run list` with interval 300 s, with `last_progress_at`/`silent_secs`
  matching the aged mtime and status still `pending`; a default wait with
  interval 1 s wakes after ≥1 s with exit 3 and the reason; a re-wait in the
  same silence (default and `--terminal`) times out with the hint visible and
  no error; appending to the transcript clears the hint.
- `no_progress_never_applies_before_the_first_turn` — header-only transcript,
  interval 0: no hint on show or wait.
- Unit tests for the wake anchor, transcript reading (header-only, empty,
  missing, past first turn), and the remedy hint.
- Snapshots updated: `run show` JSON envelope (`no_progress: false`) and the
  help-JSON `--terminal` text.

## Decisions for later review

- **No child-process suppression.** A live child of the worker is not used to
  suppress the hint during a long tool call. The recorded pid may be a wrapper
  rather than Pi, Pi may keep long-lived helper children, child observation is
  platform-specific (`/proc` on Linux, libproc on macOS), and a hung tool
  command — one shape of the silent worker this issue reports — has a live
  child too, so suppression would hide exactly that case. The trade-off is a
  possible false hint after a single tool call longer than two hours; it costs
  one look from the parent, never the work. Documented in the module header,
  skills, and CHANGELOG.
- **Pi only.** Taskfleet records the Claude session id but not the path of
  Claude's transcript; deriving it would make Claude's storage layout a
  contract here. Claude workers never show the hint.
- **Worker liveness = told facts.** "Worker alive" means a PID handshake with no
  told exit, as for `startup_blocked`; no pid probe is added. A worker that died
  silently is the crash backstop's case and is failed by the supervisor long
  before the two-hour interval.
- **Once per wait, not once per idle period across waits.** The wake anchor is
  copied from `startup_blocked`: a parent that keeps re-waiting on the same
  silent worker is reminded again after each further full interval of silence,
  never immediately. Any transcript write starts a new idle period.
- **Field name** `no_progress` / `no_progress_detail` with `last_progress_at`,
  chosen to say "since when no progress has been seen" rather than to claim the
  worker is hung.
- **Grades as an error under `--fail-on-error`**, consistent with
  `startup_blocked`.
