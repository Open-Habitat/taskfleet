# Result

Took the skills-side direction; `run merge` is unchanged and does not push.

- Generated run context (every harness and kind) and the pi research closing
  now say `run merge` is the worker's last command, teardown kills anything
  after it, the worker must not push or verify the remote even if asked, and
  the parent pushes after confirming the landing. A unit test pins this.
- `worktree-spinoff`, `worktree-merge` and `fan-out` say the same; `stint-start`
  tells the conductor not to brief a post-merge push and adds a "Push" section:
  the conductor pushes after confirmed landings under remote policy.
- CHANGELOG `[Unreleased]` Fixed entry; the `skill_examples` portable-contract
  snapshot updated for the stint-start text.

Verified: full `scripts/validate-local-release.sh` passed on the implementation
commit. Not verified: the teardown-kills-post-merge-step mechanism itself was
taken from the issue's observations, not reproduced; no behaviour change
depends on it.
