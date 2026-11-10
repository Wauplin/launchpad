"""Bumps the agent CLI pins of Home's image and Burst jobs in release.json to the newest npm release of the same major
version (semver; for 0.x the same minor), like runners do on their own (launchpad_setup::cli_update). A new major version
is only reported; pass --major to pin it. Run it from the repository root, review the diff, then commit:
the next release (or main package) builds the image with the new pins.

    python3 deploy/hf-home/bump_tools.py [--major] [--dry-run]
"""
import argparse
import json
import re
import urllib.request
from pathlib import Path

PACKAGES = {"claude": "@anthropic-ai/claude-code", "codex": "@openai/codex", "opencode": "opencode-ai"}
RELEASE = Path(__file__).with_name("release.json")
SEMVER = re.compile(r"v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?")


def parse(v):
    m = SEMVER.fullmatch(v.strip())
    return (int(m[1]), int(m[2]), int(m[3]), m[4] or "") if m else None


def line(v):
    """The versions compatible with v: same major; for 0.x the same minor; for 0.0.x only v."""
    major, minor, patch, _ = v
    return (0, 0, patch) if major == 0 and minor == 0 else (0, minor, 0) if major == 0 else (major, 0, 0)


def key(v):
    return (v[0], v[1], v[2], v[3] == "", v[3])


def plan(installed, latest, versions=()):
    """(auto, major): the newest compatible release, and latest when it is a new major version."""
    i, l = parse(installed), parse(latest)
    if not i or not l or key(l) <= key(i) or l[3]:
        return None, None
    if line(l) == line(i):
        return latest, None
    ok = [(parse(v), v) for v in versions]
    ok = [(p, v) for p, v in ok if p and not p[3] and line(p) == line(i) and key(p) > key(i)]
    return (max(ok, key=lambda x: key(x[0]))[1] if ok else None), latest


def npm(path, accept="application/json"):
    req = urllib.request.Request(f"https://registry.npmjs.org/{path}", headers={"Accept": accept})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--major", action="store_true", help="also pin new major versions")
    ap.add_argument("--dry-run", action="store_true", help="print the changes, write nothing")
    args = ap.parse_args()
    data = json.loads(RELEASE.read_text())
    changed = False
    for tool, package in PACKAGES.items():
        pinned = data["tools"][tool]
        latest = npm(package.replace("/", "%2F") + "/latest")["version"]
        auto, major = plan(pinned, latest)
        if major:
            versions = npm(package.replace("/", "%2F"), "application/vnd.npm.install-v1+json")["versions"].keys()
            auto, major = plan(pinned, latest, versions)
        target = major if major and args.major else auto
        if major and not args.major:
            print(f"{tool}: {major} is a new major version (pinned {pinned}); --major pins it")
        if target:
            print(f"{tool}: {pinned} -> {target}")
            data["tools"][tool] = target
            changed = True
    if changed and not args.dry_run:
        RELEASE.write_text(json.dumps(data, indent=2) + "\n")
    if not changed:
        print("release.json is up to date")


if __name__ == "__main__":
    main()
