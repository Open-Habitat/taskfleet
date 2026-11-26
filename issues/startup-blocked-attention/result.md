# Result: startup-blocked-attention

## What landed

`crates/taskfleet/src/run/startup.rs` computes a read-time `startup_blocked`
hint. `run show`, `run list` (`(startup-blocked)` in text) and `run wait`
expose it as `startup_blocked` plus `startup_blocked_detail` (attempt start,
age, grace, worker pid, a `resume_hint` naming `run attach` / `run cancel`).
No event, no projection write, no status change.

The hint holds when an autonomous, Taskfleet-launched, single-node run has
a node still working whose current attempt finished its PID handshake
(`agent_pid` set), has no told exit, no open request or question, no
higher-precedence verdict (attention-required, stall, confirmed-dead
supervisor), and whose native Pi transcript still holds only the header
Taskfleet wrote, once a grace has strictly passed since `node.started_at`.
The default grace is 120 s; `TASKFLEET_STARTUP_BLOCKED_GRACE_SECS` changes it.

Decisions taken inside the brief's scope:

- **Telemetry is not an input.** The issue suggested "worker-v1 telemetry
  never reported a turn start" as a second signal, but the public
  `contracts/worker-telemetry-v1` promises telemetry never affects wait
  settlement, and a last-told sample cannot show that a turn ever started.
  Workers without Taskfleet-written Pi evidence (Claude, caller-owned,
  interactive) therefore never show the hint.
- **Own flag, not `attention_required`.** `attention_required` means "clean
  exit without `run merge`" and promises an `attention` block; reusing it
  would break consumers that rely on that. Open Habitat needs to read
  `startup_blocked` to show this state.
- **"Wakes once"** without persisted state: a default wait measures the
  grace from the later of the attempt start and its own start. The wait
  running when the grace passes wakes then. A re-wait after the wake is not
  woken again until a grace of its own has passed, so it cannot spin. A
  wait that starts after the run is already blocked wakes one grace later.
  `--terminal` ignores the hint; `--fail-on-error` grades the wake exit 3.

## Verified

- Unit tests: transcript classification (header-only versus any following
  entry; malformed files fail closed to "reached"), predicate eligibility,
  strict grace boundary, the wait-relative wake anchor, hint quoting, and
  `--fail-on-error` grading.
- Integration test `startup_blocked_worker_wakes_default_wait_and_clears_on_first_turn`
  (`tests/run_wait.rs`) uses the real binary against a forged live worker: show/list
  flag it, `--terminal` times out but still shows it, a default wait wakes
  about one grace after it began and leaves the run `pending` (exit 3 under
  `--fail-on-error`), and appending a transcript entry clears it everywhere.
- Full `scripts/validate-local-release.sh` passed on 7e9a701b.

## Not verified

- No live Pi was run against a real trust or auth prompt. The signal assumes
  that Pi appends to a pre-existing `--session` file only once it has opened
  the session, which is past its startup prompts. That matches the Pi
  session-manager source read during this work, but nothing here pins it.
- Resumed resident rounds reuse a transcript that already has entries, so a
  prompt during a resumed round is not detected (a missed hint, never a
  false one).
