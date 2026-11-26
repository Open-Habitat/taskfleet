---
created: 2026-11-26
updated: 2026-11-26
type: bug
status: in-progress
priority: normal
provenance: chat
source_ref: taskfleet:01m8b6b4bn90wv2mzgrasbnccf/task:pane-capture-teardown
originating_run: 01m8b6b4bn90wv2mzgrasbnccf
originating_run_kind: spinoff
collision: [supervise-cleanup]
---

# Failed pane capture blocks terminal worktree teardown

## Description

## Description
A native worker successfully merged, then its pane exited before final evidence capture. The supervisor recorded worker.evidence.failed repeatedly and retained the landed worktree, branch and supervisor indefinitely. A second observed run showed the same missing-pane failure.

## Reproduction
Finish a native worker with run merge and close its recorded pane before terminal evidence capture. capture-pane fails and cleanup never proceeds.

## Acceptance Criteria
- [ ] Missing pane capture is recorded without blocking terminal teardown.
- [ ] Unmerged and dirty work remains preserved without endless pane retries.
- [ ] Regression tests, full local release gate and independent lifecycle review pass.

## Comments

### 2026-11-26T05:55:04Z · @agent

Source-level critical review completed with gpt-6-luna (finder) and gpt-6-astra (independent destructive-boundary verifier); no concrete findings. Assessment: zero findings, no follow-up issues. Pane diagnostics cannot alter merge authority, node status or existing Git preservation checks. Transcript identity, writer quiescence and durable archive writes remain prerequisites. Runtime validation pending.

### 2026-11-26T05:57:08Z · @agent

Release-mode targeted selection passed 11/11 tests, including native merged/failed/cancelled supervisor exit plus cleanup preservation cases. Red proof: temporarily restored the pre-fix evidence implementation while retaining the three new cleanup tests; all three failed (0 passed, 3 failed) on evidence completion as expected, then restored the fix. Full release gate follows. Tool selection: export PATH="$HOME/.cargo/bin:$PATH", then prepend dirname of rustup which --toolchain stable cargo, bypassing the host Cargo wrapper. Build scratch uses a private TMPDIR under /var/tmp/agent-scratch with the shared lease.
