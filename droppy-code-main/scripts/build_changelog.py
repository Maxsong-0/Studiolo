#!/usr/bin/env python3
"""Builds website/changelog.json from ReleaseNotes/<version>.md.

Run: python3 scripts/build_changelog.py
Idempotent: rewrites website/changelog.json from scratch each run.
"""

import datetime
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NOTES_DIR = ROOT / "ReleaseNotes"
OUT = ROOT / "website" / "changelog.json"

HEADING_MAP = {
    "new features": "New features",
    "bug fixes": "Bug fixes",
    "refinements": "Refinements",
}
SECTION_ORDER = ["New features", "Bug fixes", "Refinements"]


def git_date(args):
    try:
        out = subprocess.run(
            ["git", *args], cwd=ROOT, capture_output=True, text=True, check=True
        )
        return out.stdout.strip() or None
    except Exception:
        return None


def release_date(version, rel_path):
    date = git_date(["log", "-1", "--format=%cs", f"v{version}"])
    if date:
        return date
    date = git_date(["log", "-1", "--format=%cs", "--", str(rel_path.relative_to(ROOT))])
    if date:
        return date
    return datetime.date.today().isoformat()


def strip_emphasis(text):
    return text.replace("**", "").replace("__", "").replace("*", "").replace("_", "")


def parse_notes(path):
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    summary_lines = []
    sections = {}
    current = None
    for line in lines:
        m = re.match(r"^##\s+(.*?)\s*$", line)
        if m:
            key = m.group(1).strip().lower()
            current = HEADING_MAP.get(key)
            if current and current not in sections:
                sections[current] = []
            elif current is None:
                current = None
            continue
        if current is None:
            if line.strip():
                summary_lines.append(line.strip())
        else:
            m2 = re.match(r"^\s*-\s+(.*)$", line)
            if m2:
                item = strip_emphasis(m2.group(1).strip())
                if item:
                    sections[current].append(item)
    summary = " ".join(summary_lines)
    ordered = [
        {"title": title, "items": sections[title]}
        for title in SECTION_ORDER
        if title in sections and sections[title]
    ]
    return summary, ordered


def version_key(version):
    nums = re.findall(r"\d+", version)
    return tuple(int(n) for n in nums)


def main():
    releases = []
    for path in sorted(NOTES_DIR.glob("*.md")):
        version = path.stem
        if not re.fullmatch(r"\d+(\.\d+)*", version):
            continue
        summary, sections = parse_notes(path)
        releases.append(
            {
                "version": version,
                "date": release_date(version, path),
                "platform": "mac",
                "summary": summary,
                "sections": sections,
            }
        )
    releases.sort(key=lambda r: version_key(r["version"]), reverse=True)
    payload = {
        "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "releases": releases,
    }
    OUT.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"Wrote {len(releases)} releases to {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    sys.exit(main())
