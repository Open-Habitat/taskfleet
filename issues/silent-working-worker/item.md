---
created: 2026-11-27
updated: 2026-11-27
type: bug
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:silent-working-worker-20261126
---

# Live worker with no progress for days is never surfaced to the parent

## Description

A headless autonomous Pi research run stayed `pending` for three days. Its terminal showed Pi's "Working" spinner the whole time, no event was appended after `supervisor.started`, and the worker telemetry carried no turn activity. `run show` reported `stalled: false`, `awaiting_input: false`, no attention, and `progress_evidence: null`; nothing woke the parent. The `startup_blocked` hint does not apply because the worker had already passed its first turn. It was found only by inspecting the run by hand and was cancelled.

Expected: a live worker with no observable progress for a long, configurable interval is surfaced to the parent (read-time, like `stalled`/`startup_blocked`), or at least `run show` says progress is unknown rather than reporting a healthy pending run.
