#!/usr/bin/env python3
"""Create or update the labels declared in .github/labels.yml.

    python3 -I .github/sync-labels.py --check          # validate the file, do not call GitHub
    python3 -I .github/sync-labels.py                  # apply them with the gh CLI

The workflow .github/workflows/sync-labels.yml runs the second form. Labels that are
not in the file are left alone, including GitHub's defaults. A rename in the file
creates the new name and leaves the old one; delete the old label by hand. If GitHub
rejects a label, labels earlier in the file are already updated; run it again after
fixing the file.

The file format is deliberate and small, so this script does not need PyYAML. Each
label is three lines, in order, with double-quoted values and no escapes:

    - name: "needs-triage"
      color: "f9d0c4"
      description: "New bug report, not yet reviewed"

Comments and blank lines are allowed between labels, not inside one.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

DEFAULT_FILE = Path(__file__).resolve().parent / "labels.yml"

NAME_RE = re.compile(r"^[a-z0-9][a-z0-9:-]{0,49}$")
COLOR_RE = re.compile(r"^[0-9a-f]{6}$")
DESCRIPTION_MAX = 100
NAME_LINE = re.compile(r'^- name: "([^"\\]*)"$')
COLOR_LINE = re.compile(r'^  color: "([^"\\]*)"$')
DESCRIPTION_LINE = re.compile(r'^  description: "([^"\\]*)"$')


@dataclass(frozen=True)
class Label:
    name: str
    color: str
    description: str


class FormatError(Exception):
    pass


def fail(line_no: int, message: str) -> None:
    raise FormatError(f".github/labels.yml:{line_no}: {message}")


def parse_labels(text: str) -> list[Label]:
    """Parse the label file. Raises FormatError with a line number."""
    labels: list[Label] = []
    seen: set[str] = set()
    name: str | None = None
    color: str | None = None
    # "item" expects a new label, a comment, or a blank line.
    # "color" and "description" expect that field next.
    stage = "item"
    started_at = 0

    for line_no, raw in enumerate(text.splitlines(), 1):
        line = raw[:-1] if raw.endswith("\r") else raw
        stripped = line.strip()
        if stripped == "" or stripped.startswith("#"):
            if stage != "item":
                fail(line_no, "comment or blank line inside a label")
            continue
        if stage == "item":
            match = NAME_LINE.fullmatch(line)
            if not match:
                fail(line_no, 'expected `- name: "..."`')
            name = match.group(1)
            color = None
            stage = "color"
            started_at = line_no
            continue
        if stage == "color":
            match = COLOR_LINE.fullmatch(line)
            if not match:
                fail(line_no, 'expected `  color: "..."`')
            color = match.group(1)
            stage = "description"
            continue
        if stage != "description":
            fail(line_no, "internal: unexpected parser state")
        match = DESCRIPTION_LINE.fullmatch(line)
        if not match:
            fail(line_no, 'expected `  description: "..."`')
        if name is None or color is None:
            fail(line_no, "internal: label fields missing")
        description = match.group(1)
        if not NAME_RE.fullmatch(name):
            fail(started_at, f"label name {name!r} is empty, too long, or has characters outside a-z 0-9 : -")
        if name in seen:
            fail(started_at, f"duplicate label {name!r}")
        if not COLOR_RE.fullmatch(color):
            fail(started_at, f"color {color!r} must be six lowercase hex digits")
        if not description or len(description) > DESCRIPTION_MAX:
            fail(line_no, f"description must be 1 to {DESCRIPTION_MAX} characters (got {len(description)})")
        seen.add(name)
        labels.append(Label(name, color, description))
        name = None
        color = None
        stage = "item"

    if stage != "item":
        fail(started_at, "label is missing color or description")
    if not labels:
        raise FormatError(".github/labels.yml: no labels")
    return labels


def apply_labels(labels: list[Label], repo: str) -> None:
    if not repo:
        raise SystemExit("Set GITHUB_REPOSITORY or pass --repo OWNER/REPO")
    for label in labels:
        print(f"sync {label.name}")
        try:
            result = subprocess.run(
                [
                    "gh",
                    "label",
                    "create",
                    label.name,
                    "--repo",
                    repo,
                    "--color",
                    label.color,
                    "--description",
                    label.description,
                    "--force",
                ],
                check=False,
            )
        except FileNotFoundError:
            raise SystemExit("gh is not installed") from None
        if result.returncode != 0:
            raise SystemExit(result.returncode)
    print(f"{len(labels)} labels synced")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Create or update labels from .github/labels.yml")
    parser.add_argument("--check", action="store_true", help="validate the file and do not call GitHub")
    parser.add_argument("--file", type=Path, default=DEFAULT_FILE, help="label file (default: .github/labels.yml)")
    parser.add_argument(
        "--repo",
        default=os.environ.get("GITHUB_REPOSITORY", ""),
        help="OWNER/REPO for gh (default: GITHUB_REPOSITORY)",
    )
    args = parser.parse_args(argv)
    try:
        labels = parse_labels(args.file.read_text(encoding="utf-8"))
    except FormatError as error:
        print(error, file=sys.stderr)
        return 1
    except OSError as error:
        print(error, file=sys.stderr)
        return 1
    if args.check:
        for label in labels:
            print(f"{label.name}\t{label.color}\t{label.description}")
        print(f"{len(labels)} labels ok", file=sys.stderr)
        return 0
    apply_labels(labels, args.repo)
    return 0


if __name__ == "__main__":
    sys.exit(main())
