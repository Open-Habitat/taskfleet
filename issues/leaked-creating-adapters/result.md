# Result: terminal runs leak `.creating` control files

## Cause

A Pi launch keeps three per-run control files directly under
`<state root>/.creating`, outside the staging tree that publication renames:
`message-<run>.ts` (the adapter), `message-<run>-<attempt>.sock` and, for a
resident, `round-<run>-<attempt>.reported`. Terminal teardown removed them
only from inside `supervise/evidence.rs` after a successful evidence archive
(`remove_message_adapter`). Every other way out of the evidence step left
them behind:

- the node had no Pi evidence projection (the early `Ok(Some(_)) => true`
  return the read-only hint pointed at);
- capture failed, so the supervisor kept preserving and retrying, and the
  files stayed until somebody killed the supervisor.

The original report's run confirms the second case. Its event log on the
reporting host shows `run.status done`, then `worker.evidence.failed`
("tmux capture-pane exited Some(1): can't find pane"), then 18 minutes of
retries until `supervisor.exited` on SIGTERM. The same loop is the
"supervisor stayed alive after `done`" observation.

On the upgrade side, `run upgrade-lock` already attributed `message-*` files
of a published run to that run, but did not recognize `round-*.reported` at
all. Any round marker failed the survey as an unknown `.creating` entry
before `--abandon-stale-creations` could act, which is why the 27.11.
upgrade cleared none of the leftovers.

## What changed

- New `run/control_files.rs` holds the one exact parser for the three names
  (canonical run id, decimal attempt) and a sweep that removes one terminal
  run's control files. It removes plain files and sockets only.
- `supervise/cleanup.rs` `cleanup_terminal_nodes` runs that sweep first on
  every terminal teardown tick for native (non-caller) runs, independent of
  evidence. The files are not evidence inputs, and a terminal run has no input
  channel. The sweep applies only to a run laid out at `<root>/runs/<id>`, so
  a fixture with any other layout never touches a guessed `.creating`. Live
  Pi session transcripts under `.creating/pi-sessions/<id>` are still kept
  while evidence is pending, as before.
- `run/upgrade_survey.rs` uses the shared parser. A round marker of a
  published run is ignored like its adapter: a live run still refuses
  through the run survey, and a terminal run's leftover is inert. A round
  marker whose run does not exist is an unpublished creation, so
  `--abandon-stale-creations` moves it aside after the usual probes.
- `docs/UPGRADE-ADMISSION.md` and the CHANGELOG describe this.

## Evidence

Each new test fails on the old code and passes now (checked by disabling the
change locally):

- `supervise::cleanup::tests::terminal_cleanup_removes_control_files_without_evidence`:
  a failed node with no evidence projection, torn down through
  `cleanup_terminal_nodes`. Its adapter, socket and round marker go, and
  another run's adapter stays.
- `tests/resident.rs::failed_one_shot_pi_run_leaves_no_message_adapter`:
  a one-shot Pi spinoff through the real materializer, launcher and
  supervisor, failing without a merge while evidence capture fails. The old
  code leaked `message-<id>.ts` and `message-<id>-0.sock`.
- `tests/resident.rs::terminal_resident_leaves_no_round_marker_even_when_evidence_is_pending`:
  a Pi resident cancelled between rounds while evidence capture fails. The
  round marker, adapter and socket go.
- The merged-resident and dormant-cancelled resident tests now also assert
  that nothing of the run remains under `.creating`, the session store
  included.
- `tests/upgrade_lock_overrides.rs::leaked_control_files_of_terminal_runs_do_not_block_activation`:
  a cancelled run's adapter, socket and round marker do not block a plain
  `upgrade-lock`, which also leaves them untouched. A live run carrying the
  same files refuses as `upgrade_not_quiescent`. A round marker of a
  nonexistent run and an empty orphan `.creating/runs/<id>` refuse
  `upgrade_state_unverifiable` without the override and move aside with it.
- `run::control_files::tests`: exact name parsing, and a sweep that is
  scoped to one run.

## Supervisor alive after `done`

This does not reproduce on current code. The 0.17.0 case was the
evidence retry loop on a vanished pane (see Cause). Since v0.21.1
(a65a2910, "do not block teardown on unavailable pane evidence"), a pane
that is gone is recorded once as a durable diagnostic and evidence completes.
`supervise::evidence::tests::unavailable_pane_is_not_recaptured_after_archive_retry`
covers that path. A supervisor still retries indefinitely when the transcript
archive itself fails, because that preserves the only copy of the session.
That is by design and was not changed. Now it no longer also leaks the
control files.

## Decisions for later review

- **Ignore leftovers, don't prune them in `upgrade-lock`.** Leftovers of
  published terminal runs are ignored, not removed: the survey stays read-only
  and nothing removes state in the gate. Files that older releases left stay
  on disk (small files) until an operator removes them. Pruning them
  under admission EX after proving the run quiescent would be a small,
  additive follow-up if anyone wants a clean `.creating`.
- **Sweep only native runs.** The sweep runs for native runs only.
  Caller-owned runs never get these files from Taskfleet, and their teardown
  path stays untouched.
- **Old removal kept.** `evidence.rs` `remove_message_adapter` is left in
  place. It is now redundant on terminal paths but harmless, and keeping it
  limited the edits to the hot `supervise/*` files.
