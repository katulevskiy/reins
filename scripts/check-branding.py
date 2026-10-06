#!/usr/bin/env python3
"""Reject obsolete branding in tracked file names and contents."""

from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
# The character class lets the check inspect its own source without matching itself.
OBSOLETE = re.compile(rb"re[w]arden", re.IGNORECASE)


def main():
    names = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).split(b"\0")
    failures = []
    for name in filter(None, names):
        path = ROOT / name.decode()
        if OBSOLETE.search(name) or (path.is_file() and OBSOLETE.search(path.read_bytes())):
            failures.append(name.decode())
    if failures:
        print("Obsolete branding in tracked files:\n" + "\n".join(failures), file=sys.stderr)
        return 1
    print("Reins branding check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
