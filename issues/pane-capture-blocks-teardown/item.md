---
created: 2026-11-26
updated: 2026-11-26
type: bug
status: fixed
priority: normal
provenance: chat
source_ref: taskfleet:01m8b6b4bn90wv2mzgrasbnccf/task:pane-capture-teardown
originating_run: 01m8b6b4bn90wv2mzgrasbnccf
originating_run_kind: spinoff
collision: [supervise-cleanup]
closed: 2026-11-26
---

# Failed pane capture blocks terminal worktree teardown

## Description
A native worker successfully merged, then its pane exited before final evidence capture. The supervisor recorded worker.evidence.failed repeatedly and retained the landed worktree, branch and supervisor indefinitely. A second observed run showed the same missing-pane failure.

## Reproduction
Finish a native worker with run merge and close its recorded pane before terminal evidence capture. capture-pane fails and cleanup never proceeds.

## Acceptance Criteria
- [x] Missing pane capture is recorded without blocking terminal teardown.
- [x] Unmerged and dirty work remains preserved without endless pane retries.
- [x] Regression tests, full local release gate and independent lifecycle review pass.

## Comments

### 2026-11-26T05:55:04Z · @agent

Source-level critical review completed with gpt-6-luna (finder) and gpt-6-astra (independent destructive-boundary verifier); no concrete findings. Assessment: zero findings, no follow-up issues. Pane diagnostics cannot alter merge authority, node status or existing Git preservation checks. Transcript identity, writer quiescence and durable archive writes remain prerequisites. Runtime validation pending.

### 2026-11-26T05:57:08Z · @agent

Release-mode targeted selection passed 11/11 tests, including native merged/failed/cancelled supervisor exit plus cleanup preservation cases. Red proof: temporarily restored the pre-fix evidence implementation while retaining the three new cleanup tests; all three failed (0 passed, 3 failed) on evidence completion as expected, then restored the fix. Full release gate follows. Tool selection: export PATH="$HOME/.cargo/bin:$PATH", then prepend dirname of rustup which --toolchain stable cargo, bypassing the host Cargo wrapper. Build scratch uses a private TMPDIR under /var/tmp/agent-scratch with the shared lease.


## Resolution

Fixed by a65a2910. Terminal pane capture is bounded and best-effort. Its failure
is cached per attempt and recorded durably; transcript and report archival
continues with a pane-unavailable diagnostic. Transcript identity, writer
quiescence and archive persistence still fail closed. The existing typed
outcome and Git guards remain the only cleanup authority.

Validation on the fix commit:
- `scripts/validate-local-release.sh` passed in full: 1,339 tests passed,
  one skipped, and both core doctests passed. Formatting, clippy, rustdoc,
  identity, snapshots, project-view, cargo-deny, release fixtures, package
  archives, Shipshape contract and distribution topology checks passed.
- Three new cleanup regressions failed against the pre-fix evidence code
  (0 passed / 3 failed), then passed with the fix.
- Native end-to-end merged, failed and cancelled runs with missing panes
  reached supervisor exit within the test's 20-second deadline. Merged work
  was removed; failed/cancelled unmerged work survived. Restart retained the
  durable transcript/report history. Dirty plain-success work was preserved
  in the cleanup regression.
- Cached pane failure after a transcript-drift retry and Claude's report /
  diagnostic archive are covered; transcript identity/drift failures still
  veto cleanup.
- Critical lifecycle review: gpt-6-luna finder and gpt-6-astra verifier;
  no concrete findings; assessment produced zero follow-ups.

The gate used `PATH="$HOME/.cargo/bin:$PATH"` followed by the directory of
`rustup which --toolchain stable cargo` first on PATH, bypassing the host
Cargo wrapper, plus a private disk-backed TMPDIR and shared scratch lease.
No installed binary, installed skills, other run state, or release tags changed.

### 2026-11-26T06:08:21Z · @issuectl

Verified fix, red/green regressions and complete local release gate; lifecycle review has no findings. Final metadata commit will be gated again before native run merge.
