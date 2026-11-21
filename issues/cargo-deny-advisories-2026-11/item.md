---
created: 2026-11-21
updated: 2026-11-21
type: chore
status: untriaged
priority: high
provenance: other
provenance_detail: observed by taskfleet run gate
source_ref: taskfleet:01m7yhebct9x5jnnecf3d24mgv/gate:cargo-deny-advisories-2026-11-21
originating_run: 01m7yhebct9x5jnnecf3d24mgv
originating_run_kind: spinoff
---

# cargo deny advisories (chrono, humantime unmaintained; xattr unsound) turn the release gate red

## Description

## Observed

On 2026-11-21, `scripts/validate-local-release.sh` failed at `cargo deny --locked check`
(advisories) with an unchanged `Cargo.lock` (same lock as `main` at `eb82ca65`),
so the gate is red for every branch, not only for the run that saw it
(taskfleet run `01m7yhebct9x5jnnecf3d24mgv`):

- `RUSTSEC-2026-0543` — `chrono` is unmaintained. "No safe upgrade is available!"
- `RUSTSEC-2026-0542` — `humantime` is unmaintained. "No safe upgrade is available!"
- `RUSTSEC-2026-0486` — `xattr` unsound (zero-length buffer marks unwritten
  bytes initialized). Solution: upgrade to `>=1.1.6` (`cargo update -p rustix`
  suggested by cargo-deny).

`deny.toml` sets `unmaintained = "all"`, `unsound = "all"` and `ignore = []`
("Advisory exceptions must be narrowly justified and time-boxed. There are no
active exceptions"), so each of these fails the gate.

## Impact

No worktree can pass the required green gate before merging, and no release
can be cut, until this is resolved.

## Decision needed

`xattr` is a lockfile update. `chrono` (timestamps in every event and
projection) and `humantime` (`--timeout`/`--wait` duration parsing) have no
safe upgrade: either replace them, or add narrowly justified, time-boxed
`ignore` entries with a recorded rationale.

Also seen in the same run: stable clippy 1.100 (2026-11-09) added
`map_unwrap_or` hits in pre-existing code (`events.rs`, `config/show.rs`,
`event/tail.rs`, `run/list.rs`, `run/resume.rs`, `skill.rs`,
`tests/supervise_gates.rs`); those are fixed on branch
`wt/ecf3d24mgv-worker-asks-parent-node-ask-run-answ`.
