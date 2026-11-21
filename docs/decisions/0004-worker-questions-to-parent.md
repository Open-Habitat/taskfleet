# ADR 0004 — A worker asks its parent a blocking question

- **Status:** Accepted
- **Date:** 2026-11-21
- **Decider:** Jari Mustonen
- **Issue:** `worker-question-to-parent` (proposal written by the Open Habitat
  project against Taskfleet 0.20.0)
- **Builds on:** ADR 0001 (told facts, not inference), ADR 0003 (resident
  rounds)

## Context

A worker that meets a decision outside its brief could only end the run with
`success: false`, or append `node.awaiting_input`. That event means a prompt
answered at the worker itself: it reaches the parent's `run wait` after a
180-second grace meant to spare a person a page, and nothing in it carries an
answer back. `run message` reaches a live worker only on Pi. A Claude or Codex
worker could not get a mid-run answer, and no worker could wait for one with a
bounded, recorded fallback.

Jari approved the proposal on 2026-11-21 with one decision on top: the question
and the answer happen entirely between the worker and its parent agent. The
person talks only to the parent. Taskfleet adds no human-facing surface.

## Decision

A question is a durable record in the run's event log, matched by an id and
carried by ordinary shell commands, so it works in every harness that can run
one.

- `node ask` (worker) appends `node.asked` and blocks; `run answer` (parent)
  appends `node.answered`; `run escalate` (parent) appends
  `node.ask_escalated`; whoever first finds an open question past its deadline
  under the run's exclusive lock appends `node.ask_timed_out`. The node
  projection holds `open_ask` and `last_ask`; read surfaces show both.
- The question's id is the seq of its `node.asked`. Every later event names
  it, and only the open question with that id is affected, so the first
  closing event wins and a stale answer cannot close a newer question.
- `--default` is required and recorded, not judged. `--timeout` fixes the
  deadline once, from the `node.asked` timestamp; re-asking with the same
  idempotency key re-attaches and never moves it. `--wait` bounds one
  invocation because harnesses cap command runtime.
- A default `run wait` returns at once, with no grace, on an open question that
  is neither escalated nor past its deadline (`asked: true`). The grace exists
  so a short prompt does not page a person; here the worker is blocked and the
  reader is its parent agent. `--terminal` ignores questions. An open question
  is not graded as an error under `--fail-on-error`.
- `awaiting_input` is not set for a question.

## Choices the proposal left open

- **Who records the timeout.** The commands, not the supervisor. `node ask`,
  `run answer`, and `run escalate` each check the deadline under the exclusive
  lock and append `node.ask_timed_out` when it has passed, so an answer racing
  the deadline leaves exactly one closing event and both sides report it. The
  supervisor and its hot files are untouched. If no command ever looks again
  (the worker's command is gone), readers show the question as `expired` and it
  no longer returns `run wait`; the first later `run answer` or re-attaching
  `node ask` records the timeout. The reducer does not judge timestamps: it
  folds the first closing event, which keeps replay a pure function of the log.
- **Cancellation is a fold, not an event.** Every reducer path that ends the
  node or its worker attempt (terminal status or report, retry, the told worker
  exit, a round report, a resumed round) moves the open question to `last_ask`
  as `cancelled` with the closing event kind as its reason. A run-level
  terminal status seen before the node projection catches up also reads as
  `cancelled` to the waiting worker.
- **Idempotency is node-scoped and by key.** The key is stored as the
  `node.asked` envelope key `ask:<node>:<key>`. A re-ask with the same key and
  the same question re-attaches, even after that question closed (a killed
  command learns its answer); the same key with a different question is refused
  (`idempotency_conflict`). A second question while one is open is refused
  (`ask_already_open`) and names the open one.
- **Asking on an ended node** returns outcome `cancelled` (exit 3) without
  recording anything, because the worker must stop either way.
- **A repeated identical answer** is a successful replay; any other answer to a
  closed question is refused (`ask_closed`) with how it ended and, when it
  timed out, the default the worker took.
- **`--decided-by`** defaults to `agent`. It is the answering session's own
  statement, unverified, and grants the worker nothing its brief did not.
- **Exit codes** follow the proposal: 0 answered, 4 pending, 2 timed_out,
  3 cancelled, 1 usage. Exit 2 is also the generic system-error code; a real
  outcome always carries a success envelope on stdout, an error an error
  envelope on stderr. Either way the worker's safe move on exit 2 is its
  default.
- **Bounds:** question 8 KiB, default 4 KiB, answer 16 KiB, note 2 KiB, at most
  16 distinct options of 256 characters, key 1–128 visible ASCII, timeout and
  extension at most 7 days, one `--wait` at most 24 hours.
- **Scope of read surfaces:** `run wait`, `run show`, and `run list` read the
  default node `n-0001`, as they do for every other attention signal.
  `node show` exposes the raw projection for any node.

## Not decided here

Typing into a worker's terminal or per-harness delivery adapters, a
non-blocking note channel, a push hook to the parent, any change to who may
write to a run, and any human-facing surface.

## Consequences

The generated worker preamble now tells every worker to ask with `node ask`,
state a default the brief allows, continue on the answer or the default, and
name decisions taken by default in its final report. Ending with
`success: false` remains for a decision with no acceptable default. Parent
skills read `asked` / `open_ask` from `run wait` and answer with `run answer`.
