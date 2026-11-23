---
created: 2026-11-23
updated: 2026-11-23
type: bug
reporter: jari
status: untriaged
priority: normal
provenance: agent:homebase-wrapup
source_ref: agent:homebase-wrapup/reporter:jari/id:salvage-midrebase-20261123
---

# run salvage leaves the preserved worktree mid-rebase after merge_failed

## Description

run salvage leaves the preserved worktree mid-rebase after merge_failed

`taskfleet run salvage 01m829wf5hz4sv5cm6b1h8vbj3 --report-file … --output json` (taskfleet on Trude, 23.11.2026) returned
`{"error":{"code":"merge_failed","message":"git merge failed … git [\"rebase\", \"--\", \"main\"]: … could not apply 28355dde9 …"}}`.
Afterwards the preserved worktree was left in an interactive rebase in progress (`git status`: "interactive rebase in progress; onto 2183d5971"), with conflicted files. The run stayed `failed`. I had to `git rebase --abort` in the worktree by hand before a harvest worker could read the branch.

Expected: on merge_failed, salvage (like run merge for a live worker) leaves the preserved worktree as it found it (abort the rebase) or clearly reports that the worktree is mid-rebase and how to resume/abort, so the preserved branch is not silently left in a half-rebased state.

Reported by jari via agent:homebase-wrapup.
