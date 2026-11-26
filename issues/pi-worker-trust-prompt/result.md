# Result

Every Taskfleet-written Pi launcher (initial create, supervisor retry, resident
round) now appends Pi's one-run trust override, decided by
`crates/taskfleet/src/harness/pi_trust.rs`:

- The nearest entry in Pi's trust store for the run's recorded source checkout
  (`$PI_CODING_AGENT_DIR` with Pi's `~`/`~/`/`file://` forms, else
  `~/.pi/agent`, then `trust.json`; canonical source path, `null` entries
  skipped, keys resolved like Node's `path.resolve`): `true` gives
  `--approve`, `false` gives `--no-approve`.
- No decision, an unreadable or invalid store, a relative agent dir, or no
  recorded source: `--no-approve` for an autonomous worker, no flag for an
  interactive one (its person answers Pi's prompt).
- A trust flag already in the profile's Pi argv (`--approve`, `-a`,
  `--no-approve`, `-na`) stands; nothing is added.
- Nothing is written to Pi's trust store. The generated inner launcher carries
  a path-free `# Pi project trust: …` comment saying which rule applied.

Pi's flags were checked against the installed Pi 1.5.2 package
(`dist/cli/args.js`, `dist/core/trust-manager.js`).

## Verified

- Unit tests for the decision table (trusted, distrusted, nearest ancestor,
  `null` entry, entry below the source, missing/empty/invalid store, BOM,
  agent-dir forms, symlinked source, profile flag stands).
- Launcher test across trusted / distrusted / undecided × autonomous /
  interactive.
- The materialized `run create` integration test now proves `--approve` reaches
  the exact worker argv for a trusted source; the supervisor retry test does
  the same for a retry.
- `scripts/validate-local-release.sh` passed on the implementation commit.

## Not verified

- A real Pi worker against a repository with `.agents/skills` in a store
  without an entry (no live Pi run was started from this worktree).
- The decision is read in the process that writes the launcher (`run create`,
  the supervisor, or `run message`), so a `PI_CODING_AGENT_DIR` that differs
  between that process and the worker's tmux environment is not reconciled.
- When the source is undecided, Pi's own decision for the worktree path (an
  entry on a worktree-only ancestor such as `<repo>__worktrees`) is not
  consulted, because the worktree path is not known when the launcher is
  written; such a worker gets `--no-approve`.
