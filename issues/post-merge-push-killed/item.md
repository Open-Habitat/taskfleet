---
created: 2026-11-26
updated: 2026-11-27
type: bug
reporter: agent
status: fixed
priority: normal
provenance: agent
source_ref: stint:2026-11-26:post-merge-push
lane: merge
closed: 2026-11-27
closed_by: agent:taskfleet-worker
---

# Post-merge push in worker never runs: teardown follows the merge report

## Description

Workers told to `run merge` and then push main and verify the remote SHA consistently did not push. On 2026-11-26 on Haapa, all four self-merging spinoffs that had this step (intakectl 01m8b0ccby33j1atfpcj5h2yjp, 01m8b32nt11vf6n84p9yyfbby2, 01m8b6b4yfw4qpavgzgtwc2t5v, and taskfleet 01m8b6b4bn90wv2mzgrasbnccf) ended with local main ahead of origin by the merged commits. Their reports said a closing shell would push after the merge and verify, and some warned that teardown might interrupt it. That suggests the supervisor tears down the worker (pane/process group) as soon as the terminal `explicit-merge` report lands, which kills a post-merge step in the same shell. The conductor pushed by hand each time.

Expected: either `run merge` gains an opt-in push of the integrated source branch (as part of the merge transaction, recorded in the report), or the bundled skills stop telling workers to push after merge and tell the conductor to push instead. A post-merge step that is silently killed should not be the documented path.
