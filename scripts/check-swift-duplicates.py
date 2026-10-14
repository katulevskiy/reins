#!/usr/bin/env python3
"""Catch Swift declarations that two merged branches each added, which only an Xcode build would otherwise notice.

CI does not build the iOS app, and two pull requests that each add a `struct TimerPill` (or the same method twice after a
merge) both pass on their own and break `main` together. This checks, per Swift module:
- a top-level type (struct, class, enum, protocol, actor, typealias) declared twice without `private`/`fileprivate`;
- the same declaration line (a func or a nested type) repeated at the same indentation inside one file's same scope.
"""

import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IOS = ROOT / "ios"
# Which folders compile into which module (the app's targets in Reins.xcodeproj). Shared code goes into each of them.
MODULES = {
    "Reins": ["Reins", "Shared", "Core"],
    "ReinsWidgets": ["ReinsWidgets", "Shared"],
    "ReinsNotifications": ["ReinsNotifications", "Shared", "Core"],
    "ReinsTests": ["ReinsTests"],
    "ReinsUITests": ["ReinsUITests"],
}
TYPE = re.compile(
    r"^(?:@\w+(?:\([^)]*\))?\s+)*(?:(?:public|internal|open|private|fileprivate|final|indirect|nonisolated)\s+)*"
    r"(struct|class|enum|protocol|actor|typealias)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
DECL = re.compile(r"^(\s*)(?:@\w+(?:\([^)]*\))?\s+)*(?:(?:public|internal|private|fileprivate|static|override|mutating|nonisolated|final)\s+)*(func|struct|class|enum)\s+\S")


def swift_files(folders):
    for folder in folders:
        base = IOS / folder
        if base.is_dir():
            yield from sorted(p for p in base.rglob("*.swift") if "Vendor" not in p.parts)


def top_level_types(path):
    """Top-level type names, their line numbers and whether they are private to their file (`#else` alternatives are
    skipped: only one side of an `#if` is compiled)."""
    depth = 0
    if_depth = 0
    branch_seen = []
    for number, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
        stripped = line.strip()
        if stripped.startswith("#if"):
            if_depth += 1
            branch_seen.append(False)
        elif stripped.startswith(("#elseif", "#else")) and branch_seen:
            branch_seen[-1] = True
        elif stripped.startswith("#endif") and branch_seen:
            if_depth -= 1
            branch_seen.pop()
        elif depth == 0 and not any(branch_seen):
            match = TYPE.match(line)
            if match:
                hidden = bool(re.search(r"\b(private|fileprivate)\b", line.split(match.group(1))[0]))
                yield match.group(2), number, hidden
        code = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line.split("//")[0])
        depth += code.count("{") - code.count("}")


def repeated_declarations(path):
    """The same declaration line at the same indentation twice within one scope of one file (a merge that kept both
    sides). Scope = the stack of enclosing braces, so `func body` in two different views is fine."""
    seen = {}
    stack = [0]
    counter = 0
    problems = []
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    clean = [re.sub(r'"(?:[^"\\]|\\.)*"', '""', line.split("//")[0]) for line in lines]
    for number, code in enumerate(clean, 1):
        if DECL.match(code) and "#Preview" not in code:
            # The whole signature: overloads share their first line ("func userNotificationCenter(").
            signature = []
            for part in clean[number - 1 : number + 8]:
                signature.append(part.strip())
                if "{" in part or "=" in part:
                    break
            key = (tuple(stack), " ".join(signature))
            if key in seen:
                problems.append((number, seen[key], code.strip()))
            else:
                seen[key] = number
        for char in code:
            if char == "{":
                counter += 1
                stack.append(counter)
            elif char == "}" and len(stack) > 1:
                stack.pop()
    return problems


def main():
    failures = []
    for module, folders in MODULES.items():
        declared = defaultdict(list)
        for path in swift_files(folders):
            for name, number, hidden in top_level_types(path):
                declared[name].append((f"{path.relative_to(ROOT)}:{number}", hidden))
            for number, first, text in repeated_declarations(path):
                failures.append(f"{path.relative_to(ROOT)}:{number}: `{text}` repeats line {first} in the same scope")
        for name, places in sorted(declared.items()):
            # Two file-private types of one name live side by side; any other pair clashes (a private one shadows
            # and makes the other ambiguous in its file).
            if len(places) > 1 and not all(hidden for _, hidden in places):
                failures.append(f"{module}: `{name}` is declared more than once: " + ", ".join(p for p, _ in places))
    if failures:
        print("Swift declarations that would not compile together:\n" + "\n".join(failures), file=sys.stderr)
        return 1
    print("Swift duplicate-declaration check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
