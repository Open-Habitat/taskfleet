#!/usr/bin/env python3
"""Validate the repository's living documents without external dependencies.

The tree keeps only what the code cannot carry (root AGENTS.md, "Code is the
record"), so its documents must not point at files that are gone:

- every local Markdown link in a living document resolves to a file that
  exists; living documents are tracked Markdown outside issues/ plus the files
  of issues that are not closed. A reference into git history is a plain path,
  never a link;
- instruction files (AGENTS.md, AGENTS-*.md, bundled SKILL.template.md) name
  no issue path, because the issue will be removed and the instruction will
  not;
- no TODO.md session handoff is tracked: the session narrative is kept outside
  this repository;
- history/ stays ignored and untracked, because worker briefs land there.

Exit 0 when valid, 1 with one ERROR line per finding on stderr.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
CLOSED = {"done", "fixed", "obsolete", "wontfix", "duplicate", "cannot-reproduce"}
LINK = re.compile(r"!?(?<!\\)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
FENCE = re.compile(r"^(```|~~~).*?^\1", re.M | re.S)
ISSUE_PATH = re.compile(r"issues/[a-z0-9][a-z0-9-]*")


def git_files(*args: str) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z", *args],
        capture_output=True,
        check=True,
    ).stdout.decode()
    return [name for name in out.split("\0") if name]


def issue_is_open(slug: str, cache: dict[str, bool]) -> bool:
    if slug not in cache:
        item = ROOT / "issues" / slug / "item.md"
        text = item.read_text(encoding="utf-8") if item.is_file() else ""
        status = re.search(r"^status:\s*(\S+)", text, re.M)
        cache[slug] = bool(status) and status.group(1) not in CLOSED
    return cache[slug]


def living_markdown() -> list[Path]:
    """Markdown the tree keeps current: everything except closed issues."""
    result = []
    cache: dict[str, bool] = {}
    for name in git_files("--cached", "--others", "--exclude-standard", "*.md"):
        path = ROOT / name
        if not path.is_file() or path.is_symlink():
            continue
        parts = Path(name).parts
        if parts[0] == "issues" and len(parts) > 2 and not issue_is_open(parts[1], cache):
            continue
        result.append(path)
    return result


def is_instruction(path: Path) -> bool:
    rel = path.relative_to(ROOT)
    if rel.parts[0] == "issues":
        return False
    return (
        path.name == "AGENTS.md"
        or (path.name.startswith("AGENTS-") and path.suffix == ".md")
        or path.name == "SKILL.template.md"
    )


def check_link(errors: list[str], page: Path, raw: str) -> None:
    target = raw.strip("<>").split("#", 1)[0].split("?", 1)[0]
    if not target or re.match(r"^[a-z][a-z0-9+.-]*:", target):
        return
    rel = page.relative_to(ROOT)
    if target.startswith("/"):
        errors.append(f"{rel}: absolute local link is not portable: {raw}")
        return
    resolved = (page.parent / unquote(target)).resolve()
    if not resolved.is_relative_to(ROOT):
        errors.append(f"{rel}: local link leaves the repository: {raw}")
    elif not resolved.exists():
        errors.append(f"{rel}: missing local link target: {raw}")


def main() -> int:
    errors: list[str] = []
    for page in living_markdown():
        text = page.read_text(encoding="utf-8", errors="ignore")
        for raw in LINK.findall(FENCE.sub("", text)):
            check_link(errors, page, raw)
        if is_instruction(page):
            for match in ISSUE_PATH.finditer(text):
                errors.append(
                    f"{page.relative_to(ROOT)}: instructions name no issue: {match.group(0)}"
                )

    for name in git_files("--cached", "--", "TODO.md"):
        errors.append(f"{name}: the session handoff is not kept in this repository")

    for name in git_files("--cached", "--", "history"):
        errors.append(f"{name}: history/ is agent scratch and must stay untracked")
    probe = subprocess.run(
        ["git", "-C", str(ROOT), "check-ignore", "-q", "--no-index", "history/.worktree/probe.md"],
        capture_output=True,
    )
    if probe.returncode != 0:
        errors.append(".gitignore must ignore history/ (worker briefs land in history/.worktree/)")

    if errors:
        for message in errors:
            print(f"ERROR: {message}", file=sys.stderr)
        print(f"Project view validation failed with {len(errors)} error(s).", file=sys.stderr)
        return 1
    print("Project view valid.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
