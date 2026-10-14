#!/usr/bin/env python3
"""TestFlight through the App Store Connect API, around the upload the release workflow makes (release.yml, job
`ios-testflight`). Three commands:

    scripts/testflight.py prepare --version 0.6.0 --minimum 29860000
        The build number to upload: the larger of --minimum (the release's versionCode, minutes since 1970) and one
        more than the highest build App Store Connect has for the app, so it never collides with a build uploaded by
        hand. If a build of this version numbered --minimum or higher is there already (a rerun after the upload
        went through), that build is reused: prints `uploaded=true`. Prints `build-number=N` and `uploaded=...`.

    scripts/testflight.py finish --version 0.6.0 --build-number 29860000 --notes-file notes.md \
            --group Internal --group 228ac7ac-... [--info-plist ios/Reins/Info.plist]
        Waits until the build has been processed, then sets its export compliance (usesNonExemptEncryption, from
        ITSAppUsesNonExemptEncryption in --info-plist), its "What to Test" (from the release notes), adds it to each
        --group (a group id, or a name that must match exactly one group), and submits it for beta app review when
        one of them is external. A refused review submission (a build already in review, missing test information,
        ...) is a warning, not a failure.

    scripts/testflight.py profile --plist decoded.plist --bundle-id com.reins2fa.app --team 5DRCQV8HY2
        Checks a provisioning profile (as `security cms -D` decodes it) is an App Store profile of that team for that
        bundle id, still valid, and prints its UUID.

Credentials (prepare, finish): an App Store Connect API key, --key-file (the .p8), --key-id and --issuer-id (or
AC_API_KEY_ID, AC_API_ISSUER_ID). Each request carries a short-lived JWT (ES256, signed with `openssl`); neither the
key nor the token is printed. Every request is retried on network errors, 429 and 5xx. Standard library only.

Exit codes: 0 done, 1 failed, 2 bad arguments or files.
"""

import argparse
import base64
import datetime
import json
import os
import plistlib
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

API = "https://api.appstoreconnect.apple.com/v1"
AUDIENCE = "appstoreconnect-v1"
BUNDLE_ID = "com.reins2fa.app"
LOCALE = "en-US"
WHAT_TO_TEST_LIMIT = 4000
TOKEN_LIFETIME = 15 * 60  # Apple accepts at most 20 minutes
TRANSIENT = frozenset({0, 408, 429, 500, 502, 503, 504})
# The beta review states in which a build needs no (new) submission.
REVIEWED = frozenset({"WAITING_FOR_BETA_REVIEW", "IN_BETA_REVIEW", "BETA_APPROVED", "IN_BETA_TESTING"})


class AscError(Exception):
    """A failed request: the HTTP status (0: no answer), Apple's error code and its detail."""

    def __init__(self, status, detail, code=""):
        super().__init__(f"HTTP {status}{f' {code}' if code else ''}: {detail}")
        self.status = status
        self.code = code
        self.detail = detail

    @property
    def refused(self):
        """Apple understood the request and declined it in this state (409 conflict, 422 unprocessable)."""
        return self.status in (409, 422)


def log(message):
    print(f"testflight: {message}", file=sys.stderr)


def annotate(kind, message, environ=os.environ):
    """A ::warning:: or ::error:: on the run's summary under GitHub Actions; a plain line elsewhere."""
    if environ.get("GITHUB_ACTIONS") == "true":
        print(f"::{kind}::{message}", file=sys.stderr)
    else:
        log(f"{kind}: {message}")


def b64url(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def der_to_raw(der, size=32):
    """An ECDSA signature from DER (SEQUENCE of two INTEGERs, what openssl writes) to JWS's r || s."""
    if len(der) < 8 or der[0] != 0x30:
        raise ValueError("not a DER ECDSA signature")
    i = 2 if der[1] < 0x80 else 2 + (der[1] & 0x7F)
    out = b""
    for _ in range(2):
        if i + 2 > len(der) or der[i] != 0x02:
            raise ValueError("not a DER ECDSA signature")
        length = der[i + 1]
        value = der[i + 2:i + 2 + length].lstrip(b"\0")
        i += 2 + length
        if len(value) > size:
            raise ValueError("signature integer too long for P-256")
        out += value.rjust(size, b"\0")
    return out


def sign_es256(key_pem, data):
    """ES256 with `openssl`: the key goes to a private temporary file that is removed right after."""
    with tempfile.TemporaryDirectory(prefix="testflight-") as directory:
        key = Path(directory) / "key.p8"
        key.touch(mode=0o600)
        key.write_text(key_pem)
        result = subprocess.run(["openssl", "dgst", "-sha256", "-sign", str(key)], input=data, capture_output=True,
                                check=False)
    if result.returncode != 0 or not result.stdout:
        raise ValueError("openssl could not sign with the App Store Connect key (is it the .p8 text?)")
    return der_to_raw(result.stdout)


def jwt_parts(key_id, issuer_id, now):
    header = {"alg": "ES256", "kid": key_id, "typ": "JWT"}
    claims = {"iss": issuer_id, "iat": int(now), "exp": int(now) + TOKEN_LIFETIME, "aud": AUDIENCE}
    return header, claims


def make_jwt(key_pem, key_id, issuer_id, now=None):
    header, claims = jwt_parts(key_id, issuer_id, time.time() if now is None else now)
    signing_input = f"{b64url(json.dumps(header).encode())}.{b64url(json.dumps(claims).encode())}"
    return f"{signing_input}.{b64url(sign_es256(key_pem, signing_input.encode()))}"


# The JSON:API bodies this script sends.

def encryption_body(build_id, uses_non_exempt):
    return {"data": {"type": "builds", "id": build_id, "attributes": {"usesNonExemptEncryption": uses_non_exempt}}}


def localization_create_body(build_id, text, locale=LOCALE):
    return {"data": {"type": "betaBuildLocalizations", "attributes": {"locale": locale, "whatsNew": text},
                     "relationships": {"build": {"data": {"type": "builds", "id": build_id}}}}}


def localization_update_body(localization_id, text):
    return {"data": {"type": "betaBuildLocalizations", "id": localization_id, "attributes": {"whatsNew": text}}}


def group_builds_body(build_id):
    return {"data": [{"type": "builds", "id": build_id}]}


def review_submission_body(build_id):
    return {"data": {"type": "betaAppReviewSubmissions",
                     "relationships": {"build": {"data": {"type": "builds", "id": build_id}}}}}


def what_to_test(markdown, version, release_url=None, limit=WHAT_TO_TEST_LIMIT):
    """TestFlight's "What to Test" from the GitHub release notes (scripts/release-notes.sh): the changes, as plain
    text, without commit links or the install section, within Apple's 4000 characters."""
    text = markdown.split("**Full changelog:**", 1)[0].split("## Install", 1)[0]
    lines = []
    for line in text.splitlines():
        line = re.sub(r"\s*\(\[`[0-9a-f]{7,40}`\]\([^)]*\)\)", "", line)  # ([`abc1234`](commit url))
        line = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", line)  # other links keep their text
        line = re.sub(r"^#+\s*(.+)$", r"\1:", line)
        line = re.sub(r"^>\s?", "", line)
        line = line.replace("**", "").replace("`", "")
        line = re.sub(r"\\([\\*<>_`\[\]])", r"\1", line)
        lines.append(line.rstrip())
    body = re.sub(r"\n{3,}", "\n\n", "\n".join(lines)).strip()
    head = f"Reins {version}"
    tail = f"\n\nAll changes: {release_url}" if release_url else ""
    room = limit - len(head) - len(tail) - 2
    if len(body) > room:
        body = body[:max(room - 1, 0)].rstrip() + "…"
    return f"{head}\n\n{body}{tail}" if body else f"{head}{tail}"


def build_number_value(text):
    """A CFBundleVersion ("12", "1.2.3") as a tuple of integers, or None for anything else."""
    try:
        return tuple(int(part) for part in str(text).split("."))
    except ValueError:
        return None


def next_build_number(existing, minimum):
    """The larger of `minimum` and one more than the highest build number in `existing`."""
    values = [v for v in map(build_number_value, existing) if v]
    highest = max(values, default=(0,))
    return max(int(minimum), highest[0] + 1)


def uses_non_exempt_encryption(info_plist):
    """ITSAppUsesNonExemptEncryption from the app's Info.plist (None when the key is absent)."""
    with open(info_plist, "rb") as handle:
        value = plistlib.load(handle).get("ITSAppUsesNonExemptEncryption")
    return None if value is None else bool(value)


def check_profile(profile, bundle_id, team, now=None):
    """A decoded provisioning profile (dict) must be an App Store profile of `team` for `bundle_id`; returns its
    UUID and name."""
    entitlements = profile.get("Entitlements") or {}
    name = profile.get("Name", "?")
    problems = []
    if team not in (profile.get("TeamIdentifier") or []):
        problems.append(f"team {profile.get('TeamIdentifier')} is not {team}")
    if entitlements.get("application-identifier") != f"{team}.{bundle_id}":
        problems.append(f"it is for {entitlements.get('application-identifier')}, not {team}.{bundle_id}")
    if profile.get("ProvisionedDevices") or profile.get("ProvisionsAllDevices") or entitlements.get("get-task-allow"):
        problems.append("it is a development, ad hoc or enterprise profile, not App Store")
    expires = profile.get("ExpirationDate")
    now = now or datetime.datetime.now(datetime.timezone.utc)
    if isinstance(expires, datetime.datetime):
        if expires.tzinfo is None:
            expires = expires.replace(tzinfo=datetime.timezone.utc)
        if expires <= now:
            problems.append(f"it expired on {expires:%Y-%m-%d}")
    if not profile.get("UUID"):
        problems.append("it has no UUID")
    if problems:
        raise ValueError(f"profile {name!r} for {bundle_id}: " + "; ".join(problems))
    return profile["UUID"], name


class Http:
    """The network, behind one method so the tests can replace it. A transient failure is sent again after each of
    `delays` (seconds); any other answer, or one that keeps failing, is raised."""

    def __init__(self, delays=(5, 15, 45, 90)):
        self.delays = tuple(delays)

    def request(self, method, url, headers=None, body=None, timeout=60):
        for delay in (*self.delays, None):
            try:
                return self.send(method, url, headers, body, timeout)
            except AscError as error:
                if delay is None or error.status not in TRANSIENT:
                    raise
                log(f"{method} {url.split('?')[0]}: {error}; again in {delay} s")
                time.sleep(delay)

    @staticmethod
    def send(method, url, headers, body, timeout):
        data = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(url, data=data, method=method, headers=headers or {})
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                text = response.read().decode()
        except urllib.error.HTTPError as error:
            raise AscError(error.code, *error_detail(error.read().decode(errors="replace"))) from None
        except urllib.error.URLError as error:
            raise AscError(0, f"{url.split('?')[0]}: {error.reason}") from None
        except (ConnectionError, TimeoutError) as error:
            raise AscError(0, f"{url.split('?')[0]}: {error}") from None
        return json.loads(text) if text.strip() else {}


def error_detail(text):
    """(detail, code) of Apple's first error, without anything else the body holds."""
    try:
        errors = json.loads(text).get("errors") or []
    except (json.JSONDecodeError, AttributeError):
        return (text.strip()[:500] or "no details"), ""
    if not errors:
        return "no details", ""
    first = errors[0]
    return str(first.get("detail") or first.get("title") or "no details"), str(first.get("code") or "")


class Asc:
    def __init__(self, http, key_pem, key_id, issuer_id, clock=time.time, environ=os.environ):
        self.http = http
        self.environ = environ
        self.key = (key_pem, key_id, issuer_id)
        self.clock = clock
        self.token = None
        self.minted = 0.0

    def warn(self, message):
        annotate("warning", message, self.environ)

    def call(self, method, path, body=None):
        if self.token is None or self.clock() - self.minted > TOKEN_LIFETIME - 300:
            self.minted = self.clock()
            self.token = make_jwt(*self.key, now=self.minted)
        url = path if path.startswith("https://") else f"{API}/{path}"
        headers = {"Authorization": f"Bearer {self.token}"}
        if body is not None:
            headers["Content-Type"] = "application/json"
        return self.http.request(method, url, headers, body)

    def pages(self, path):
        """Every item of a list, following `links.next`."""
        items = []
        while path:
            page = self.call("GET", path)
            items += page.get("data") or []
            path = (page.get("links") or {}).get("next")
        return items

    def app_id(self, bundle_id):
        apps = self.call("GET", "apps?" + urllib.parse.urlencode({"filter[bundleId]": bundle_id, "limit": 2}))
        found = [app for app in apps.get("data") or [] if app["attributes"].get("bundleId") == bundle_id]
        if not found:
            raise AscError(404, f"no app with bundle id {bundle_id} in App Store Connect (or the key cannot see it)")
        return found[0]["id"]

    def builds(self, app_id, version=None, number=None):
        query = {"filter[app]": app_id, "limit": 200, "fields[builds]": "version,processingState,"
                 "usesNonExemptEncryption,uploadedDate"}
        if version:
            query["filter[preReleaseVersion.version]"] = version
        if number:
            query["filter[version]"] = str(number)
        return self.pages("builds?" + urllib.parse.urlencode(query))

    def prepare(self, bundle_id, version, minimum):
        """(build number, already uploaded)."""
        app = self.app_id(bundle_id)
        mine = [b for b in self.builds(app, version)
                if (build_number_value(b["attributes"].get("version")) or (0,)) >= (int(minimum),)]
        if mine:
            number = max(mine, key=lambda b: build_number_value(b["attributes"]["version"]))["attributes"]["version"]
            log(f"{version} ({number}) is in App Store Connect already: not uploaded again")
            return number, True
        existing = [b["attributes"].get("version") for b in self.builds(app)]
        number = next_build_number(existing, minimum)
        log(f"{len(existing)} builds in App Store Connect; this one is {version} ({number})")
        return str(number), False

    def wait_processed(self, app_id, version, number, timeout, interval):
        deadline = self.clock() + timeout
        state = "not there yet"
        while True:
            found = self.builds(app_id, version, number)
            if found:
                build = found[0]
                state = build["attributes"].get("processingState")
                if state == "VALID":
                    log(f"{version} ({number}) is processed")
                    return build
                if state in ("FAILED", "INVALID"):
                    raise AscError(0, f"App Store Connect could not process {version} ({number}): {state} "
                                      "(its email to the account holder says why)")
            if self.clock() + interval >= deadline:
                raise AscError(0, f"{version} ({number}) was not processed within {timeout // 60} minutes ({state})")
            log(f"{version} ({number}): {state or 'not there yet'}; again in {interval} s")
            time.sleep(interval)

    def set_encryption(self, build, uses):
        current = build["attributes"].get("usesNonExemptEncryption")
        if uses is None:
            if current is None:
                self.warn("Export compliance is unanswered and the Info.plist has no "
                          "ITSAppUsesNonExemptEncryption: answer it in App Store Connect")
            return
        if current == uses:
            log(f"export compliance: usesNonExemptEncryption={str(uses).lower()} already")
            return
        if uses:
            self.warn("The app declares non-exempt encryption: export compliance documents are needed in "
                      "App Store Connect before testers get the build")
            return
        self.call("PATCH", f"builds/{build['id']}", encryption_body(build["id"], uses))
        log("export compliance: usesNonExemptEncryption=false")

    def set_what_to_test(self, build_id, text, locale=LOCALE):
        existing = self.call("GET", f"builds/{build_id}/betaBuildLocalizations?limit=50").get("data") or []
        same = [loc for loc in existing if loc["attributes"].get("locale") == locale]
        if same:
            self.call("PATCH", f"betaBuildLocalizations/{same[0]['id']}", localization_update_body(same[0]["id"], text))
        else:
            self.call("POST", "betaBuildLocalizations", localization_create_body(build_id, text, locale))
        log(f"What to Test ({locale}): {len(text)} characters")

    def resolve_groups(self, app_id, wanted):
        """Each wanted group (an id, or a name only one group has) as the API's group, and the errors."""
        groups = self.pages("betaGroups?" + urllib.parse.urlencode(
            {"filter[app]": app_id, "limit": 200,
             "fields[betaGroups]": "name,isInternalGroup,hasAccessToAllBuilds,publicLinkEnabled"}))
        resolved, errors = [], []
        for spec in wanted:
            by_id = [g for g in groups if g["id"] == spec]
            by_name = [g for g in groups if g["attributes"].get("name") == spec]
            if by_id:
                resolved.append(by_id[0])
            elif len(by_name) == 1:
                resolved.append(by_name[0])
            elif by_name:
                ids = ", ".join(g["id"] for g in by_name)
                errors.append(f"{len(by_name)} TestFlight groups are named {spec!r} ({ids}): name one by its id "
                              "(the repository variable TESTFLIGHT_EXTERNAL_GROUP_ID for the external group)")
            else:
                names = ", ".join(sorted(f"{g['attributes'].get('name')} ({g['id']})" for g in groups)) or "none"
                errors.append(f"No TestFlight group {spec!r} (groups: {names})")
        return resolved, errors

    def add_to_group(self, group, build_id):
        attributes = group["attributes"]
        label = f"{attributes.get('name')!r} ({'internal' if attributes.get('isInternalGroup') else 'external'})"
        if attributes.get("isInternalGroup") and attributes.get("hasAccessToAllBuilds"):
            log(f"group {label} gets every build by itself")
            return
        try:
            self.call("POST", f"betaGroups/{group['id']}/relationships/builds", group_builds_body(build_id))
        except AscError as error:
            if not error.refused:
                raise
            self.warn(f"TestFlight group {label} did not take the build: {error}")
            return
        log(f"added to group {label}")

    def submit_for_review(self, build_id):
        detail = self.call("GET", f"builds/{build_id}/buildBetaDetail").get("data") or {}
        state = (detail.get("attributes") or {}).get("externalBuildState")
        if state in REVIEWED:
            log(f"beta app review: nothing to submit ({state})")
            return state
        try:
            self.call("POST", "betaAppReviewSubmissions", review_submission_body(build_id))
        except AscError as error:
            if not error.refused:
                raise
            self.warn(f"Beta app review did not take the build ({error}). Testers in the external group "
                      "get it once a build of this version is approved; submit it in App Store Connect "
                      "if this is not expected.")
            return "REFUSED"
        log("submitted for beta app review")
        return "SUBMITTED"

    def finish(self, bundle_id, version, number, notes, groups, uses_encryption, timeout=5400, interval=30):
        app = self.app_id(bundle_id)
        resolved, errors = self.resolve_groups(app, groups)
        build = self.wait_processed(app, version, number, timeout, interval)
        self.set_encryption(build, uses_encryption)
        self.set_what_to_test(build["id"], notes)
        for group in resolved:
            self.add_to_group(group, build["id"])
        if any(not g["attributes"].get("isInternalGroup") for g in resolved):
            self.submit_for_review(build["id"])
        return errors


def load_key(args, environ):
    key_id = args.key_id or environ.get("AC_API_KEY_ID", "")
    issuer = args.issuer_id or environ.get("AC_API_ISSUER_ID", "")
    if not (args.key_file and key_id and issuer):
        raise ValueError("an App Store Connect API key is needed: --key-file, --key-id and --issuer-id "
                         "(or AC_API_KEY_ID, AC_API_ISSUER_ID)")
    key = Path(args.key_file).read_text()
    if "PRIVATE KEY" not in key:
        raise ValueError(f"{args.key_file} is not a .p8 private key")
    return key, key_id, issuer


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("prepare", "finish"):
        command = commands.add_parser(name)
        command.add_argument("--bundle-id", default=BUNDLE_ID)
        command.add_argument("--version", required=True, help="the marketing version (CFBundleShortVersionString)")
        command.add_argument("--key-file", help="the App Store Connect API key (.p8)")
        command.add_argument("--key-id")
        command.add_argument("--issuer-id")
    prepare = commands.choices["prepare"]
    prepare.add_argument("--minimum", type=int, required=True, help="the lowest build number to use")
    finish = commands.choices["finish"]
    finish.add_argument("--build-number", required=True)
    finish.add_argument("--notes-file", help="the release notes (Markdown); empty or missing: the version only")
    finish.add_argument("--release-url", help="named at the end of What to Test")
    finish.add_argument("--group", action="append", default=[], help="a TestFlight group id or name (repeatable)")
    finish.add_argument("--info-plist", default=str(Path(__file__).resolve().parent.parent / "ios/Reins/Info.plist"))
    finish.add_argument("--timeout", type=int, default=5400, help="seconds to wait for processing")
    finish.add_argument("--interval", type=int, default=30)
    profile = commands.add_parser("profile")
    profile.add_argument("--plist", required=True, help="the profile as `security cms -D -i` decodes it")
    profile.add_argument("--bundle-id", required=True)
    profile.add_argument("--team", required=True)
    return parser.parse_args(argv)


def main(argv=None, http=None, environ=os.environ):
    args = parse_args(argv)
    try:
        if args.command == "profile":
            with open(args.plist, "rb") as handle:
                uuid, name = check_profile(plistlib.load(handle), args.bundle_id, args.team)
            log(f"{args.bundle_id}: {name} ({uuid})")
            print(uuid)
            return 0
        asc = Asc(http or Http(), *load_key(args, environ), environ=environ)
        if args.command == "prepare":
            number, uploaded = asc.prepare(args.bundle_id, args.version, args.minimum)
            print(f"build-number={number}")
            print(f"uploaded={str(uploaded).lower()}")
            return 0
        notes = Path(args.notes_file).read_text() if args.notes_file and Path(args.notes_file).is_file() else ""
        text = what_to_test(notes, args.version, args.release_url)
        uses = uses_non_exempt_encryption(args.info_plist)
        groups = list(dict.fromkeys(g.strip() for spec in args.group for g in spec.split(",") if g.strip()))
        errors = asc.finish(args.bundle_id, args.version, args.build_number, text, groups, uses, args.timeout,
                            args.interval)
    except (OSError, ValueError, plistlib.InvalidFileException) as error:
        annotate("error", str(error), environ)
        return 2
    except AscError as error:
        annotate("error", f"App Store Connect: {error}", environ)
        return 1
    for error in errors:
        annotate("error", error, environ)
    if errors:
        return 1
    log(f"{args.version} ({args.build_number}) is on TestFlight")
    return 0


if __name__ == "__main__":
    sys.exit(main())
