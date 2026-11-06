#!/usr/bin/env -S RUST_LOG=warn uv run -q --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["textstat==0.7.3", "setuptools<70"]
# ///
"""Checks the prose a change adds: proselint findings and Flesch reading ease (aim above 50).

    prose-check.py <base>..<head> [<path>...]   # lines added to *.md files between two commits
    prose-check.py HEAD [<path>...]             # lines added since the last commit (not committed yet)
    prose-check.py <file>...                    # whole files (Markdown or text)

Code blocks, inline code, tables, link targets and HTML are left out. textstat is pinned to 0.7.3 (pyphen based):
newer versions need NLTK data that this machine refuses to load.
"""

import os
import re
import subprocess
import sys
import tempfile

import textstat

MIN_EASE = 50  # whole text
LINE_EASE = 30  # single lines worth rewriting


def clean(line: str) -> str:
    """Markdown line -> plain sentence text ('' when nothing to check)."""
    s = line.strip()
    if not s or s.startswith(("|", "<!--", "<", "---", "```", "~~~")):
        return ""
    s = re.sub(r"^#{1,6}\s+", "", s)
    s = re.sub(r"^(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s+)?", "", s)
    s = re.sub(r"^>\s?", "", s)
    s = re.sub(r"!\[([^\]]*)\]\([^)]*\)", r"\1", s)
    s = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", s)
    s = re.sub(r"`[^`]*`", "it", s)  # inline code reads as one short word
    s = re.sub(r"https?://\S+", "link", s)
    s = re.sub(r"[*_]{1,3}([^*_]+)[*_]{1,3}", r"\1", s)
    s = re.sub(r"<[^>]+>", "", s)
    return s.strip()


def from_text(name: str, lines: list[tuple[int, str]], out: list[tuple[str, int, str]], markdown: bool):
    fence = False
    for num, raw in lines:
        if markdown and raw.lstrip().startswith(("```", "~~~")):
            fence = not fence
            continue
        if fence:
            continue
        text = clean(raw) if markdown else raw.strip()
        if text:
            out.append((name, num, text))


def from_range(rng: str, paths: list[str]) -> list[tuple[str, int, str]]:
    spec = [f"{p.rstrip('/')}/*.md" if os.path.isdir(p) else p for p in paths] or ["*.md"]
    diff = subprocess.run(["git", "diff", "--unified=0", "--no-color", rng, "--", *spec], capture_output=True, text=True, check=True).stdout
    per_file: dict[str, list[tuple[int, str]]] = {}
    name, num = None, 0
    for line in diff.splitlines():
        if line.startswith("+++ "):
            name = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("@@"):
            num = int(re.search(r"\+(\d+)", line).group(1))
        elif name and line.startswith("+"):
            per_file.setdefault(name, []).append((num, line[1:]))
            num += 1
    out: list[tuple[str, int, str]] = []
    for name, lines in per_file.items():
        from_text(name, lines, out, markdown=True)
    return out


def main(args: list[str]) -> int:
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    items: list[tuple[str, int, str]] = []
    if not os.path.exists(args[0]):
        items += from_range(args[0], args[1:])
    else:
        for arg in args:
            with open(arg, encoding="utf-8") as f:
                lines = list(enumerate(f.read().splitlines(), 1))
            from_text(arg, lines, items, markdown=not arg.endswith(".txt"))
    if not items:
        print("prose-check: no prose to check")
        return 0

    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as tmp:
        tmp.write("\n".join(text for _, _, text in items) + "\n")
    findings = 0
    try:
        env = {k: v for k, v in os.environ.items() if k != "RUST_LOG"}
        res = subprocess.run(["uvx", "-q", "proselint", "check", tmp.name], capture_output=True, text=True, env=env)
        for line in res.stdout.splitlines():
            m = re.match(r".*?:(\d+):(\d+): (.*)", line)
            # "Resume" is a button label here, not a résumé.
            if not m or "'résumé'" in m.group(3):
                continue
            name, num, _ = items[int(m.group(1)) - 1]
            print(f"{name}:{num}: {m.group(3)}")
            findings += 1
    finally:
        os.unlink(tmp.name)

    for name, num, text in items:
        if len(text.split()) >= 12 and textstat.flesch_reading_ease(text) < LINE_EASE:
            print(f"{name}:{num}: hard to read (ease {textstat.flesch_reading_ease(text):.0f}): {text[:90]}…")
    ease = textstat.flesch_reading_ease(" ".join(text for _, _, text in items))
    print(f"prose-check: {len(items)} lines, {findings} proselint findings, reading ease {ease:.0f} (aim above {MIN_EASE})")
    return 1 if findings or ease < MIN_EASE else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
