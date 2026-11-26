# Result

The bundled conductor skills learned the narrative-root / product-root split
as a new `## Narrative root and product root` section in `stint-start`,
`stint-handoff` and `night-stint-prep`, and a `### A caller in a separate
narrative root` subsection in `worktree-spinoff` (issue resolution and run
creation). A project is split only when its instructions, handoff, or the user
name the product root explicitly; otherwise every section says nothing
changes, so single-repository behaviour is the existing text untouched.

Covered: `issuectl --root <product>` for DAG reads and slug resolution;
`run create --source-repo <abs product> --source-branch <branch>` and
`run list --repo <product>` for spawns and holds; driver children follow the
product because the driver works in a product worktree; worker filing,
lifecycle, full run ids and the single `run merge` unchanged, with no change
to the generated intake policy; optional private output only under
`<narrative root>/worker-results/<full-run-id>/`, never staged by the worker,
committed by the conductor by explicit paths after verifying the product
landing; recovery after interruption from the run id rather than a re-run;
the `history/.worktree/` brief copy and its ignore rule left alone.

Verified: the flags the text relies on exist (`issuectl --root`,
`run create --source-repo`/`--source-branch`, `run list --repo` accepting a
path); skill example, catalog and contract tests pass with one reviewed
snapshot change (`skill__handoff_independent_preservation`, which spans the
new handoff section); the full `scripts/validate-local-release.sh` gate passed
on the implementation commit.

Not verified: no end-to-end split-project round was run (a real narrative
repo plus product repo with a conductor agent following the skills); the
acceptance scenarios are covered by instruction text, not by an executed
trial. The released 0.21.0 CHANGELOG already carried an `Added` line naming
this issue before any implementation existed; it was left as released, and
the real change is described under `[Unreleased]`.
