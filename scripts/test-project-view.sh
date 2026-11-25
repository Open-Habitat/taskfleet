#!/usr/bin/env bash
# Regression: validate-project-view.py accepts a clean tree and rejects each
# way a living document can point at something the tree no longer holds.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/taskfleet-project-view.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

fixture="$tmp/repo"
mkdir -p "$fixture/scripts" "$fixture/docs" "$fixture/issues/open-work" "$fixture/issues/closed-work"
cp "$repo_root/scripts/validate-project-view.py" "$fixture/scripts/"
cd "$fixture"
git init -q
git config user.email fixture@example.org
git config user.name Fixture
printf 'history/\n' >.gitignore
cat >AGENTS.md <<'EOF'
# Fixture
Read [the guide](docs/guide.md). A removed ADR is a plain path:
`docs/decisions/0001-example.md` in history.

```text
[not a link](missing-inside-a-fence.md)
```
EOF
printf '# Guide\nBack to [the instructions](../AGENTS.md#fixture).\n' >docs/guide.md
printf -- '---\nstatus: open\n---\nSee [the guide](../../docs/guide.md).\n' >issues/open-work/item.md
printf -- '---\nstatus: done\n---\nOld [link](../../docs/removed.md).\n' >issues/closed-work/item.md
git add -A
git commit -q -m fixture

expect_pass() {
  python3 scripts/validate-project-view.py >/dev/null 2>"$tmp/stderr" || {
    echo "project view rejected a valid tree ($1):" >&2
    cat "$tmp/stderr" >&2
    exit 1
  }
}

expect_fail() {
  local label=$1 needle=$2
  if python3 scripts/validate-project-view.py >/dev/null 2>"$tmp/stderr"; then
    echo "project view accepted $label" >&2
    exit 1
  fi
  grep -F -- "$needle" "$tmp/stderr" >/dev/null || {
    echo "project view rejected $label without naming it ($needle):" >&2
    cat "$tmp/stderr" >&2
    exit 1
  }
}

expect_pass 'clean fixture'

printf 'Rule from `issues/open-work/item.md`.\n' >>AGENTS.md
expect_fail 'an issue path in an instruction' 'AGENTS.md: instructions name no issue: issues/open-work'
git checkout -q -- AGENTS.md

mkdir -p crates/tool/skills/example
printf 'See issues/closed-work for why.\n' >crates/tool/skills/example/SKILL.template.md
git add -A
expect_fail 'an issue path in a bundled skill' 'SKILL.template.md: instructions name no issue'
git rm -q -r --cached crates && rm -rf crates

printf 'See [gone](docs/gone.md).\n' >>docs/guide.md
expect_fail 'a dangling link' 'docs/guide.md: missing local link target: docs/gone.md'
git checkout -q -- docs/guide.md

printf 'Now [dangling](missing.md).\n' >>issues/open-work/item.md
expect_fail 'a dangling link in an open issue' 'issues/open-work/item.md: missing local link target: missing.md'
git checkout -q -- issues/open-work/item.md

printf '# TODO\n' >TODO.md
git add TODO.md
expect_fail 'a tracked TODO.md' 'TODO.md: the session handoff is not kept in this repository'
git rm -q --cached TODO.md && rm TODO.md

mkdir -p history
printf 'brief\n' >history/brief.md
git add -f history/brief.md
expect_fail 'a tracked history/ file' 'history/brief.md: history/ is agent scratch'
git rm -q --cached history/brief.md && rm -rf history

expect_pass 'restored fixture'
echo 'project view regression passed'
