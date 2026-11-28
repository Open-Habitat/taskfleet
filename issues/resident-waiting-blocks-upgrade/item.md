---
created: 2026-11-28
updated: 2026-11-28
type: improvement
reporter: jari
status: untriaged
priority: normal
provenance: chat
source_ref: chat:taskfleet-conductor/2026-11-28/resident-blocks-upgrade
---

# Residents waiting between rounds block fleet upgrades

## Description

## Observed

2026-11-28 on Trude, the fleet upgrade 0.22.0 → 0.23.0 was refused by `run upgrade-lock` with `upgrade_state_unverifiable: unpublished creation state at .creating/round-<run>-9.reported`. The run was a live resident (`round_state: waiting`, last round reported hours earlier, no worker process). Two such residents held the machine on the old release; a resident can wait between rounds for days (idle expiry 24 h, longer with unlanded work), so every upgrade on a machine that uses helpers can stall.

## Expected

A resident waiting between rounds has no worker process running taskfleet code, so swapping the binary is not a risk to it: its next round is started by `run message` with whatever binary is installed then. `upgrade-lock` should let an upgrade through when the only live runs are residents between rounds (state schema permitting), and keep refusing for a worker that is mid-round or a creation in progress. If the state schema changes between versions, the existing mixed-fleet refusal still applies.
