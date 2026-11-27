---
created: 2026-11-27
updated: 2026-11-27
type: bug
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:release-missing-dist-manifest-v0-22-0
---

# Green release workflow published a GitHub Release without dist-manifest.json

## Description

The v0.22.0 cut's `release.yml` run succeeded in every job, and the `host` step logged "artifacts uploaded and released successfully", but the GitHub Release had no `dist-manifest.json` asset (all other 11 assets present). `scripts/shipshape-release.sh` stopped with `rust:taskfleet:gh-releases is missing at its destination`. v0.21.0 and v0.21.1, built by the same workflow and dist 0.33.0, have the asset. Uploading the host job's `artifacts-dist-manifest` artifact by hand made `verify` match and `resume` completed the cut.

Expected: the release workflow either publishes `dist-manifest.json` or fails, so a green run never leaves the gh-releases target unreconcilable. Root cause not established.
