<!--
Thanks for contributing to taskfleet! Please read CONTRIBUTING.md first.
Keep PRs focused — one concern per commit, one issue per PR.
-->

## What & why

<!-- What does this change and why? Link the issue it addresses. -->

Closes: <!-- #<gh-issue>, or "n/a (typo fix)" -->

## Green gate

All must pass locally before review; there is no hosted CI, and maintainers
run the complete gate (`scripts/validate-local-release.sh`) before merging:

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`

## Checklist

- [ ] Linked the GitHub issue or discussion for anything beyond a typo fix
      (see [CONTRIBUTING.md](../CONTRIBUTING.md)).
- [ ] New commands/flags follow the AI-first CLI canon (the
      `/ai-first-cli-canon` skill) — `--json` output,
      JSONL logs, strict input validation, informative error envelopes, no
      interactive prompts.
- [ ] CLI-surface or `SKILL.template.md` changes: validated with repository-local
      artifacts and the documented insta snapshot loop; no installed taskfleet
      binary or bundled-skill set was changed for this work.
- [ ] Commits are focused (one concern each).

## Notes for reviewers

<!-- Anything worth flagging: trade-offs, follow-ups, areas wanting extra eyes. -->
