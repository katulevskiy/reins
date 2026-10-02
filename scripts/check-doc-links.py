#!/usr/bin/env python3
"""Checks that the relative links in the public Markdown docs resolve to files in the repository.

    scripts/check-doc-links.py                 # README.md, the top-level *.md and docs/**/*.md
    scripts/check-doc-links.py FILE.md ...     # just these

Markdown links and images, reference definitions and HTML href/src attributes are checked. A link to a Markdown file
with a #fragment must name one of its headings (GitHub's anchor rules). External links (http:, mailto:, ...) are not
fetched. Exits 1 and lists the broken links if there are any.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

INLINE = re.compile(r"!?\[(?:[^\[\]]|\[[^\]]*\])*\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
REFERENCE = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*<?(\S+?)>?(?:\s+.*)?$", re.M)
HTML = re.compile(r"""\b(?:href|src)\s*=\s*["']([^"']+)["']""")
FENCE = re.compile(r"^\s*(```|~~~).*?^\s*\1", re.M | re.S)
CODE_SPAN = re.compile(r"`[^`\n]*`")
SCHEME = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")


def default_files():
    files = sorted(ROOT.glob("*.md")) + sorted((ROOT / "docs").rglob("*.md"))
    return [f for f in files if f.is_file()]


def anchors(path, cache={}):
    if path not in cache:
        text = FENCE.sub("", path.read_text(encoding="utf-8"))
        seen, found = {}, set()
        for line in text.splitlines():
            m = re.match(r"^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$", line)
            if not m:
                continue
            title = re.sub(r"<[^>]+>", "", m.group(1))
            title = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", title)
            slug = re.sub(r"[^\w\- ]", "", title.lower().replace("`", "")).replace(" ", "-")
            n = seen.get(slug, 0)
            seen[slug] = n + 1
            found.add(slug if n == 0 else f"{slug}-{n}")
        found.update(re.findall(r"""<a\s+(?:name|id)=["']([^"']+)["']""", text))
        cache[path] = found
    return cache[path]


def check(md):
    text = md.read_text(encoding="utf-8")
    body = CODE_SPAN.sub("", FENCE.sub("", text))
    targets = [m.group(1) for r in (INLINE, REFERENCE, HTML) for m in r.finditer(body)]
    broken = []
    for target in targets:
        if SCHEME.match(target) or target.startswith("//"):
            continue
        path_part, _, fragment = target.partition("#")
        path_part = path_part.split("?", 1)[0]
        if path_part:
            dest = (ROOT / path_part.lstrip("/")) if path_part.startswith("/") else (md.parent / path_part)
            dest = dest.resolve()
            if not dest.exists():
                broken.append(f"{target} (no such file)")
                continue
            if ROOT not in dest.parents and dest != ROOT:
                broken.append(f"{target} (outside the repository)")
                continue
        else:
            dest = md
        if fragment and dest.suffix == ".md" and fragment.lower() not in anchors(dest):
            broken.append(f"{target} (no heading #{fragment} in {dest.relative_to(ROOT)})")
    return broken


def main(args):
    files = [Path(a).resolve() for a in args] or default_files()
    failed = 0
    for md in files:
        for problem in check(md):
            print(f"{md.relative_to(ROOT)}: {problem}")
            failed += 1
    print(f"{len(files)} files checked, {failed} broken link(s)", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
