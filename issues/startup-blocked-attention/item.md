---
created: 2026-11-14
updated: 2026-11-26
type: feature
status: done
priority: normal
provenance: other
source_ref: open-habitat:taskfleet-worker-trust-prompt
lane: worker-startup
lane_seq: 2
closed: 2026-11-26
closed_by: agent:taskfleet-worker
---

# Wake the parent when a live worker never reaches its first turn (startup_blocked)

## Description

## Observed (14.11.2026)

Taskfleet Pi workers on an Open Habitat installation stopped at Pi's
interactive "Trust project folder?" prompt before their first turn and sat
there for 14 minutes. Nothing woke the parent conversation: Taskfleet
attention comes only from agent-emitted `node.awaiting_input` or worker
exit, and `--agent-startup-timeout` covers only the PID handshake before
exec. (The trust cause itself was fixed in Open Habitat's launcher.)

## Proposal (from the Open Habitat worker that fixed the cause)

After the handshake, if a live worker has not reached its first turn (Pi:
no entry after the session header Taskfleet wrote; or worker-v1 telemetry
never reported a turn start) within a configurable grace (default 60–120 s),
project a read-time `startup_blocked` hint that sets attention_required /
awaiting_input on `run show` / `run list` and wakes a default `run wait`
once (not `--terminal`), naming `run attach` as the remedy. Read-time like
`stalled.rs`, no new events. Any interactive startup prompt (trust, missing
session cwd selector, auth) is then visible to the parent.

Related design discussion in Open Habitat: `worker-asks-parent` (a
cross-harness ask/answer exchange for workers).
