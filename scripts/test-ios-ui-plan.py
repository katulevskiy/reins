#!/usr/bin/env python3
"""The iOS UI plan: mapped classes, fail-closed paths, and the five shard lists."""

import importlib.util
import os
from pathlib import Path
import tempfile


scratch = Path.home() / "Documents/dev/tmp/ios-ci"
scratch.mkdir(parents=True, exist_ok=True)
os.environ["TMPDIR"] = str(scratch)
tempfile.tempdir = str(scratch)

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("ios_ui_plan", ROOT / "scripts" / "ios-ui-plan.py")
plan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plan)

QUARANTINE = "ReinsUITests/ActivityFlowTests/testAScrollStartingOnARowAutopilotDecidedScrollsAndOpensNothing"


def classes_in_sources():
    found = []
    for path in sorted((ROOT / "ios/ReinsUITests").glob("*.swift")):
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith("final class ") and "XCTestCase" in line:
                found.append(line.split()[2].split("(")[0].rstrip(":"))
    return found


def runs(shard, classes):
    only, skip = plan.shard_args(shard, classes)
    return [part for part in only.split(",") if part], [part for part in skip.split(",") if part]


def test_mapped_files():
    assert plan.choose(["ios/Reins/Screens/Activity/ActivityScreen.swift"]) == "ActivityFlowTests"
    assert plan.choose(["ios/Reins/Screens/Settings/SoundsScreen.swift"]) == "SoundsScreenUITests"
    assert plan.choose(["ios/Reins/Screens/Settings/SettingsScreen.swift"]) == "GrantsSettingsUITests"
    assert plan.choose(["ios/Reins/Platform/NotificationAccess.swift"]) == "NotificationsUITests"
    assert plan.choose(["ios/Reins/Screens/Settings/VaultPasskeysScreen.swift"]) == "PasskeyUITests"
    assert plan.choose([
        "ios/Reins/Screens/Activity/Row.swift",
        "ios/Reins/Feedback/Sounds.swift",
    ]) == "ActivityFlowTests,SoundsScreenUITests"


def test_fail_closed():
    all_paths = [
        "ios/Reins/Design/ScrollSafeGestures.swift",
        "ios/Reins/App/Navigation.swift",
        "ios/Reins/Shell/MainShell.swift",
        "ios/Reins/Debug/DemoCore.swift",
        "ios/Core/Generated/reins_core.swift",
        "ios/scripts/build-core.sh",
        "ios/Reins.xcodeproj/project.pbxproj",
        "ios/ci-ui-map.txt",
        "ios/README.md",
        "ios/Reins/Platform/Authenticator.swift",
        "crates/reins-core/src/lib.rs",
        "crates/reins-proto/src/lib.rs",
        "crates/reins-policy/src/lib.rs",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".cargo/config.toml",
        "scripts/ios-ui-plan.py",
        ".github/workflows/ios.yml",
    ]
    for path in all_paths:
        assert plan.choose([path]) == "ALL", path
    assert plan.choose(["ios/Reins/Screens/Activity/Row.swift", "ios/Reins/Design/Type.swift"]) == "ALL"


def test_unit_tests_add_no_ui_class():
    assert plan.choose(["ios/ReinsTests/PasswordlessTests.swift"]) == ""
    assert plan.choose(["docs/example.md"]) == ""


def test_full_events_and_label():
    activity = ["ios/Reins/Screens/Activity/Row.swift"]
    for event in ("push", "schedule", "workflow_dispatch"):
        assert plan.selection(event, files=activity) == "ALL"
    assert plan.selection("pull_request", labels="bug,ios-full", files=activity) == "ALL"
    assert plan.selection("pull_request", labels="bug", files=activity) == "ActivityFlowTests"
    assert plan.selection("pull_request", base="", files=None) == "ALL"
    assert plan.selection("pull_request", base="0" * 40, files=None) == "ALL"


def test_all_shards_match_the_full_suite():
    only, skip = runs("autopilot", "ALL")
    assert only == ["ReinsUITests/AutopilotUITests"]
    assert skip == []
    only, skip = runs("rest", "ALL")
    assert only == ["ReinsTests", "ReinsUITests"]
    assert "ReinsUITests/ActivityFlowTests" not in skip
    assert "ReinsUITests/AutopilotUITests" in skip
    assert QUARANTINE in skip


def test_mapped_shards_leave_the_others_empty():
    only, _ = runs("field", "PasskeyUITests")
    assert only == ["ReinsUITests/PasskeyUITests"]
    only, _ = runs("autopilot", "PasskeyUITests")
    assert only == []
    only, _ = runs("rest", "PasskeyUITests")
    assert only == ["ReinsTests"]


def test_activity_stays_split_and_the_quarantine_still_runs():
    only, skip = runs("grants", "ActivityFlowTests")
    assert only == ["ReinsUITests/ActivityFlowTests/testAnEntryOpensAndAnEmailInItOpensInFull"]
    assert skip == []
    only, _ = runs("field", "ActivityFlowTests")
    assert QUARANTINE in only
    only, skip = runs("rest", "ActivityFlowTests")
    assert "ReinsTests" in only
    assert "ReinsUITests/ActivityFlowTests" in only
    assert "ReinsUITests/IntegrationsUITests" not in only
    assert "ReinsUITests/ActivityFlowTests/testAnEntryOpensAndAnEmailInItOpensInFull" in skip
    assert QUARANTINE in skip
    assert "ReinsUITests/ActivityFlowTests" not in skip


def test_every_ui_class_runs_on_the_full_suite():
    for name in classes_in_sources():
        covered = False
        for shard in ("autopilot", "grants", "onboarding", "field", "rest"):
            only, skip = runs(shard, "ALL")
            whole = f"ReinsUITests/{name}"
            if whole in only:
                covered = True
            if "ReinsUITests" in only and whole not in skip:
                covered = True
            if any(item.startswith(whole + "/") for item in only):
                covered = True
        assert covered, name
    mapped = set()
    for _, classes in plan.load_map():
        mapped.update(classes)
    assert set(classes_in_sources()) <= mapped


def test_cli_select_and_shard():
    env = os.environ | {"EVENT": "pull_request", "LABELS": "", "BASE_SHA": ""}
    import subprocess
    output = subprocess.check_output(
        ["python3", str(ROOT / "scripts/ios-ui-plan.py"), "select", "--files", "ios/Reins/Screens/Vault/VaultScreen.swift"],
        env=env, text=True,
    )
    assert output == "ui=PasskeyUITests\n"
    output = subprocess.check_output(
        ["python3", str(ROOT / "scripts/ios-ui-plan.py"), "shard", "--name", "rest", "--classes", ""],
        text=True,
    )
    assert output == "only=ReinsTests\nskip=\n"


if __name__ == "__main__":
    test_mapped_files()
    test_fail_closed()
    test_unit_tests_add_no_ui_class()
    test_full_events_and_label()
    test_all_shards_match_the_full_suite()
    test_mapped_shards_leave_the_others_empty()
    test_activity_stays_split_and_the_quarantine_still_runs()
    test_every_ui_class_runs_on_the_full_suite()
    test_cli_select_and_shard()
    print("iOS UI plan checks passed")
