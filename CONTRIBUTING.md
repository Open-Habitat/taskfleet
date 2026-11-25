# Contributing to taskfleet

Issues and pull requests are welcome. The repo is small and the
contribution loop is short; please
read this file before opening either.

## Before you file an issue

- Run `taskfleet version` and `taskfleet doctor` and include
  the output. `doctor` reports `63 ok / 0 fail` on a clean install — any
  non-zero `fail` count is worth surfacing.
- Report bugs and feature requests as GitHub Issues. For workflow questions
  ("how do I do X with `/worktree-*`"), open a GitHub Discussion instead.
- Search the open GitHub Issues and the accepted work under
  `issues/<slug>/` first; a known gap may already be tracked.

## Before you open a pull request

- For anything more than a typo fix, open a GitHub Issue or Discussion
  first (or reference an existing one) so the decision conversation has a
  home. Maintainers accept work into the repository's
  [`issuectl`](https://github.com/jarimustonen/issuectl) tracker under
  `issues/`, filed with the report's URL as its source, and commits carry a
  `Refs-Issue:` or `Fixes-Issue:` trailer naming that issue.
- All code must pass:
  ```bash
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test --workspace
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
  ```
  There is no hosted CI. Maintainers run the complete gate,
  `scripts/validate-local-release.sh`, on the exact commit before it lands.
- Use synthetic hosts, people, home paths and accounts in everything you
  write for the project (issues, commit messages, fixtures, branch names),
  never those of a real installation.
- New commands or flags must follow the AI-first conventions documented
  in `/ai-first-cli-canon` — `--json` output,
  JSONL logs, strict input validation, informative error envelopes, no
  interactive prompts. New SKILL.md examples are test-gated against the
  actual binary CLI surface.
- Keep commits focused. One concern per commit; one issue per PR.

## Repo layout

- `crates/taskfleet-core/` — schema, file I/O, locking, supervisor protocol.
  Library crate; new public items get rustdoc (the crate carries
  `#![warn(missing_docs)]`).
- `crates/taskfleet/` — the `taskfleet` binary and the bundled
  SKILL.md files (`crates/taskfleet/skills/`). Validate CLI-surface edits with a
  repository-local build and explicit `./target/release/taskfleet …`
  invocations. For `SKILL.template.md` edits, run and review the insta snapshot
  loop documented in `crates/taskfleet/AGENTS.md`; no install is needed.
- `issues/<slug>/item.md` — accepted open work (flat layout, status in
  frontmatter). A closed issue or an implemented decision record is
  removed from the tree once the code carries it; read it with
  `scripts/from-history.sh <path>` or `git log --all -- <path>`.
- `/ai-first-cli-canon` — externally maintained CLI design canon; treat the
  installed skill as canonical, not a project-local copy.

## License

By contributing, you agree your contributions will be licensed under the
[MIT License](LICENSE).
