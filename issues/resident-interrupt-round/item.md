---
created: 2026-11-08
updated: 2026-11-28
type: feature
reporter: jari
status: done
priority: normal
lane: resident
lane_seq: 2
closed: 2026-11-28
closed_by: pi-conductor
---

# Interrupt a resident worker's current round without ending the run

## Description

Jari 8.11.2026, using Open Habitat's helpers (named resident workers): there is
no way to stop what a helper is doing right now while keeping it. A person can
press Esc on their own agent to stop the turn; for a resident worker the only
choices are `run cancel` (ends the whole run) or `run message` (reaches a live
Pi worker as steering, or resumes one between rounds). The missing middle is
an interrupt: stop the current turn, keep worktree, branch and native session,
and leave the worker between rounds so the next `run message` resumes it with
new instructions, the same for Pi and Claude.

Sketch: `taskfleet run interrupt <run-id|name>` — Pi through the acknowledged
adapter's abort (or SIGINT to the recorded agent process), Claude by SIGINT
to its recorded process; record an interrupted round (open round report
`interrupted: true`, no merge), uncommitted changes stay in the worktree,
`run wait` settles as an interrupted round, `run message` resumes. Fenced to
the recorded worker identity like salvage.

Decision (Jari 8.11.): the interrupt is the main agent's tool, not a
person's button in Open Habitat; Habitat only teaches its agent the command.

## Acceptance Criteria

- [ ] `run interrupt` stops a resident Pi and Claude worker's current turn; worktree, branch, session kept; `run message` resumes it.
- [ ] `run wait` and `run show` report the interrupted round distinctly.

## Resolution

### 2026-11-28T06:27:01Z · @pi-conductor

Implemented in d3fb00bc and released in v0.20.0 (taskfleet run interrupt); result.md records the design. Only the closure was missing (Jari 28.11.2026).
