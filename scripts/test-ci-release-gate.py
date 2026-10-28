#!/usr/bin/env python3
"""The release gate must wait, match the tested commit, and reject failed or unavailable CI."""

import os
import re
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHA = "a" * 40

with tempfile.TemporaryDirectory(prefix="reins-ci-gate-") as directory:
    root = Path(directory)
    # A disposable gh supplies states and records the real gate's query arguments.
    gh = root / "gh"
    gh.write_text(
        "#!/usr/bin/env python3\n"
        "import os, pathlib, sys\n"
        "root = pathlib.Path(os.environ['GATE_TEST_ROOT'])\n"
        "if sys.argv[1:3] == ['api', 'rate_limit']:\n"
        "    print((root / 'limit').read_text() if (root / 'limit').exists() else '4000 0'); sys.exit(0)\n"
        "(root / 'args').write_text('\\n'.join(sys.argv[1:]))\n"
        "states = (root / 'states').read_text().splitlines()\n"
        "state = states.pop(0) if states else 'missing'\n"
        "(root / 'states').write_text('\\n'.join(states))\n"
        "if state == 'api-error': sys.exit(1)\n"
        "if state == 'rate-limited':\n"
        "    (root / 'limit').write_text('0 0'); sys.exit(1)\n"
        "(root / 'limit').unlink(missing_ok=True)\n"
        "print(state)\n"
    )
    gh.chmod(0o755)
    cases = [
        (["success"], True),
        (["missing", "queued", "success"], True),
        (["in_progress", "success"], True),
        (["failure"], False),
        (["cancelled"], False),
        (["timed_out"], False),
        (["skipped"], False),
        (["neutral"], False),
        (["action_required"], False),
        (["unknown"], False),
        (["api-error"], False),
        (["api-error", "success"], True),
        (["api-error"] * 3, False),
        (["missing"] * 3, False),
        (["in_progress"] * 3, False),
        # A spent rate limit is waited out (until its reset) without using up the attempts, a few times at most.
        (["rate-limited"] * 3 + ["success"], True),
        (["rate-limited"] * 3 + ["in_progress", "in_progress", "success"], True),
        (["rate-limited"] * 4 + ["in_progress", "in_progress", "success"], False),
    ]
    environment = os.environ | {
        "PATH": f"{root}:{os.environ['PATH']}",
        "GATE_TEST_ROOT": str(root),
        "REINS_CI_POLL_ATTEMPTS": "3",
        "REINS_CI_POLL_SECONDS": "0",
    }
    for states, expected in cases:
        (root / "states").write_text("\n".join(states))
        (root / "limit").unlink(missing_ok=True)
        result = subprocess.run(
            ["bash", str(ROOT / "scripts/wait-for-ci.sh"), SHA],
            env=environment,
            check=False,
            capture_output=True,
            text=True,
        )
        assert (result.returncode == 0) == expected, (
            states,
            result.stdout,
            result.stderr,
        )
        args = (root / "args").read_text().splitlines()
        for flag, value in (
            ("--commit", SHA),
            ("--workflow", "ci.yml"),
            ("--event", "push"),
            ("--branch", "main"),
        ):
            assert args[args.index(flag) + 1] == value
    result = subprocess.run(
        ["bash", str(ROOT / "scripts/wait-for-ci.sh"), "invalid"],
        env=environment,
        check=False,
        capture_output=True,
    )
    assert result.returncode != 0

workflow = (ROOT / ".github/workflows/release.yml").read_text()
publisher = workflow.split("  publish:\n", 1)[1]
gate = publisher.index('scripts/wait-for-ci.sh "$SHA"')
assert gate < publisher.index("docker buildx imagetools create")
assert gate < publisher.index("gh release create")
assert publisher.index("--draft\n") < publisher.index(
    'gh release edit "$TAG" --draft=false --latest'
)
assert "needs.android.result == 'success'" in publisher
# Publication waits for every build it needs, each of which must have succeeded: the Linux packages are release
# assets (and the feed's repositories), installed from on both architectures.
needs = publisher.split("    needs:\n", 1)[1].split("    if:", 1)[0]
needed = [line.strip().removeprefix("- ") for line in needs.splitlines() if line.strip()]
assert {"desktop-linux", "desktop", "desktop-app-linux", "desktop-app", "macos", "server", "docker", "audit-tools",
        "executor-image", "android", "linux-packages", "linux-packages-amd64",
        "linux-packages-arm64"} <= set(needed), needed
# Every macOS build publication waits for is the one macos job; TestFlight comes after publication.
assert "ios-testflight" not in needed, needed
for job in needed[1:]:
    assert f"needs.{job}.result == 'success'" in publisher, job
# The Linux packages start once the Linux builds they package are done, not after every platform's.
packages = workflow.split("\n  linux-packages:\n", 1)[1].split("\n    steps:\n", 1)[0]
assert "needs: [plan, desktop-linux, desktop-app-linux]" in packages
# The AUR job runs after publication and can neither block nor fail the release.
aur = publisher.split("\n  aur:\n", 1)[1]
assert "needs: [plan, publish]" in aur and "continue-on-error: true" in aur
assert "needs.publish.result == 'success'" in aur
# Artifact retries must preserve the release gate: only the first attempt may fail softly.
action = (ROOT / ".github/actions/upload-artifact/action.yml").read_text()
first, retry = action.split("    - id: retry\n", 1)
assert "continue-on-error: true" in first
assert "continue-on-error:" not in retry
assert "steps.upload.outcome == 'failure'" in retry
assert "!cancelled()" in retry
assert "overwrite: true" in retry
assert action.count("if-no-files-found: error") == 2
for field in ("name", "path", "compression-level", "retention-days"):
    assert action.count(f"{field}: ${{{{ inputs.{field} }}}}") == 2
for job in ("desktop-linux", "desktop", "android", "audit-tools"):
    section = workflow.split(f"\n  {job}:\n", 1)[1].split("\n\n  ", 1)[0]
    assert "fail-fast: false" in section, job
assert "uses: actions/upload-artifact@" not in workflow
assert workflow.count("uses: ./.github/actions/upload-artifact") == 16
# One macOS job before publication, on one runner: the CLI, the app's binaries and the notarized app. Its artifacts
# keep the names publication downloads (dist-*), and nothing else in the run runs on macOS before publication.
macos = workflow.split("\n  macos:\n", 1)[1].split("\n\n  server:\n", 1)[0]
assert "    name: macOS builds\n" in macos and "    needs: plan\n" in macos and "runs-on: macos-15" in macos
assert "name: dist-desktop-apple-darwin" in macos and "name: dist-app-macOS" in macos
assert "download-artifact" not in macos and "\n    environment:" not in macos
assert "timeout-minutes: 75 #" in macos and "timeout-minutes: 160" in macos
# Every build ends before the first secret: nothing runs in the background, a step checks that no build process is
# left, and the keychain and key files are removed whatever happened.
macos_steps = macos.split("\n    steps:\n", 1)[1]
assert " &\n" not in macos_steps and "nohup" not in macos_steps
first_secret = min(macos_steps.index(marker) for marker in ("secrets.", "release-secrets"))
assert macos_steps.index("name: No build is running") < first_secret
assert macos_steps.rindex("cargo build") < macos_steps.index("name: No build is running")
assert "pgrep -fl" in macos_steps
removal = macos_steps.split("      - name: Remove signing material\n", 1)[1].split("\n      - ", 1)[0]
assert removal.startswith("        if: always()") and "delete-keychain" in removal and "notary.p8" in removal
assert ".apple-signing.env" in removal and "cert.p12" in removal
jobs = re.split(r"\n  ([a-z0-9-]+):\n", workflow.split("\njobs:\n", 1)[1])
macos_jobs = [name for name, body in zip(jobs[1::2], jobs[2::2])
              if re.search(r"^    runs-on: .*(macos|xcode)", body.split("\n    steps:", 1)[0], re.M)]
assert macos_jobs == ["macos", "ios-testflight"], macos_jobs
# The release never waits for CI's macOS gate.
assert "uses: ./.github/workflows/macos-gate.yml" not in workflow
# The APK and the Play bundle are built on two runners and must carry the same versionCode: plan's.
android = workflow.split("\n  android:\n", 1)[1].split("\n\n  ", 1)[0]
assert "VERSION_CODE: ${{ needs.plan.outputs.version-code }}" in android
assert "date -u" not in android
assert "needs.android.outputs.version-code" not in workflow
# The organization tools and the executor image: builds like the others, before publication, with no secret, no OIDC
# token and only read access (the image also writes packages, by digest only). A verify_only run pushes nothing, and
# only publish tags the image, after the CI gate.
tools = workflow.split("\n  audit-tools:\n", 1)[1].split("\n\n  ", 1)[0]
image = workflow.split("\n  executor-image:\n", 1)[1].split("\n\n  ", 1)[0]
for job, section, permissions in (("audit-tools", tools, ["contents: read"]),
                                  ("executor-image", image, ["contents: read", "packages: write"])):
    block = section.split("\n    permissions:\n", 1)[1].split("\n    strategy:", 1)[0].split("\n    env:", 1)[0]
    assert [line.strip() for line in block.splitlines()] == permissions, (job, block)
    for forbidden in ("secrets.", "release-secrets", "id-token", "environment:", "INFISICAL"):
        assert forbidden not in section, (job, forbidden)
    assert "if: needs.plan.outputs.release == 'true'" in section, job
    assert "ref: ${{ needs.plan.outputs.sha }}" in section and "persist-credentials: false" in section, job
assert "    needs: plan\n" in tools and "    needs: [plan, audit-tools]\n" in image
assert "cargo zigbuild --release --locked -p reins-audit -p reins-executor" in tools
assert "cargo-zigbuild==0.23.4 ziglang==0.16.0" in tools
assert "name: dist-tools-${{ matrix.target }}" in tools and "name: executor-bin-${{ matrix.target }}" in tools
assert 'scripts/package-tool.sh "$tool" "$TARGET" "$VERSION"' in tools
assert "name: executor-image-digest" in image and "file: docker/Dockerfile.executor" in image
assert image.count("platforms: linux/amd64,linux/arm64") == 2
# Every output of the image job pushes only when the run is not verify_only; the check build only loads.
outputs = re.findall(r"^          outputs: (.*)$", image, re.M)
assert outputs == ["type=docker"] + ["${{ inputs.verify_only && 'type=cacheonly' || format('type=image,name={0},"
                                      "push-by-digest=true,name-canonical=true,push=true', "
                                      "steps.image.outputs.name) }}"] * 2, outputs
upload = ("      - uses: ./.github/actions/upload-artifact"
          " # zizmor: ignore[self-repository] use the exact release checkout\n")
for step in ("      - uses: docker/login-action@v4\n", "      - name: Export image digest\n", upload):
    assert image.split(step, 1)[1].startswith("        if: inputs.verify_only != true\n"), step
push = "format('type=image,name={0},push-by-digest=true,name-canonical=true,push=true'"
assert "push=true" not in image.replace(push, "") and image.count(push) == 2
assert image.count("org.opencontainers.image.licenses=Apache-2.0") == 2
tagging = publisher.split("      - name: Tag the executor image\n", 1)[1].split("\n      - ", 1)[0]
assert tagging.startswith("        if: inputs.verify_only != true") and "/reins-executor" in tagging
assert gate < publisher.index("      - name: Tag the executor image\n") < publisher.index("gh release create")
assert "--executor-image" in publisher.split("      - name: Release notes\n", 1)[1].split("\n      - ", 1)[0]
# The Linux packages take the desktop archives only; publication takes the tools' archives as release assets.
packaging = workflow.split("\n  linux-packages:\n", 1)[1].split("\n\n  ", 1)[0]
assert "pattern: dist-desktop-*-unknown-linux-musl" in packaging and "dist-tools" not in packaging
assert "          pattern: dist-*\n" in publisher
dockerfile = (ROOT / "docker/Dockerfile.executor").read_text()
assert "FROM scratch" in dockerfile and "\nRUN " not in dockerfile and "ARG TARGETARCH" in dockerfile
assert "USER 65532:65532" in dockerfile and "VOLUME /var/lib/reins-executor" in dockerfile
assert 'ENTRYPOINT ["/reins-executor"]' in dockerfile and "Apache-2.0" in dockerfile
# The release notes name the tools' archives and the executor image.
with tempfile.TemporaryDirectory(prefix="reins-notes-") as directory:
    # A throwaway repository, configured as scripts/test-next-version.sh configures its own.
    subprocess.run(["git", "init", "-q", directory], check=True)
    for key, value in (("user.name", "test"), ("user.email", "test@example.com"), ("commit.gpgsign", "false")):
        subprocess.run(["git", "-C", directory, "config", key, value], check=True)
    subprocess.run(["git", "-C", directory, "commit", "-q", "--allow-empty", "-m", "feat: one"], check=True)
    notes = subprocess.run(
        ["bash", str(ROOT / "scripts/release-notes.sh"), "--version", "1.2.3", "--previous", "", "--repo",
         "https://github.com/Owner/reins", "--executor-image", "ghcr.io/owner/reins-executor"],
        cwd=directory, check=True, capture_output=True, text=True).stdout
    assert "docker pull ghcr.io/owner/reins-executor:1.2.3" in notes, notes
    assert "reins-audit-1.2.3-x86_64-unknown-linux-musl.tar.gz" in notes, notes
    assert "reins-executor-1.2.3-<target>.tar.gz" in notes, notes
    assert "docker pull ghcr.io/owner/reins-server:1.2.3" in notes, notes
print("92 release CI gate and artifact retry checks passed")
