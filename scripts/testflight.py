#!/usr/bin/env python3
"""TestFlight through the App Store Connect API, around the upload the release workflow makes (release.yml, job
`ios-testflight`). Its commands:

    scripts/testflight.py prepare --version 0.6.0 --minimum 29860000
        The build number to upload: the larger of --minimum (the release's versionCode, minutes since 1970) and one
        more than the highest build App Store Connect has for the app, so it never collides with a build uploaded by
        hand. If a build of this version numbered --minimum or higher is there already (a rerun after the upload
        went through), that build is reused: prints `uploaded=true`. Prints `build-number=N` and `uploaded=...`.

    scripts/testflight.py finish --version 0.6.0 --build-number 29860000 --notes-file notes.md \
            --group Internal --group 228ac7ac-...
        Waits until the build has been processed, reports its export compliance (never changes it: see below), sets
        its "What to Test" (from the release notes), adds it to each
        --group (a group id, or a name that must match exactly one group), and submits it for beta app review when
        one of them is external. A refused review submission (a build already in review, missing test information,
        ...) is a warning, not a failure. Apple takes only a few submissions a day, so none is made while a build of
        the app went to review in the last 24 hours (a notice: the next day's first release goes), and Apple's own
        "submission limit reached" is a notice too.

    scripts/testflight.py submit --version 0.6.0 --build-number 29860000 [--release-type MANUAL] [--dry-run]
        Sends a processed build to App Store review (.github/workflows/ios-app-store.yml, by hand only): finds or
        creates the App Store version of --version (iOS), sets its release type (MANUAL by default: the maintainer
        releases it after approval), attaches the build, and submits the version through a review submission (a
        draft submission of the app is reused). Refuses, before any request and with --dry-run too, unless
        EXPORT_COMPLIANCE_CODE is set in the environment (see below). Fails when the build is not processed, does not
        declare non-exempt encryption, the app has no approved encryption declaration with that code, the version is
        past "Prepare for Submission" (or rejected), or the app has a submission waiting for or in review. --dry-run
        reads and reports only. The version's metadata, screenshots,
        privacy answers and review information are App Store Connect's (docs/ios-app-store-submission.md); Apple
        refuses the submission while any is missing, and the error says which.

    scripts/testflight.py profile --plist decoded.plist --bundle-id com.reins2fa.app --team 5DRCQV8HY2
        Checks a provisioning profile (as `security cms -D` decodes it) is an App Store profile of that team for that
        bundle id, still valid, and prints its UUID.

    scripts/testflight.py secrets --need cert|profiles|asc --out DIR
        Writes the signing secrets one step needs, from its environment variables of the same names, as files (mode
        0600) in DIR (0700), with `team` (APPLE_TEAM_ID): `cert` the .p12 and its password, `profiles` the three
        profiles, `asc` the API key (.p8) and asc.env (its ids). Every value is masked (::add-mask::) before anything
        else is printed; missing names fail, all named at once, and nothing is written then.

    scripts/testflight.py packages --resolved Package.resolved --checkouts DIR
        The Swift packages the archive builds: every pin must name a revision and be checked out at it. Lists each
        package's plugins and binary targets.

Export compliance: Reins uses standard non-exempt encryption and is distributed in France, so every build declares
ITSAppUsesNonExemptEncryption = YES and carries ITSEncryptionExportComplianceCode, the code of the app's encryption
declaration Apple approved (ios/scripts/export-compliance.sh adds it to the archive from REINS_EXPORT_COMPLIANCE_CODE).
This script never sets a build's usesNonExemptEncryption: the archive's Info.plist answers it. `submit` needs the same
code as EXPORT_COMPLIANCE_CODE (the release secret ITS_ENCRYPTION_EXPORT_COMPLIANCE_CODE) and checks App Store Connect
has that declaration approved and the build declares the encryption before it sends anything.

Credentials (prepare, finish, submit): an App Store Connect API key, --key-file (the .p8), with AC_API_KEY_ID and
AC_API_ISSUER_ID in the environment (never on the command line). Each request carries a short-lived JWT (ES256,
signed with `openssl`); neither the key nor the token is printed. Every request is retried on network errors, 429 and
5xx. Standard library only.

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
# Beta app review takes only a few submissions a day: at most one every this many hours.
REVIEW_SPACING_HOURS = 24
# A build submitted for review was uploaded within this many days (a build can be submitted days after its upload).
REVIEW_LOOKBACK_DAYS = 14
# App Store versions that can still take a build and be submitted.
EDITABLE_VERSION_STATES = frozenset({"PREPARE_FOR_SUBMISSION", "DEVELOPER_REJECTED", "REJECTED", "METADATA_REJECTED",
                                     "INVALID_BINARY"})
# Review submissions of the app that block a new one.
OPEN_REVIEW_STATES = ("WAITING_FOR_REVIEW", "IN_REVIEW", "UNRESOLVED_ISSUES", "CANCELING")
RELEASE_TYPES = ("MANUAL", "AFTER_APPROVAL")
# App Store Connect's export compliance code, as ios/scripts/export-compliance.sh takes it.
EXPORT_COMPLIANCE_CODE_PATTERN = r"[A-Za-z0-9][A-Za-z0-9-]*"
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
    """A ::notice::, ::warning:: or ::error:: on the run's summary under GitHub Actions; a plain line elsewhere."""
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


def app_store_version_create_body(app_id, version, release_type):
    return {"data": {"type": "appStoreVersions",
                     "attributes": {"platform": "IOS", "versionString": version, "releaseType": release_type},
                     "relationships": {"app": {"data": {"type": "apps", "id": app_id}}}}}


def app_store_version_update_body(version_id, release_type):
    return {"data": {"type": "appStoreVersions", "id": version_id, "attributes": {"releaseType": release_type}}}


def version_build_body(build_id):
    return {"data": {"type": "builds", "id": build_id}}


def review_create_body(app_id):
    return {"data": {"type": "reviewSubmissions", "attributes": {"platform": "IOS"},
                     "relationships": {"app": {"data": {"type": "apps", "id": app_id}}}}}


def review_item_body(submission_id, version_id):
    return {"data": {"type": "reviewSubmissionItems",
                     "relationships": {"reviewSubmission": {"data": {"type": "reviewSubmissions", "id": submission_id}},
                                       "appStoreVersion": {"data": {"type": "appStoreVersions", "id": version_id}}}}}


def review_submit_body(submission_id):
    return {"data": {"type": "reviewSubmissions", "id": submission_id, "attributes": {"submitted": True}}}


def version_state(version):
    attributes = version.get("attributes") or {}
    return attributes.get("appVersionState") or attributes.get("appStoreState") or "?"


def item_version(item):
    """The App Store version id a review submission item holds, or None."""
    related = ((item.get("relationships") or {}).get("appStoreVersion") or {}).get("data") or {}
    return related.get("id")


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


def parse_time(text):
    """Seconds since 1970 for an App Store Connect time ("2026-10-15T00:15:57-07:00"); None when there is none."""
    if not text:
        return None
    try:
        return datetime.datetime.fromisoformat(text.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


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


def export_compliance_code(environ):
    """EXPORT_COMPLIANCE_CODE (the app's approved encryption declaration's code), or ValueError when unset or not one."""
    code = environ.get("EXPORT_COMPLIANCE_CODE", "").strip()
    if not code:
        raise ValueError("refusing to submit: EXPORT_COMPLIANCE_CODE is not set. Reins uses non-exempt encryption and "
                         "is distributed in France: App Store review needs the code of the app's encryption "
                         "declaration once Apple approves it (the secret "
                         "ITS_ENCRYPTION_EXPORT_COMPLIANCE_CODE; docs/ios-app-store-submission.md, Export compliance)")
    if not re.fullmatch(EXPORT_COMPLIANCE_CODE_PATTERN, code):
        raise ValueError("refusing to submit: EXPORT_COMPLIANCE_CODE is not an App Store Connect export compliance "
                         "code (letters, digits and hyphens)")
    return code


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


DEFAULT_TEAM = "5DRCQV8HY2"
# The secrets per step group, and the files they become.
SECRETS = {
    "cert": ("IOS_DIST_CERT_P12", "IOS_DIST_CERT_PASSWORD"),
    "profiles": ("IOS_PROFILE_APP", "IOS_PROFILE_WIDGETS", "IOS_PROFILE_NOTIFICATIONS"),
    "asc": ("AC_API_KEY_P8", "AC_API_KEY_ID", "AC_API_ISSUER_ID"),
}
PROFILES = (("Reins", "IOS_PROFILE_APP"), ("ReinsWidgets", "IOS_PROFILE_WIDGETS"),
            ("ReinsNotifications", "IOS_PROFILE_NOTIFICATIONS"))


def write_private(path, data):
    with open(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "wb") as handle:
        handle.write(data)


def materialize(kinds, out, environ=os.environ, mask=print):
    """The secrets of `kinds`, from the environment, as files in `out`; returns the file names written."""
    names = [name for kind in kinds for name in SECRETS[kind]] + ["APPLE_TEAM_ID"]
    values = {name: environ.get(name, "") for name in names}
    for value in values.values():
        for line in value.splitlines():
            if line.strip():
                mask(f"::add-mask::{line.strip()}")
    missing = [name for name in names if name != "APPLE_TEAM_ID" and not values[name]]
    if missing:
        raise ValueError(f"TestFlight signing is incomplete, missing: {' '.join(missing)} (secrets of the GitHub "
                         "environment testflight; see CONTRIBUTING.md)")
    team = values["APPLE_TEAM_ID"] or DEFAULT_TEAM
    if not re.fullmatch(r"[A-Z0-9]{10}", team):
        raise ValueError("APPLE_TEAM_ID is malformed")
    os.makedirs(out, mode=0o700, exist_ok=True)
    written = {"team": team.encode() + b"\n"}
    try:
        if "cert" in kinds:
            written["dist.p12"] = base64.b64decode(values["IOS_DIST_CERT_P12"], validate=False)
            written["dist.pass"] = values["IOS_DIST_CERT_PASSWORD"].encode()
        if "profiles" in kinds:
            for target, name in PROFILES:
                written[f"{target}.mobileprovision"] = base64.b64decode(values[name], validate=False)
    except ValueError:
        raise ValueError("IOS_DIST_CERT_P12 and IOS_PROFILE_* must be base64") from None
    if "asc" in kinds:
        key_id, issuer = values["AC_API_KEY_ID"], values["AC_API_ISSUER_ID"]
        if not (re.fullmatch(r"[A-Z0-9]+", key_id) and re.fullmatch(r"[0-9a-f-]+", issuer)):
            raise ValueError("AC_API_KEY_ID or AC_API_ISSUER_ID is malformed")
        if "PRIVATE KEY" not in values["AC_API_KEY_P8"]:
            raise ValueError("AC_API_KEY_P8 is not a .p8 private key")
        written["AuthKey.p8"] = values["AC_API_KEY_P8"].strip().encode() + b"\n"
        written["asc.env"] = f"AC_API_KEY_ID={key_id}\nAC_API_ISSUER_ID={issuer}\nTEAM={team}\n".encode()
    for name, data in written.items():
        write_private(os.path.join(out, name), data)
    return sorted(written)


def audit_packages(resolved, checkouts, run=subprocess.run):
    """Each Package.resolved pin: pinned to a revision and checked out at it. Returns lines describing the packages'
    plugins and binary targets; raises ValueError for anything unpinned or not at its pin."""
    pins = json.loads(Path(resolved).read_text()).get("pins") or []
    report, problems = [], []
    for pin in pins:
        identity = pin.get("identity", "?")
        revision = (pin.get("state") or {}).get("revision", "")
        if not re.fullmatch(r"[0-9a-f]{40}", revision):
            problems.append(f"{identity} is not pinned to a revision")
            continue
        checkout = Path(checkouts) / identity
        head = run(["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True, check=False)
        if head.returncode != 0 or head.stdout.strip() != revision:
            problems.append(f"{identity} is checked out at {head.stdout.strip() or 'nothing'}, not {revision}")
            continue
        manifests = sorted(checkout.glob("Package*.swift"))
        text = "\n".join(m.read_text(errors="replace") for m in manifests)
        plugins = re.findall(r"\.plugin\(\s*name:\s*\"([^\"]+)\"", text)
        binaries = re.findall(r"\.binaryTarget\(\s*name:\s*\"([^\"]+)\"", text)
        report.append(f"{identity} {revision[:12]} ({(pin.get('state') or {}).get('version', 'no version')}): "
                      f"plugins {', '.join(plugins) or 'none'}; binary targets {', '.join(binaries) or 'none'}")
    if problems:
        raise ValueError("; ".join(problems))
    return report


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

    def notice(self, message):
        annotate("notice", message, self.environ)

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

    def report_encryption(self, build):
        """Logs the build's export compliance answer; never changes it (the archive's Info.plist answers it)."""
        current = build["attributes"].get("usesNonExemptEncryption")
        if current is True:
            log("export compliance: usesNonExemptEncryption=true (from the Info.plist)")
        elif current is False:
            self.warn("Export compliance: App Store Connect says this build uses no non-exempt encryption, which is "
                      "wrong for Reins: do not distribute it. Builds from current sources declare "
                      "ITSAppUsesNonExemptEncryption = YES with the approved declaration's code.")
        else:
            self.warn("Export compliance is unanswered for this build (Missing Compliance): testers cannot get it "
                      "until it is. Its archive had no ITSEncryptionExportComplianceCode App Store Connect "
                      "recognised: check ITS_ENCRYPTION_EXPORT_COMPLIANCE_CODE against the approved declaration.")

    def check_encryption(self, app_id, build, code):
        """Refuses (AscError) unless the build declares non-exempt encryption and the app has an approved encryption
        declaration whose code is `code`. Reads only."""
        current = build["attributes"].get("usesNonExemptEncryption")
        if current is not True:
            answer = "unanswered" if current is None else "usesNonExemptEncryption=false"
            raise AscError(409, f"refusing to submit: the build's export compliance is {answer}, but Reins uses "
                                "non-exempt encryption. Upload a build archived with REINS_EXPORT_COMPLIANCE_CODE "
                                "(ITSAppUsesNonExemptEncryption = YES and the approved code) and submit that one")
        declarations = self.pages(f"apps/{app_id}/appEncryptionDeclarations?limit=200")
        same = [d for d in declarations if (d.get("attributes") or {}).get("codeValue") == code]
        if not same:
            raise AscError(409, f"refusing to submit: none of the app's {len(declarations)} encryption declarations "
                                "has the code in EXPORT_COMPLIANCE_CODE")
        states = {(d.get("attributes") or {}).get("appEncryptionDeclarationState") for d in same}
        if "APPROVED" not in states:
            listed = ", ".join(sorted(str(state) for state in states))
            raise AscError(409, f"refusing to submit: the encryption declaration with that code is {listed}, not "
                                "APPROVED: wait for Apple's approval (the French declaration included)")
        log("export compliance: usesNonExemptEncryption=true, encryption declaration APPROVED")

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

    def recent_review(self, app_id, hours=REVIEW_SPACING_HOURS):
        """When a build of the app last went to beta app review, in seconds since 1970, if within `hours`."""
        since = self.clock() - REVIEW_LOOKBACK_DAYS * 86400
        recent = [b["id"] for b in self.builds(app_id)
                  if (uploaded := parse_time(b["attributes"].get("uploadedDate"))) is None or uploaded >= since]
        newest = None
        for start in range(0, len(recent), 50):
            submissions = self.pages("betaAppReviewSubmissions?" + urllib.parse.urlencode(
                {"filter[build]": ",".join(recent[start:start + 50]), "limit": 200,
                 "fields[betaAppReviewSubmissions]": "submittedDate"}))
            for submission in submissions:
                submitted = parse_time(submission["attributes"].get("submittedDate"))
                if submitted is not None and (newest is None or submitted > newest):
                    newest = submitted
        if newest is not None and self.clock() - newest < hours * 3600:
            return newest
        return None

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
            if "SUBMISSION_LIMIT_REACHED" in error.code:
                self.notice("Beta app review takes no more submissions today (Apple's limit). External testers get "
                            "the next release that goes to review; submit this build in App Store Connect to send "
                            "it sooner.")
                return "LIMIT"
            self.warn(f"Beta app review did not take the build ({error}). Testers in the external group "
                      "get it once a build of this version is approved; submit it in App Store Connect "
                      "if this is not expected.")
            return "REFUSED"
        log("submitted for beta app review")
        return "SUBMITTED"

    def finish(self, bundle_id, version, number, notes, groups, timeout=5400, interval=30):
        app = self.app_id(bundle_id)
        resolved, errors = self.resolve_groups(app, groups)
        build = self.wait_processed(app, version, number, timeout, interval)
        self.report_encryption(build)
        self.set_what_to_test(build["id"], notes)
        for group in resolved:
            self.add_to_group(group, build["id"])
        if any(not g["attributes"].get("isInternalGroup") for g in resolved):
            last = self.recent_review(app)
            if last is None:
                self.submit_for_review(build["id"])
            else:
                hours = max(1, round((self.clock() - last) / 3600))
                self.notice(f"Not submitted for beta app review: a build went to review {hours} h ago, and Apple "
                            f"takes only a few a day. The first release after {REVIEW_SPACING_HOURS} h goes; "
                            "internal testers have this one now.")
        return errors


    def app_store_version(self, app_id, version):
        """The app's iOS App Store version `version`, or None."""
        found = self.pages(f"apps/{app_id}/appStoreVersions?" + urllib.parse.urlencode(
            {"filter[versionString]": version, "filter[platform]": "IOS", "limit": 10,
             "fields[appStoreVersions]": "versionString,platform,appVersionState,appStoreState,releaseType"}))
        found = [v for v in found if (v.get("attributes") or {}).get("versionString") == version]
        return found[0] if found else None

    def review_submissions(self, app_id, states):
        return self.pages("reviewSubmissions?" + urllib.parse.urlencode(
            {"filter[app]": app_id, "filter[platform]": "IOS", "filter[state]": ",".join(states), "limit": 50}))

    def submit(self, bundle_id, version, number, release_type, code, dry_run=False):
        """Sends build `number` of `version` to App Store review. Returns what it did, for the log."""
        app = self.app_id(bundle_id)
        found = self.builds(app, version, number)
        if not found:
            raise AscError(404, f"no build {version} ({number}) in App Store Connect: upload it to TestFlight first")
        build = found[0]
        state = build["attributes"].get("processingState")
        if state != "VALID":
            raise AscError(0, f"{version} ({number}) is {state}, not processed (VALID): wait, then run this again")
        self.check_encryption(app, build, code)
        blocking = self.review_submissions(app, OPEN_REVIEW_STATES)
        if blocking:
            states = ", ".join(sorted({(r.get("attributes") or {}).get("state", "?") for r in blocking}))
            raise AscError(409, f"the app has a review submission already ({states}): wait for it, or resolve or "
                                "cancel it in App Store Connect")
        record = self.app_store_version(app, version)
        if record is not None and version_state(record) not in EDITABLE_VERSION_STATES:
            raise AscError(409, f"App Store version {version} is {version_state(record)}: it takes no new build or "
                                "submission (a newer release needs a new version)")
        if dry_run:
            what = "create" if record is None else f"use ({version_state(record)})"
            log(f"dry run: would {what} App Store version {version} ({release_type}), attach build {number} and "
                "submit it for review; nothing changed")
            return "DRY_RUN"
        if record is None:
            try:
                record = self.call("POST", "appStoreVersions",
                                   app_store_version_create_body(app, version, release_type))["data"]
            except AscError as error:
                if not error.refused:
                    raise
                raise AscError(error.status, f"App Store Connect did not create version {version} ({error.detail}); "
                                             "another version in Prepare for Submission is the usual cause: rename "
                                             "or remove it in App Store Connect", error.code) from None
            log(f"created App Store version {version} ({release_type})")
        elif (record.get("attributes") or {}).get("releaseType") != release_type:
            self.call("PATCH", f"appStoreVersions/{record['id']}",
                      app_store_version_update_body(record["id"], release_type))
            log(f"App Store version {version}: release type {release_type}")
        attached = (self.call("GET", f"appStoreVersions/{record['id']}/build").get("data") or {}).get("id")
        if attached != build["id"]:
            self.call("PATCH", f"appStoreVersions/{record['id']}/relationships/build", version_build_body(build["id"]))
            log(f"App Store version {version}: build {number} attached")
        drafts = self.review_submissions(app, ("READY_FOR_REVIEW",))
        if drafts:
            submission = drafts[0]["id"]
            items = self.pages(f"reviewSubmissions/{submission}/items?limit=50&include=appStoreVersion")
            has_version = record["id"] in {item_version(i) for i in items}
        else:
            submission = self.call("POST", "reviewSubmissions", review_create_body(app))["data"]["id"]
            has_version = False
        if not has_version:
            self.call("POST", "reviewSubmissionItems", review_item_body(submission, record["id"]))
        self.call("PATCH", f"reviewSubmissions/{submission}", review_submit_body(submission))
        log(f"{version} ({number}) submitted for App Store review ({release_type} release)")
        return "SUBMITTED"


def load_key(args, environ):
    key_id = environ.get("AC_API_KEY_ID", "")
    issuer = environ.get("AC_API_ISSUER_ID", "")
    if not (args.key_file and key_id and issuer):
        raise ValueError("an App Store Connect API key is needed: --key-file, and AC_API_KEY_ID and "
                         "AC_API_ISSUER_ID in the environment")
    key = Path(args.key_file).read_text()
    if "PRIVATE KEY" not in key:
        raise ValueError(f"{args.key_file} is not a .p8 private key")
    return key, key_id, issuer


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("prepare", "finish", "submit"):
        command = commands.add_parser(name)
        command.add_argument("--bundle-id", default=BUNDLE_ID)
        command.add_argument("--version", required=True, help="the marketing version (CFBundleShortVersionString)")
        command.add_argument("--key-file", help="the App Store Connect API key (.p8)")
    prepare = commands.choices["prepare"]
    prepare.add_argument("--minimum", type=int, required=True, help="the lowest build number to use")
    finish = commands.choices["finish"]
    finish.add_argument("--build-number", required=True)
    finish.add_argument("--notes-file", help="the release notes (Markdown); empty or missing: the version only")
    finish.add_argument("--release-url", help="named at the end of What to Test")
    finish.add_argument("--group", action="append", default=[], help="a TestFlight group id or name (repeatable)")
    finish.add_argument("--timeout", type=int, default=5400, help="seconds to wait for processing")
    finish.add_argument("--interval", type=int, default=30)
    submit = commands.choices["submit"]
    submit.add_argument("--build-number", required=True)
    submit.add_argument("--release-type", choices=RELEASE_TYPES, default="MANUAL",
                        help="MANUAL: released by hand after approval (default); AFTER_APPROVAL: at once")
    submit.add_argument("--dry-run", action="store_true", help="read and report only")
    profile = commands.add_parser("profile")
    profile.add_argument("--plist", required=True, help="the profile as `security cms -D -i` decodes it")
    profile.add_argument("--bundle-id", required=True)
    profile.add_argument("--team", required=True)
    secrets = commands.add_parser("secrets")
    secrets.add_argument("--need", required=True, type=lambda text: [k for k in text.split(",") if k],
                         help="cert, profiles or asc (comma-separated)")
    secrets.add_argument("--out", required=True)
    packages = commands.add_parser("packages")
    packages.add_argument("--resolved", required=True)
    packages.add_argument("--checkouts", required=True, help="the cloned source packages' checkouts folder")
    args = parser.parse_args(argv)
    if args.command == "secrets" and (not args.need or set(args.need) - set(SECRETS)):
        parser.error(f"--need takes {', '.join(SECRETS)}")
    return args


def main(argv=None, http=None, environ=os.environ):
    args = parse_args(argv)
    try:
        if args.command == "profile":
            with open(args.plist, "rb") as handle:
                uuid, name = check_profile(plistlib.load(handle), args.bundle_id, args.team)
            log(f"{args.bundle_id}: {name} ({uuid})")
            print(uuid)
            return 0
        if args.command == "secrets":
            written = materialize(args.need, args.out, environ, mask=lambda line: print(line, flush=True))
            log(f"wrote {', '.join(written)}")
            return 0
        if args.command == "packages":
            for line in audit_packages(args.resolved, args.checkouts):
                log(line)
            return 0
        if args.command == "submit":
            # Before the key is read or anything is requested, --dry-run included.
            code = export_compliance_code(environ)
        asc = Asc(http or Http(), *load_key(args, environ), environ=environ)
        if args.command == "prepare":
            number, uploaded = asc.prepare(args.bundle_id, args.version, args.minimum)
            print(f"build-number={number}")
            print(f"uploaded={str(uploaded).lower()}")
            return 0
        if args.command == "submit":
            if not re.fullmatch(r"[0-9]+(\.[0-9]+){0,2}", args.version) or not re.fullmatch(r"[0-9]+(\.[0-9]+)*",
                                                                                          args.build_number):
                raise ValueError(f"{args.version!r} ({args.build_number!r}) is not a version and build number")
            result = asc.submit(args.bundle_id, args.version, args.build_number, args.release_type, code,
                                args.dry_run)
            print(f"result={result}")
            return 0
        notes = Path(args.notes_file).read_text() if args.notes_file and Path(args.notes_file).is_file() else ""
        text = what_to_test(notes, args.version, args.release_url)
        groups = list(dict.fromkeys(g.strip() for spec in args.group for g in spec.split(",") if g.strip()))
        errors = asc.finish(args.bundle_id, args.version, args.build_number, text, groups, args.timeout,
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
