#!/usr/bin/env python3
"""Decide which iOS UI test classes a run executes, and how the five UI shards split them.

Every run has a sixth shard, unit, for ReinsTests: it gates the iOS check. The UI shards never do (ios.yml).
ios.yml runs the UI shards on a push, the nightly and a hand run only, each the whole suite; a pull request runs
ReinsTests alone. For a pull request, select still gives the classes ios/ci-ui-map.txt names for its diff.
Longest matching prefix wins. Anything the map does not cover under ios/ (except the
unit tests), and any shared or core path, runs the whole suite. So does a pull request labelled ios-full.

  select                 print ui=ALL or ui=Class,Class from EVENT, BASE_SHA, LABELS
  select --files PATH    same, from those paths instead of git
  shard --name NAME --classes CLASSES
                         print only= and skip= for that shard
"""

import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MAP_PATH = ROOT / "ios" / "ci-ui-map.txt"
UI = "ReinsUITests"

# A matching path runs every UI class. Directory prefixes end in /.
ALL_PREFIXES = (
    "crates/reins-core/",
    "crates/reins-proto/",
    "crates/reins-policy/",
    "Cargo.lock",
    ".cargo/",
    "rust-toolchain.toml",
    "ios/Core/",
    "ios/Reins/Design/",
    "ios/Reins/App/",
    "ios/Reins/Shell/",
    "ios/Reins/Debug/",
    "ios/scripts/",
    "ios/Reins.xcodeproj/",
    "ios/ci-ui-map.txt",
    "scripts/ios-ui-plan.py",
    ".github/workflows/ios.yml",
)

# Whole classes owned by one shard. ActivityFlowTests is split by method below.
# Rest runs the classes in REMAINDER and the activity methods nobody else named. Unit runs ReinsTests.
SHARDS = (
    ("autopilot", ("AutopilotUITests",), ()),
    ("grants", ("GrantsSettingsUITests",), (
        "testAnEntryOpensAndAnEmailInItOpensInFull",
    )),
    ("onboarding", ("OnboardingUITests",), (
        "testTheQuickMenuSwitchesModesAndBypassAsksForALength",
        "testAScrollStartingOnAWaitingCardScrollsAndOpensNothing",
        "testAScrollStartingOnARowScrollsAndOpensNothing",
        "testTheDockOffersHowLongAndApprovingClosesTheScreen",
        "testDenyIsOneTap",
        "testSwipingARowLeftStillRevealsThisWasWrong",
        "testTheModePillOpensTheQuickMenuAndMoreOpensAutopilot",
        "testAPushFromAnotherDeviceLoadsItsDiffFromGitHub",
        "testTheQuickMenuOffersOvernight",
        "testTheSyncNoticeShowsUntilOK",
        "testARatedRequestShowsItsRiskAndThatOvernightHeldIt",
    )),
    ("field", (
        "PasskeyUITests",
        "ApprovalFieldUITests",
        "SoundsScreenUITests",
        "NotificationsUITests",
        "LaunchTests",
    ), (
        "testAScrollStartingOnARowAutopilotDecidedScrollsAndOpensNothing",
        "testTheTabBarStepsAsideForTheKeyboard",
        "testTheModeChipOpensOnATapAndOnAPressThatSlidesOntoARow",
        "testTappingARowStillOpensIt",
        "testALoadThatFailsSaysWhyAndOffersRetry",
        "testRowsScrollUnderPaperAtTheTop",
    )),
)
REMAINDER = ("IntegrationsUITests", "LiveOnboardingUITests", "LiveServerUITests")
FULL_EVENTS = frozenset(("push", "schedule", "workflow_dispatch"))


def load_map(path=MAP_PATH):
    rules = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        prefix, *classes = line.split()
        if not classes:
            raise SystemExit(f"map line has no class: {prefix}")
        rules.append((prefix, tuple(classes)))
    # Longest prefix first, so the first match wins.
    rules.sort(key=lambda item: len(item[0]), reverse=True)
    return rules


def matches(path, prefix):
    return path == prefix or path.startswith(prefix if prefix.endswith("/") else prefix)


def is_all_path(path):
    return any(matches(path, prefix) for prefix in ALL_PREFIXES)


def classes_for_path(path, rules):
    """Return 'ALL', a tuple of class names, or an empty tuple when the path adds nothing."""
    if is_all_path(path):
        return "ALL"
    for prefix, classes in rules:
        if matches(path, prefix):
            return classes
    if path.startswith("ios/") and not path.startswith("ios/ReinsTests/"):
        return "ALL"
    return ()


def choose(files, rules=None):
    """Comma-separated class names, 'ALL', or '' when the diff has no UI class."""
    rules = load_map() if rules is None else rules
    found = []
    for path in files:
        classes = classes_for_path(path, rules)
        if classes == "ALL":
            return "ALL"
        for name in classes:
            if name not in found:
                found.append(name)
    found.sort()
    return ",".join(found)


def full_run(event, labels):
    if event in FULL_EVENTS:
        return True
    return "ios-full" in [part.strip() for part in labels.split(",") if part.strip()]


def files_from_git(base):
    """None when the base is missing or not a commit. An empty list is a real empty diff."""
    if not base or set(base) == {"0"}:
        return None
    try:
        output = subprocess.check_output(
            ["git", "diff", "--name-only", "-z", base, "HEAD"], stderr=subprocess.DEVNULL
        )
    except subprocess.CalledProcessError:
        return None
    return [path for path in output.decode().split("\0") if path]


def selection(event, labels="", base="", files=None):
    if full_run(event, labels):
        return "ALL"
    if files is None:
        files = files_from_git(base)
        if files is None:
            return "ALL"
    return choose(files)


def ident(name):
    if name == "ReinsTests" or name.startswith(UI + "/"):
        return name
    return f"{UI}/{name}"


def activity_methods():
    methods = []
    for _, _, names in SHARDS:
        methods.extend(names)
    return methods


def shard_by_name(name):
    for shard, classes, methods in SHARDS:
        if shard == name:
            return classes, methods
    if name in ("rest", "unit"):
        return (), ()
    raise SystemExit(f"unknown shard {name}")


def parse_classes(text):
    text = text.strip()
    if text == "ALL":
        return "ALL"
    return {part.strip() for part in text.split(",") if part.strip()}


def shard_args(name, classes):
    """Return (only, skip) comma-separated. only is empty when the shard has nothing to run."""
    selected = parse_classes(classes)
    if name == "unit":
        return "ReinsTests", ""
    if name == "rest":
        return rest_args(selected)
    owned, methods = shard_by_name(name)
    parts = []
    if selected == "ALL":
        parts.extend(ident(item) for item in owned)
        parts.extend(ident(f"ActivityFlowTests/{item}") for item in methods)
        return ",".join(parts), ""
    for item in owned:
        if item in selected:
            parts.append(ident(item))
    if "ActivityFlowTests" in selected:
        parts.extend(ident(f"ActivityFlowTests/{item}") for item in methods)
    return ",".join(parts), ""


def rest_args(selected):
    dedicated = []
    for _, classes, _ in SHARDS:
        dedicated.extend(classes)
    method_skips = [ident(f"ActivityFlowTests/{item}") for item in activity_methods()]
    if selected == "ALL":
        skips = [ident(item) for item in dedicated] + method_skips
        return "ReinsUITests", ",".join(skips)
    parts = []
    if "ActivityFlowTests" in selected:
        parts.append(ident("ActivityFlowTests"))
    for item in REMAINDER:
        if item in selected:
            parts.append(ident(item))
    skips = method_skips if "ActivityFlowTests" in selected else []
    return ",".join(parts), ",".join(skips)


def main(argv):
    if len(argv) < 2 or argv[1] not in ("select", "shard"):
        raise SystemExit("usage: ios-ui-plan.py select [--files PATH ...] | shard --name NAME --classes CLASSES")
    if argv[1] == "select":
        files = []
        index = 2
        while index < len(argv):
            if argv[index] != "--files" or index + 1 >= len(argv):
                raise SystemExit(f"unknown argument {argv[index]}")
            files.append(argv[index + 1])
            index += 2
        if files:
            result = selection(os.environ.get("EVENT", "pull_request"), os.environ.get("LABELS", ""), files=files)
        else:
            result = selection(
                os.environ.get("EVENT", "pull_request"),
                os.environ.get("LABELS", ""),
                base=os.environ.get("BASE_SHA", ""),
            )
        print(f"ui={result}")
        return
    name = None
    classes = None
    index = 2
    while index < len(argv):
        if argv[index] == "--name" and index + 1 < len(argv):
            name = argv[index + 1]
            index += 2
        elif argv[index] == "--classes" and index + 1 < len(argv):
            classes = argv[index + 1]
            index += 2
        else:
            raise SystemExit(f"unknown argument {argv[index]}")
    if name is None or classes is None:
        raise SystemExit("shard needs --name and --classes")
    only, skip = shard_args(name, classes)
    print(f"only={only}")
    print(f"skip={skip}")


if __name__ == "__main__":
    main(sys.argv)
