# Result

`scripts/shipshape-release.sh` no longer aborts a held cut when origin/main
gained only intake commits. When every commit between the bump commit's parent
and origin/main touches only `issues/` (first-parent diff and net diff, renames
counted as delete + add), main becomes a merge of the journalled bump commit
and that intake (`git merge-tree` + `commit-tree`, pushed without force), and
the merge is the commit the exact-HEAD local gate validates. Intake that lands
during the gate, on top of a pushed release main, is adopted and validated
again, for at most three rounds. Any other movement is refused as before.

Deviation from the issue text: the bump commit is merged, not rebased. The
Shipshape journal, the held local tag, the authorization ref, and Shipshape's
`advance_branch` all bind the release to the journalled bump commit; a rebased
commit would leave them pointing at a commit main no longer contains. The tag
stays on the bump commit, whose code tree equals the validated merge outside
`issues/`.

## Verified

- `scripts/test-shipshape-release-current-protocol.sh` (real Shipshape 0.12.4,
  disposable origin) now replays the race via the wrapper's `resume`: a code
  commit on the pre-bump base is refused before validation; an issue-only
  commit is merged (parents: bump, intake) and the merge validated; later
  intake on the pushed merge is adopted and validated; a code commit on top is
  refused; the held tag stays on the bump commit and unpublished throughout;
  Shipshape's own resume then completes from the merged main. A mutation that
  rejects every path made the fixture fail as expected.
- `scripts/test-shipshape-release-held-tag.sh` (stubbed git) still passes.
- Full `scripts/validate-local-release.sh` green on 7a5ec78e, which the close stamp then rewrote to 0f4de5b2 with an identical tree.

## Not verified

- A real production cut that loses the race (only the disposable fixture).
- Shipshape `advance_branch` against a merged main in a real run: the fixture
  covers it only through its separate tag-only advance fixture and the source
  of `RealTagger::advance_branch` (an origin branch that already contains the
  release commit is idempotent success).
