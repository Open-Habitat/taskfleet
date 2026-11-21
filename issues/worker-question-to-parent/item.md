---
created: 2026-11-21
updated: 2026-11-21
type: feature
status: done
priority: normal
provenance: other
provenance_detail: Open Habitat proposal approved by Jari 2026-11-21
source_ref: taskfleet:01m7yhebct9x5jnnecf3d24mgv/brief:tf-ask-proposal
originating_run: 01m7yhebct9x5jnnecf3d24mgv
originating_run_kind: spinoff
lane: worker-messaging
closed: 2026-11-21
closed_by: taskfleet:01m7yktrmyzj9sdfepdyr61qqm
---

# Worker's blocking question to its parent (node ask / run answer)

## Description

## Problem

A worker that meets a decision outside its brief can only end the run with
`success: false`, or append a low-level `node.awaiting_input` event. That event
is described as a human decision, reaches the parent's `run wait` only after a
180-second grace, and has no answer: `node.input_resolved` clears attention and
delivers nothing. `run message` reaches a live worker only on Pi. A Claude or
Codex worker cannot get a mid-run answer, and no worker can wait for one with
a bounded, recorded fallback.

## Decision (approved by Jari, 2026-11-21)

Implement the Open Habitat proposal "A worker's blocking question to its
parent: `node ask`, `run answer`" (written against Taskfleet 0.20.0), with one
decision on top: the question and answer happen entirely between the worker and
its parent agent. The person talks only to the parent. No human-facing
surfaces.

## Scope

- Worker side: `taskfleet node ask (--current | <RUN_ID> <NODE_ID>)
  (--question | --question-file) --default TEXT [--option]... [--timeout 30m]
  [--wait 8m] --idempotency-key KEY`, with exits 0 answered, 4 pending,
  2 timed_out, 3 cancelled, 1 usage.
- Parent side: `taskfleet run answer <RUN> --ask <ID> (--text | --body-file |
  --choice) [--decided-by agent|person]` and `taskfleet run escalate <RUN>
  --ask <ID> [--extend] [--note]`.
- Durable node events `node.asked`, `node.ask_escalated`, `node.answered`,
  `node.ask_timed_out`; `open_ask` / `last_ask` on `run show`, `run list`,
  `run wait`.
- A default `run wait` returns at once on an open, non-escalated question;
  `--terminal` ignores it.
- Timeout: the worker continues on its default, recorded as one closing
  event. Late answers refused with the default named. The node ending cancels.
- Worker preamble, bundled skills and CLI help updated.

The open choices the proposal left are recorded in
`docs/decisions/0004-worker-questions-to-parent.md`.

## Resolution

### 2026-11-21T08:47:48Z · @taskfleet:01m7yktrmyzj9sdfepdyr61qqm

Landed via run 01m7yktrmyzj9sdfepdyr61qqm: node ask / run answer / run escalate adopted from the preserved branch of 01m7yhebct9x5jnnecf3d24mgv, ported to jiff, full local release gate green (1331 tests, review-regression tests included).
