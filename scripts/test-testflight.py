#!/usr/bin/env python3
"""Exercise scripts/testflight.py against a fake App Store Connect (no network): the ES256 JWT, the request bodies,
the build number, What to Test, group selection and a refused review submission."""

import base64
import contextlib
import datetime
import importlib.util
import io
import json
import plistlib
import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("testflight", ROOT / "scripts/testflight.py")
tf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tf)

checks = 0


def check(condition, message):
    global checks
    checks += 1
    assert condition, message


def b64decode(part):
    return base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))


def raw_to_der(raw):
    def integer(value):
        value = value.lstrip(b"\0") or b"\0"
        if value[0] & 0x80:
            value = b"\0" + value
        return bytes([0x02, len(value)]) + value
    body = integer(raw[:32]) + integer(raw[32:])
    return bytes([0x30, len(body)]) + body


class FakeAsc:
    """The endpoints the script uses. `builds` is the app's builds; a build becomes VALID after `processing` polls."""

    def __init__(self, builds=(), groups=(), review_error=None, processing=1, encryption=None, reviews=()):
        self.calls = []
        self.builds = [dict(b) for b in builds]
        self.groups = list(groups)
        self.review_error = review_error
        # The app's beta review submissions, as the API lists them.
        self.reviews = list(reviews)
        self.processing = processing
        self.encryption = encryption
        self.localizations = []

    def request(self, method, url, headers=None, body=None, timeout=60):
        self.calls.append((method, url, headers or {}, body))
        path = url.removeprefix(tf.API + "/")
        if path.startswith("apps?"):
            return {"data": [{"id": "6820830980", "attributes": {"bundleId": tf.BUNDLE_ID}}]}
        if path.startswith("builds?"):
            query = dict(p.split("=", 1) for p in path.split("?", 1)[1].split("&"))
            found = [b for b in self.builds
                     if query.get("filter%5BpreReleaseVersion.version%5D", b["pre"]) == b["pre"]
                     and query.get("filter%5Bversion%5D", b["version"]) == b["version"]]
            if "filter%5Bversion%5D" in query:
                self.processing -= 1
            # Two pages, to follow links.next.
            if len(found) > 1 and "page=2" not in path:
                return {"data": [self.build(found[0])], "links": {"next": tf.API + "/" + path + "&page=2"}}
            return {"data": [self.build(b) for b in (found[1:] if "page=2" in path else found)]}
        if path.startswith("betaGroups?"):
            return {"data": self.groups}
        if path.endswith("/betaBuildLocalizations?limit=50"):
            return {"data": self.localizations}
        if path.endswith("/buildBetaDetail"):
            return {"data": {"attributes": {"externalBuildState": "READY_FOR_BETA_SUBMISSION"}}}
        if path.startswith("betaAppReviewSubmissions?"):
            return {"data": self.reviews}
        if path == "betaAppReviewSubmissions" and self.review_error:
            raise tf.AscError(*self.review_error)
        return {}

    def build(self, b):
        state = "VALID" if self.processing <= 0 else "PROCESSING"
        return {"id": f"build-{b['version']}", "attributes": {"version": b["version"], "processingState": state,
                                                              "usesNonExemptEncryption": self.encryption}}

    def routes(self):
        return [(m, u.removeprefix(tf.API + "/").split("?")[0]) for m, u, _, _ in self.calls]

    def bodies(self, method, path):
        return [b for m, u, _, b in self.calls if m == method and u.removeprefix(tf.API + "/").split("?")[0] == path]


def group(gid, name, internal, all_builds=False):
    return {"id": gid, "attributes": {"name": name, "isInternalGroup": internal, "hasAccessToAllBuilds": all_builds}}


GROUPS = [group("int-1", "Internal", True, True), group("228ac7ac-1", "Testers", False),
          group("04f25a5f-1", "Testers", False)]
NOTES = """## Features

- **ios:** TestFlight on release ([`abc1234`](https://github.com/o/r/commit/abc1234def))
- docs: \\<computer\\> names `machine-x` ([`def5678`](https://github.com/o/r/commit/def5678))

**Full changelog:** https://github.com/o/r/compare/v0.5.12...v0.6.0

## Install

curl ...
"""


def run(argv, http, environ=None):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            code = tf.main(argv, http=http, environ=environ or {})
        except SystemExit as exit_:
            code = exit_.code
    return code, out.getvalue(), err.getvalue()


with tempfile.TemporaryDirectory(prefix="reins-testflight-") as directory:
    root = Path(directory)
    key = root / "AuthKey_TEST.p8"
    subprocess.run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", str(key)],
                   check=True, capture_output=True)
    public = root / "public.pem"
    subprocess.run(["openssl", "pkey", "-in", str(key), "-pubout", "-out", str(public)], check=True,
                   capture_output=True)
    pem = key.read_text()

    # The JWT: ES256 header with the key id, Apple's audience, at most 20 minutes, and a signature openssl verifies.
    token = tf.make_jwt(pem, "KEY123", "issuer-uuid", now=1_800_000_000)
    header, claims, signature = token.split(".")
    check(json.loads(b64decode(header)) == {"alg": "ES256", "kid": "KEY123", "typ": "JWT"}, header)
    claims = json.loads(b64decode(claims))
    check(claims == {"iss": "issuer-uuid", "iat": 1_800_000_000, "exp": 1_800_000_900, "aud": "appstoreconnect-v1"},
          claims)
    check(claims["exp"] - claims["iat"] <= 20 * 60, "Apple refuses tokens living over 20 minutes")
    raw = b64decode(signature)
    check(len(raw) == 64, "JWS ES256 signatures are r || s, 64 bytes")
    (root / "sig.der").write_bytes(raw_to_der(raw))
    (root / "input").write_text(f"{token.rsplit('.', 1)[0]}")
    verified = subprocess.run(["openssl", "dgst", "-sha256", "-verify", str(public),
                               "-signature", str(root / "sig.der"), str(root / "input")],
                              capture_output=True, text=True, check=False)
    check(verified.returncode == 0, verified.stdout + verified.stderr)
    check(tf.der_to_raw(raw_to_der(b"\1" * 64)) == b"\1" * 64, "DER round trip")
    check(tf.der_to_raw(raw_to_der(b"\0" * 31 + b"\x80" + b"\x7f" * 32)) == b"\0" * 31 + b"\x80" + b"\x7f" * 32,
          "short and padded integers")
    for bad in (b"", b"\x31\x06\x02\x01\x01\x02\x01\x01", b"\x30\x06\x02\x01\x01\x04\x01\x01"):
        try:
            tf.der_to_raw(bad)
        except ValueError:
            check(True, "")
        else:
            check(False, f"{bad!r} is not a signature")

    # The bodies.
    check(tf.encryption_body("b1", False) == {"data": {"type": "builds", "id": "b1",
                                                      "attributes": {"usesNonExemptEncryption": False}}}, "encryption")
    created = tf.localization_create_body("b1", "text")
    check(created["data"]["attributes"] == {"locale": "en-US", "whatsNew": "text"}
          and created["data"]["relationships"]["build"]["data"] == {"type": "builds", "id": "b1"}, created)
    check(tf.localization_update_body("l1", "t")["data"] == {"type": "betaBuildLocalizations", "id": "l1",
                                                            "attributes": {"whatsNew": "t"}}, "update")
    check(tf.group_builds_body("b1") == {"data": [{"type": "builds", "id": "b1"}]}, "group")
    check(tf.review_submission_body("b1")["data"]["relationships"]["build"]["data"]["id"] == "b1", "review")

    # What to Test: the changes as plain text, no links, no install section, within 4000 characters.
    text = tf.what_to_test(NOTES, "0.6.0", "https://github.com/o/r/releases/tag/v0.6.0")
    check(text.startswith("Reins 0.6.0\n\nFeatures:\n\n- ios: TestFlight on release\n"), text)
    check("- docs: <computer> names machine-x" in text, text)
    check("commit" not in text and "curl" not in text and "Full changelog" not in text, text)
    check(text.endswith("All changes: https://github.com/o/r/releases/tag/v0.6.0"), text)
    long = tf.what_to_test("- change\n" * 2000, "0.6.0", "https://example.com/x")
    check(len(long) <= tf.WHAT_TO_TEST_LIMIT and "…" in long and long.endswith("https://example.com/x"), len(long))
    check(tf.what_to_test("", "0.6.0") == "Reins 0.6.0", "no notes")

    # The build number: never below the scheme, always above anything uploaded (by hand too).
    check(tf.next_build_number(["1", "2"], 29_860_000) == 29_860_000, "the scheme")
    check(tf.next_build_number(["29860005", "3.1"], 29_860_000) == 29_860_006, "after the highest")
    check(tf.next_build_number([], 5) == 5 and tf.next_build_number(["x"], 5) == 5, "empty or odd")

    # Export compliance comes from the Info.plist, which declares standard (exempt) encryption.
    check(tf.uses_non_exempt_encryption(ROOT / "ios/Reins/Info.plist") is False, "Info.plist")

    # prepare: hand uploads 0.2.5 (1) and 0.2.6 (2) leave the scheme alone; a rerun reuses its upload.
    env = {"AC_API_KEY_ID": "KEY123", "AC_API_ISSUER_ID": "issuer-uuid"}
    hand = [{"pre": "0.2.5", "version": "1"}, {"pre": "0.2.6", "version": "2"}]
    fake = FakeAsc(hand)
    code, out, err = run(["prepare", "--version", "0.6.0", "--minimum", "29860000", "--key-file", str(key)], fake, env)
    check(code == 0 and out == "build-number=29860000\nuploaded=false\n", (code, out, err))
    check(all(h["Authorization"].startswith("Bearer ey") for _, _, h, _ in fake.calls), "authorized")
    check(pem not in out + err and "Bearer" not in out + err, "no secrets printed")
    fake = FakeAsc(hand + [{"pre": "0.2.6", "version": "29860100"}])
    code, out, err = run(["prepare", "--version", "0.6.0", "--minimum", "29860000", "--key-file", str(key)], fake, env)
    check(out == "build-number=29860101\nuploaded=false\n", out)
    fake = FakeAsc(hand + [{"pre": "0.6.0", "version": "29860000"}])
    code, out, err = run(["prepare", "--version", "0.6.0", "--minimum", "29860000", "--key-file", str(key)], fake, env)
    check(out == "build-number=29860000\nuploaded=true\n", out)
    code, out, err = run(["prepare", "--version", "0.6.0", "--minimum", "1"], FakeAsc(), env)
    check(code == 2 and "--key-file" in err, err)

    # finish: processing is waited for, compliance set, What to Test written, the build goes to the external group
    # (by id: two groups share its name), the internal one gets every build by itself, and a refused review
    # submission is only a warning.
    notes = root / "notes.md"
    notes.write_text(NOTES)
    build = {"pre": "0.6.0", "version": "29860000"}
    finish = ["finish", "--version", "0.6.0", "--build-number", "29860000", "--notes-file", str(notes),
              "--group", "Internal,228ac7ac-1", "--interval", "0", "--key-file", str(key)]
    fake = FakeAsc([build], GROUPS, review_error=(409, "Another build is in review", "STATE_ERROR"), processing=2)
    code, out, err = run(finish, fake, env | {"GITHUB_ACTIONS": "true"})
    check(code == 0, (code, err))
    check(fake.bodies("PATCH", "builds/build-29860000") == [tf.encryption_body("build-29860000", False)], fake.calls)
    posted = fake.bodies("POST", "betaBuildLocalizations")
    check(len(posted) == 1 and posted[0]["data"]["attributes"]["whatsNew"].startswith("Reins 0.6.0"), posted)
    check(fake.bodies("POST", "betaGroups/228ac7ac-1/relationships/builds") == [tf.group_builds_body("build-29860000")],
          fake.routes())
    check(not any("int-1" in u or "04f25a5f" in u for _, u in fake.routes()), fake.routes())
    check(("POST", "betaAppReviewSubmissions") in fake.routes(), fake.routes())
    check("::warning::Beta app review did not take the build" in err, err)
    routes = fake.routes()
    check(routes.index(("POST", "betaGroups/228ac7ac-1/relationships/builds"))
          < routes.index(("POST", "betaAppReviewSubmissions")), "the group before the review")

    # Compliance already answered, an existing localization: nothing re-set, the text updated in place.
    fake = FakeAsc([build], GROUPS, encryption=False, processing=0)
    fake.localizations = [{"id": "loc-1", "attributes": {"locale": "en-US"}}]
    code, out, err = run(finish, fake, env)
    check(code == 0 and not fake.bodies("PATCH", "builds/build-29860000"), fake.routes())
    check(len(fake.bodies("PATCH", "betaBuildLocalizations/loc-1")) == 1, fake.routes())

    # A group named by its ambiguous name, or missing, fails clearly (after the rest is done).
    fake = FakeAsc([build], GROUPS, processing=0)
    code, out, err = run(finish[:7] + ["--group", "Internal,Testers,Nobody"] + finish[9:], fake, env)
    check(code == 1 and "2 TestFlight groups are named 'Testers'" in err and "TESTFLIGHT_EXTERNAL_GROUP_ID" in err, err)
    check("No TestFlight group 'Nobody'" in err, err)
    check(("POST", "betaBuildLocalizations") in fake.routes(), "What to Test is set anyway")
    check(("POST", "betaAppReviewSubmissions") not in fake.routes(), "no external group, no review")

    # Any other refusal of the review (not 409/422) fails the job.
    fake = FakeAsc([build], GROUPS, review_error=(403, "forbidden"), processing=0)
    code, out, err = run(finish, fake, env)
    check(code == 1 and "HTTP 403" in err, err)

    # Apple takes few review submissions a day: none while a build went to review in the last 24 hours, a notice
    # instead; the build still goes to its groups.
    def submitted(hours_ago):
        at = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(hours=hours_ago)
        return {"id": "sub-1", "attributes": {"submittedDate": at.isoformat(timespec="seconds")}}

    fake = FakeAsc([build], GROUPS, processing=0, reviews=[submitted(2)])
    code, out, err = run(finish, fake, env | {"GITHUB_ACTIONS": "true"})
    check(code == 0 and ("POST", "betaAppReviewSubmissions") not in fake.routes(), fake.routes())
    check("::notice::Not submitted for beta app review: a build went to review 2 h ago" in err, err)
    check("::warning::" not in err, err)
    check(("POST", "betaGroups/228ac7ac-1/relationships/builds") in fake.routes(), "still in the external group")
    fake = FakeAsc([build], GROUPS, processing=0, reviews=[submitted(30)])
    code, out, err = run(finish, fake, env)
    check(code == 0 and ("POST", "betaAppReviewSubmissions") in fake.routes(), "a day later it goes to review")

    # Apple's own daily limit is a notice, not a warning.
    limit = (422, "Submission limit has been reached.", "ENTITY_UNPROCESSABLE.SUBMISSION_LIMIT_REACHED")
    fake = FakeAsc([build], GROUPS, review_error=limit, processing=0)
    code, out, err = run(finish, fake, env | {"GITHUB_ACTIONS": "true"})
    check(code == 0 and "::notice::Beta app review takes no more submissions today" in err, err)
    check("::warning::" not in err, err)
    check(tf.parse_time("2026-10-15T00:15:57-07:00") == 1_792_048_557 and tf.parse_time(None) is None
          and tf.parse_time("soon") is None, "App Store Connect times")

    # A build that failed processing fails; one never processed times out.
    class Invalid(FakeAsc):
        def build(self, b):
            return {"id": "b", "attributes": {"version": b["version"], "processingState": "INVALID"}}
    code, out, err = run(finish, Invalid([build], GROUPS), env)
    check(code == 1 and "INVALID" in err, err)
    code, out, err = run(finish[:-2] + ["--timeout", "0", "--key-file", str(key)], FakeAsc([], GROUPS), env)
    check(code == 1 and "was not processed" in err, err)

    # Transient answers are retried; others are not.
    class Flaky(tf.Http):
        def __init__(self, answers):
            super().__init__(delays=(0, 0))
            self.answers = list(answers)

        def send(self, method, url, headers, body, timeout):
            answer = self.answers.pop(0)
            if isinstance(answer, tf.AscError):
                raise answer
            return answer
    check(Flaky([tf.AscError(503, "x"), tf.AscError(0, "y"), {"ok": 1}]).request("GET", "u") == {"ok": 1}, "retried")
    for answers in ([tf.AscError(409, "no")], [tf.AscError(500, "x")] * 3):
        try:
            Flaky(answers).request("GET", "u")
        except tf.AscError:
            check(True, "")
        else:
            check(False, answers)
    detail = tf.error_detail('{"errors": [{"code": "STATE_ERROR", "detail": "in review"}]}')
    check(detail == ("in review", "STATE_ERROR"), detail)

    # Profiles: an App Store profile of the team for the bundle id passes; others are named.
    later = datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(days=300)
    good = {"UUID": "u-1", "Name": "Reins App Store", "TeamIdentifier": ["5DRCQV8HY2"], "ExpirationDate": later,
            "Entitlements": {"application-identifier": "5DRCQV8HY2.com.reins2fa.app", "get-task-allow": False}}
    check(tf.check_profile(good, "com.reins2fa.app", "5DRCQV8HY2") == ("u-1", "Reins App Store"), "good profile")
    for change, reason in (({"ProvisionedDevices": ["x"]}, "not App Store"),
                           ({"Entitlements": {"application-identifier": "5DRCQV8HY2.com.reins2fa.app.widgets"}},
                            "not 5DRCQV8HY2.com.reins2fa.app"),
                           ({"ExpirationDate": datetime.datetime(2020, 1, 1)}, "expired")):
        try:
            tf.check_profile(good | change, "com.reins2fa.app", "5DRCQV8HY2")
        except ValueError as error:
            check(reason in str(error), str(error))
        else:
            check(False, change)
    (root / "profile.plist").write_bytes(plistlib.dumps(good))
    code, out, err = run(["profile", "--plist", str(root / "profile.plist"), "--bundle-id", "com.reins2fa.app",
                          "--team", "5DRCQV8HY2"], None)
    check(code == 0 and out == "u-1\n", (out, err))

    # secrets: from the step's environment, every value masked first, private files, only the group asked for, and
    # all missing names named (nothing written then).
    p12 = base64.b64encode(b"p12-bytes").decode()
    environment = {"AC_API_KEY_P8": pem, "AC_API_KEY_ID": "KEY123", "AC_API_ISSUER_ID": "0a1b-2c3d",
                   "IOS_DIST_CERT_P12": p12, "IOS_DIST_CERT_PASSWORD": "hunter2",
                   **{f"IOS_PROFILE_{n}": base64.b64encode(n.encode()).decode()
                      for n in ("APP", "WIDGETS", "NOTIFICATIONS")}}
    out_dir = root / "signing"
    code, out, err = run(["secrets", "--need", "asc", "--out", str(out_dir)], None, environment)
    check(code == 0 and sorted(p.name for p in out_dir.iterdir()) == ["AuthKey.p8", "asc.env", "team"], (out, err))
    check("AC_API_KEY_ID=KEY123\n" in (out_dir / "asc.env").read_text() and "TEAM=5DRCQV8HY2" in
          (out_dir / "asc.env").read_text(), "ids and the default team")
    check(all(p.stat().st_mode & 0o777 == 0o600 for p in out_dir.iterdir()), "private files")
    check(out_dir.stat().st_mode & 0o777 == 0o700, "a private folder")
    check("::add-mask::-----BEGIN PRIVATE KEY-----" in out and "::add-mask::KEY123" in out, out)
    check("hunter2" not in out + err, "only the group asked for is read")
    code, out, err = run(["secrets", "--need", "cert", "--out", str(out_dir)], None, environment)
    check((out_dir / "dist.p12").read_bytes() == b"p12-bytes" and (out_dir / "dist.pass").read_text() == "hunter2",
          "the certificate")
    check("::add-mask::hunter2" in out and "hunter2" not in err and not list(out_dir.glob("*.mobileprovision")),
          (out, err))
    code, out, err = run(["secrets", "--need", "profiles", "--out", str(out_dir)], None, environment)
    check((out_dir / "ReinsWidgets.mobileprovision").read_bytes() == b"WIDGETS" and "hunter2" not in out, "profiles")
    code, out, err = run(["secrets", "--need", "cert,profiles", "--out", str(root / "none")], None,
                         {"GITHUB_ACTIONS": "true", "IOS_DIST_CERT_P12": p12})
    check(code == 2 and "::error::TestFlight signing is incomplete, missing: IOS_DIST_CERT_PASSWORD IOS_PROFILE_APP "
          "IOS_PROFILE_WIDGETS IOS_PROFILE_NOTIFICATIONS" in err and "testflight" in err, err)
    check(not (root / "none").exists(), "nothing written when something is missing")
    code, out, err = run(["secrets", "--need", "dotenv", "--out", str(root / "none")], None, environment)
    check(code == 2, "unknown groups refused")

    # packages: pinned to a revision and checked out at it; plugins and binary targets are listed.
    checkouts = root / "spm" / "checkouts"
    package = checkouts / "onnxruntime-swift-package-manager"
    package.mkdir(parents=True)
    (package / "Package.swift").write_text('targets: [.binaryTarget(name: "onnxruntime", url: "u", checksum: "c"),\n'
                                          '  .plugin(name: "Gen", capability: .buildTool())]')
    git = ["git", "-C", str(package), "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]
    subprocess.run(git[:3] + ["init", "-q"], check=True)
    subprocess.run(git + ["add", "-A"], check=True)
    subprocess.run(git + ["commit", "-q", "-m", "pin"], check=True)
    head = subprocess.run(git[:3] + ["rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    resolved = root / "Package.resolved"

    def pins(*states):
        resolved.write_text(json.dumps({"pins": [{"identity": "onnxruntime-swift-package-manager", "state": s}
                                                 for s in states], "version": 3}))
    pins({"revision": head, "version": "1.24.2"})
    code, out, err = run(["packages", "--resolved", str(resolved), "--checkouts", str(checkouts)], None)
    check(code == 0 and "plugins Gen; binary targets onnxruntime" in err, err)
    for state, why in (({"version": "1.24.2"}, "not pinned to a revision"), ({"revision": "0" * 40}, "checked out at")):
        pins(state)
        code, out, err = run(["packages", "--resolved", str(resolved), "--checkouts", str(checkouts)], None)
        check(code == 2 and why in err, err)
    real = ROOT / "ios/Reins.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved"
    real_pins = json.loads(real.read_text())["pins"]
    check(all(re.fullmatch(r"[0-9a-f]{40}", p["state"].get("revision", "")) for p in real_pins),
          "the app's packages are pinned to revisions")

# The workflow: one job, ios-testflight, after this run's publication, builds, signs and uploads. It never holds
# publication back, and uploads one release at a time.
workflow = (ROOT / ".github/workflows/release.yml").read_text()
job = workflow.split("\n  ios-testflight:\n", 1)[1].split("\n\n  # ", 1)[0]
head = job.split("\n    steps:\n", 1)[0]
publisher = workflow.split("\n  publish:\n", 1)[1].split("\n    steps:\n", 1)[0]
check("ios-testflight" not in publisher and "ios-build" not in publisher, "publish must not wait for TestFlight")
check("\n  ios-build:\n" not in workflow and "needs.ios-build" not in workflow, "one iOS job")
check("    needs: [plan, publish]\n" in head and "needs.publish.result == 'success'" in head
      and "inputs.testflight_tag != ''" in head, "after publication, or a testflight_tag run")
check(job.count("xcodebuild -exportArchive") == 1 and '"destination": "upload",' in job and "altool" not in job,
      "the upload is the export with destination upload")
check("group: ${{ (inputs.testflight_tag || inputs.verify_only) && format('release-testflight-{0}', github.run_id) "
      "|| 'release-testflight' }}" in head and "cancel-in-progress: true" in head,
      "one upload at a time, the newest release's; a verify_only run cancels none")
check("timeout-minutes: 105\n" in head and "--timeout 1200" in job, "a stuck upload does not hold the next one long")
section = job
check("runs-on: xcode-27" in section and "Xcode_27.0.app" in section, "a released Xcode")
# Trust: main only, in its environment; secrets stay in files.
check("github.ref == 'refs/heads/main'" in section and "environment: testflight" in section, "main, environment")
check('git merge-base --is-ancestor "$SHA" origin/main' in section and "compare/$sha...main" in section, "on main")
check("releases/tags/$tag" in section, "a published release")
check("GITHUB_ENV" not in section, "nothing exported to the job")
check("--key-id" not in section and "--issuer-id" not in section, "the key ids reach the script by environment")
check("Infisical" not in section and "release-secrets" not in section
      and "id-token" not in section and "INFISICAL_" not in section, "no Infisical in the job")
permissions = section.split("    permissions:\n", 1)[1].split("\n    outputs:", 1)[0].split("\n    env:", 1)[0]
check(permissions.split("#", 1)[0].split() == ["contents:", "read"], "least permissions")
for line in section.splitlines():
    if "uses: " in line and "uses: ./" not in line:
        check(re.search(r"@[0-9a-f]{40} # v", line), f"pinned to a commit: {line.strip()}")
removal = section.rsplit("\n      - name: Remove signing material\n", 1)[1]
check(removal.startswith("        if: always()") and '"$RUNNER_TEMP/ios-signing"' in removal
      and '"$RUNNER_TEMP/Reins.xcarchive"' in removal and "delete-keychain" in removal
      and "Provisioning Profiles/$uuid.mobileprovision" in removal and "number.txt" in removal,
      "signing material and the archive removed")
check("scripts/testflight.py prepare" in job and "scripts/testflight.py finish" in job, "the script")
check(job.count("scripts/testflight.py prepare") == 2 and "uploaded=true" in job, "a rerun does not upload again")
check('[[ "$object" == "$sha" ]]' in job and '"$VERIFY_ONLY" != true' in job, "the tag names the commit built")
check("pull_request" not in workflow.split("\njobs:\n", 1)[0], "no pull request trigger")


def parse(section):
    parts = section.split("\n      - ")[1:]
    return ({part.split("\n", 1)[0].removeprefix("name: ").strip(): part for part in parts},
            [part.split("\n", 1)[0] for part in parts])


steps, order = parse(job)


def at(name, within=None):
    return next(i for i, step in enumerate(within or order) if name in step)


# The signing material comes only from the testflight environment's secrets, each mapped only into the steps that need
# it, all after the build; no Infisical, no OIDC token.
first_secret = min(at(name) for name, step in steps.items() if "secrets." in step)
for build_step in ("Build the Rust core", "Resolve and check the Swift packages", "rust-cache@"):
    check(at(build_step) < first_secret, f"{build_step} before the secrets")
asc = {"AC_API_KEY_P8", "AC_API_KEY_ID", "AC_API_ISSUER_ID", "APPLE_TEAM_ID"}
signing = {
    "Signing identity": {"IOS_DIST_CERT_P12", "IOS_DIST_CERT_PASSWORD", "APPLE_TEAM_ID"},
    "Provisioning profiles": {"IOS_PROFILE_APP", "IOS_PROFILE_WIDGETS", "IOS_PROFILE_NOTIFICATIONS", "APPLE_TEAM_ID"},
}
allowed = {"Build number": asc, **signing, "App settings": {"TELEGRAM_API_ID", "TELEGRAM_API_HASH"},
           "App Store Connect key": asc}
for name, step in steps.items():
    used = set(re.findall(r"\$\{\{ secrets\.([A-Z0-9_]+) \}\}", step))
    check(used <= allowed.get(name, set()), f"{name} maps {sorted(used)}")
check(set().union(*allowed.values()) - {"TELEGRAM_API_ID", "TELEGRAM_API_HASH"}
      == set(re.findall(r"secrets\.((?:AC|IOS|APPLE)_[A-Z0-9_]+)", job)), "all nine, and only those")
check("--need asc " in steps["App Store Connect key"] and "--need asc " in steps["Build number"]
      and "--need cert " in steps["Signing identity"] and "--need profiles " in steps["Provisioning profiles"],
      "written by the script, each step only its own secrets")
# The archive sees no certificate file and no API key: the key is removed after the build number; a step checks the
# folder first. The export options and the key for the upload come after the archive; the keychain is locked after the
# upload, before TestFlight's processing wait.
check('trap \'rm -f "$dir/AuthKey.p8" "$dir/asc.env"\' EXIT' in steps["Build number"], "the key removed")
check(at("The release to build") < at("actions/checkout@") < at("Build number") < at("Signing identity")
      < at("Provisioning profiles") < at("App settings") < at("Only the keychain") < at("Archive")
      < at("Export options") < at("App Store Connect key") < at("Upload to App Store Connect")
      < at("Lock the keychain") < at("name: TestFlight\n", [step + "\n" for step in order])
      < at("Remove signing material"), order)
check("ExportOptions" not in "".join(part for name, part in steps.items() if at(name) < at("Archive") + 1),
      "no export options while the app is built")
for name in ("Export options", "App Store Connect key", "Upload to App Store Connect", "TestFlight"):
    check("inputs.verify_only != true" in steps[name].split("\n", 3)[1] + steps[name].split("\n", 3)[2],
          f"{name}: a verify_only run uploads nothing")
check("security lock-keychain" in steps["Lock the keychain"]
      and steps["Lock the keychain"].split("\n", 2)[1].strip() == "if: always()", "locked after the upload")
check("-authenticationKeyPath \"$dir/AuthKey.p8\"" in steps["Upload to App Store Connect"]
      and '"$plist"' in steps["Upload to App Store Connect"]
      and "steps.number.outputs.build-number" in steps["Upload to App Store Connect"],
      "the archive's version checked, the key a file")
check('-P "$IOS_DIST_CERT_PASSWORD"' not in job and '-passin "file:' in job, "the password never on a command line")
check("scripts/testflight.py secrets" in job, "masked and written by the script")
check("--need cert " in steps["Signing identity"] and "--need profiles " in steps["Provisioning profiles"],
      "each step writes only its own secrets")
identity_step = steps["Signing identity"]
check("-x -P" in identity_step and "-T /usr/bin/codesign\n" in identity_step
      and "-T /usr/bin/security" not in identity_step,
      "non-extractable, for codesign only")
check("-S apple-tool:,apple: -s" in identity_step and "security export" in identity_step, "partition list, checked")
check('rm -f "$dir/dist.p12" "$dir/dist.pass"' in identity_step and 'rm -f "$dir/import.p12"' in identity_step,
      "certificate files removed at once")
check("upload-artifact" not in job and "download-artifact" not in job, "the archive never leaves the runner")
# Swift packages: only as resolved, checked, never with plugin or macro validation skipped.
calls = re.findall(r"xcodebuild (?:archive|-resolvePackageDependencies)(?:[^\n]*\\\n)*[^\n]*", job)
check(len(calls) == 2, calls)
for call in calls:
    check("-onlyUsePackageVersionsFromResolvedFile" in call and "-disableAutomaticPackageResolution" in call, call)
check("skipPackagePluginValidation" not in job and "skipMacroValidation" not in job, "validation kept")
check("scripts/testflight.py packages" in steps["Resolve and check the Swift packages"], "packages checked")
check('cache-bin: "false"' in job and 'REINS_SKIP_CORE: "1"' in steps["Archive"], "no cached binaries, no cargo")
build_core = (ROOT / "ios/scripts/build-core.sh").read_text()
check(build_core.index('REINS_SKIP_CORE:-}" == "1"') < build_core.index("rustup target add")
      < build_core.index("run_cargo rustc"), "build-core.sh stops before rustup and cargo when skipping")
# The built core is kept by its sources, with the bindings it was generated with, and only main writes it.
saved = job.split("uses: actions/cache/save@", 1)[1].split("\n      - ", 1)[0]
check("github.ref == 'refs/heads/main'" in saved and "ios/Core/Generated/reins_core.swift" in saved
      and "crates/reins-core/**" in steps["uses: actions/cache/restore@caa296126883cff596d87d8935842f9db880ef25 # v5.1.0"],
      "the core's own cache")
check(at("cache/save@") < first_secret, "the core is saved before any secret is in the job")
check("--profile host-tool" in build_core and "target/host-tool/uniffi-bindgen" in build_core,
      "uniffi-bindgen is a cheap host build")
print(f"{checks} TestFlight checks passed")
