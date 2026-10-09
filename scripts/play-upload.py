#!/usr/bin/env python3
"""Uploads the Play app bundle to a Google Play track with the Google Play Developer API (androidpublisher v3).

    scripts/play-upload.py --bundle dist/reins-0.3.0-play.aab --version 0.3.0 \
        [--track internal|alpha|beta|production] [--status completed|draft|inProgress] [--user-fraction 0.1] \
        [--notes-dir android/play/release-notes] [--version-code N] [--changes-not-sent-for-review] [--dry-run]

One edit: create it, upload the bundle, set the track's release (the bundle's version code, the release notes,
the status), commit. Anything that fails deletes the edit, so nothing half-done is left in the Play Console.

Credentials: a Google Cloud service account's JSON key, from PLAY_SERVICE_ACCOUNT_JSON (the JSON itself) or
--credentials FILE. The account needs the Play Console permission "Release apps to testing tracks" (and "Release to
production" for --track production) on com.reins2fa.app. The key is used to sign a JWT (RS256, with `openssl`) that is
exchanged for a one-hour access token; neither the key nor the token is ever printed. Needs only Python's standard
library and `openssl`.

Release notes: <notes-dir>/<language>.txt (en-US.txt, de-DE.txt, ...), `{version}` replaced by --version; Play allows
500 characters per language.

--status draft is required while the app itself is a draft in the Play Console (never published on any track): the
API then refuses any other status with "Only releases with status draft may be created on draft app". A draft
release is rolled out from the Play Console by hand. --dry-run checks the bundle, the notes and the key (it signs a
JWT) and prints what would be sent, without network access.

Exit codes: 0 done, 1 failed, 2 bad arguments or files, 3 the app is still a draft (use --status draft).
"""

import argparse
import base64
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

PACKAGE = "com.reins2fa.app"
API = "https://androidpublisher.googleapis.com/androidpublisher/v3/applications"
UPLOAD_API = "https://androidpublisher.googleapis.com/upload/androidpublisher/v3/applications"
SCOPE = "https://www.googleapis.com/auth/androidpublisher"
TOKEN_URI = "https://oauth2.googleapis.com/token"
TRACKS = ("internal", "alpha", "beta", "production")
STATUSES = ("completed", "draft", "inProgress", "halted")
NOTES_LIMIT = 500
DRAFT_APP = 3

# What the Play Console's errors mean for this script, matched on Google's message (lower case).
EXPLANATIONS = (
    ("draft app", (
        "The app is still a draft in the Play Console: it has never been published on any track, so the API only "
        "accepts draft releases. Upload with --status draft (CI: set the repository variable PLAY_RELEASE_STATUS to "
        "draft), then roll the release out from the Play Console (Test and release > Testing > Internal testing). "
        "Once a release of the app has been rolled out, completed releases work. See android/PLAY_STORE.md, "
        "\"First upload\"."
    )),
    ("package not found", (
        f"Google Play does not know {PACKAGE} yet, or this service account cannot see it. Create the app in the "
        "Play Console, upload the first bundle there by hand (Google Play only accepts API uploads for an app that "
        "has one), and invite the service account under Users and permissions with access to the app."
    )),
    ("changesnotsentforreview", (
        "The app's changes must be sent for review from the Play Console (managed publishing, or the app needs a "
        "review). Rerun with --changes-not-sent-for-review, then send the changes for review in the Play Console."
    )),
    ("version code", (
        "This versionCode was used before. Every upload needs a new one (the release workflow uses minutes since "
        "1970): rebuild the bundle."
    )),
    ("caller does not have permission", (
        "The service account has no access to the app. Play Console > Users and permissions: invite its email with "
        "\"Release apps to testing tracks\" (and \"Release to production\" for production), for this app."
    )),
    ("has not been used in project", (
        "The Google Play Android Developer API is not enabled in the service account's Google Cloud project. "
        "Enable it there (APIs and services > Library), wait a few minutes, and retry."
    )),
)


class PlayError(Exception):
    """A failed request: the HTTP status, Google's message and, when there is one, what it means here."""

    def __init__(self, status, message):
        super().__init__(f"HTTP {status}: {message}")
        self.status = status
        self.message = message

    @property
    def draft_app(self):
        return "draft app" in self.message.lower()

    def explanation(self):
        text = self.message.lower()
        return next((why for key, why in EXPLANATIONS if key in text), None)


def fail(message, code=2):
    print(f"play-upload: {message}", file=sys.stderr)
    sys.exit(code)


def b64url(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def load_credentials(path=None, environ=os.environ):
    raw = Path(path).read_text() if path else environ.get("PLAY_SERVICE_ACCOUNT_JSON", "")
    if not raw.strip():
        return None
    try:
        account = json.loads(raw)
    except json.JSONDecodeError:
        # Never echo the text: it is a private key.
        raise ValueError("the service account key is not valid JSON") from None
    missing = [k for k in ("client_email", "private_key") if not account.get(k)]
    if account.get("type") != "service_account" or missing:
        raise ValueError(
            "the credentials are not a service account key (type service_account, client_email, private_key)"
        )
    return account


def sign_rs256(private_key_pem, data):
    """RS256 with `openssl`: the PEM goes to a private temporary file that is removed right after."""
    with tempfile.TemporaryDirectory(prefix="play-upload-") as directory:
        key = Path(directory) / "key.pem"
        key.touch(mode=0o600)
        key.write_text(private_key_pem)
        result = subprocess.run(
            ["openssl", "dgst", "-sha256", "-sign", str(key)],
            input=data,
            capture_output=True,
            check=False,
        )
    if result.returncode != 0 or not result.stdout:
        raise ValueError("openssl could not sign with the service account key (is private_key a PEM RSA key?)")
    return result.stdout


def assertion(account, now=None):
    """The signed JWT that Google's token endpoint exchanges for an access token."""
    now = int(time.time() if now is None else now)
    header = {"alg": "RS256", "typ": "JWT"}
    if account.get("private_key_id"):
        header["kid"] = account["private_key_id"]
    claims = {
        "iss": account["client_email"],
        "scope": SCOPE,
        "aud": account.get("token_uri") or TOKEN_URI,
        "iat": now,
        "exp": now + 3600,
    }
    signing_input = f"{b64url(json.dumps(header).encode())}.{b64url(json.dumps(claims).encode())}"
    return f"{signing_input}.{b64url(sign_rs256(account['private_key'], signing_input.encode()))}"


class Http:
    """The network, behind one method so the tests can replace it."""

    def request(self, method, url, headers=None, body=None, timeout=120):
        request = urllib.request.Request(url, data=body, method=method, headers=headers or {})
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                text = response.read().decode()
        except urllib.error.HTTPError as error:
            raise PlayError(error.code, error_message(error.read().decode(errors="replace"))) from None
        except urllib.error.URLError as error:
            raise PlayError(0, f"{url.split('?')[0]}: {error.reason}") from None
        return json.loads(text) if text.strip() else {}


def error_message(text):
    """Google's error message, without anything else the body may hold."""
    try:
        body = json.loads(text)
    except json.JSONDecodeError:
        return text.strip()[:500] or "no details"
    error = body.get("error")
    if isinstance(error, dict):
        return str(error.get("message") or error.get("status") or "no details")
    # The token endpoint: {"error": "invalid_grant", "error_description": "..."}
    return " ".join(str(body[k]) for k in ("error", "error_description") if body.get(k)) or "no details"


def check_bundle(path):
    """A readable app bundle (not an APK) whose base module has a manifest; returns its size."""
    try:
        with zipfile.ZipFile(path) as archive:
            names = set(archive.namelist())
    except (OSError, zipfile.BadZipFile) as error:
        raise ValueError(f"{path} is not an app bundle: {error}") from None
    for required in ("BundleConfig.pb", "base/manifest/AndroidManifest.xml"):
        if required not in names:
            raise ValueError(f"{path} is not an app bundle: no {required}")
    if not any(name.startswith("META-INF/") and name.endswith((".RSA", ".EC", ".DSA")) for name in names):
        raise ValueError(f"{path} is not signed (Google Play requires the upload key's signature)")
    return Path(path).stat().st_size


def release_notes(directory, version):
    """[{language, text}] from <directory>/<language>.txt, or [] when there is no directory."""
    if directory is None:
        return []
    notes = []
    for path in sorted(Path(directory).glob("*.txt")):
        text = path.read_text(encoding="utf-8").replace("{version}", version).strip()
        if not text:
            continue
        if len(text) > NOTES_LIMIT:
            raise ValueError(f"{path}: {len(text)} characters; Google Play allows {NOTES_LIMIT}")
        notes.append({"language": path.stem, "text": text})
    if not notes:
        raise ValueError(f"no release notes in {directory} (<language>.txt, e.g. en-US.txt)")
    return notes


def release_body(track, version_code, version, status, notes, user_fraction=None):
    release = {"name": version, "versionCodes": [str(version_code)], "status": status}
    if notes:
        release["releaseNotes"] = notes
    if user_fraction is not None:
        release["userFraction"] = user_fraction
    return {"track": track, "releases": [release]}


class Uploader:
    def __init__(self, http, account, package=PACKAGE, log=print):
        self.http = http
        self.account = account
        self.package = package
        self.log = log
        self.token = None

    def authorize(self):
        body = urllib.parse.urlencode(
            {"grant_type": "urn:ietf:params:oauth:grant-type:jwt-bearer", "assertion": assertion(self.account)}
        ).encode()
        response = self.http.request(
            "POST",
            self.account.get("token_uri") or TOKEN_URI,
            {"Content-Type": "application/x-www-form-urlencoded"},
            body,
        )
        self.token = response.get("access_token")
        if not self.token:
            raise PlayError(0, "the token endpoint returned no access token")

    def call(self, method, path, body=None, upload=None, timeout=120):
        headers = {"Authorization": f"Bearer {self.token}"}
        data = None
        if upload is not None:
            headers |= {"Content-Type": "application/octet-stream", "Content-Length": str(upload[1])}
            data = upload[0]
        elif body is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(body).encode()
        base = UPLOAD_API if upload is not None else API
        return self.http.request(method, f"{base}/{self.package}/{path}", headers, data, timeout)

    def publish(self, bundle, track, status, version, notes, version_code=None, user_fraction=None,
                changes_not_sent_for_review=False):
        self.authorize()
        edit = self.call("POST", "edits", {})["id"]
        self.log(f"Edit {edit} opened for {self.package}")
        try:
            size = Path(bundle).stat().st_size
            self.log(f"Uploading {Path(bundle).name} ({size / 1e6:.1f} MB)...")
            with open(bundle, "rb") as data:
                # Google processes the bundle before it answers; large bundles take minutes.
                uploaded = self.call("POST", f"edits/{edit}/bundles?uploadType=media", upload=(data, size), timeout=1800)
            code = int(uploaded["versionCode"])
            if version_code is not None and code != version_code:
                raise PlayError(0, f"Google Play read versionCode {code} from the bundle, not {version_code}")
            self.log(f"Uploaded versionCode {code} (SHA-256 {uploaded.get('sha256', '?')})")
            self.call("PUT", f"edits/{edit}/tracks/{track}",
                      release_body(track, code, version, status, notes, user_fraction))
            self.log(f"Track {track}: release {version} ({code}), status {status}")
            query = "?changesNotSentForReview=true" if changes_not_sent_for_review else ""
            self.call("POST", f"edits/{edit}:commit{query}")
        except BaseException:
            try:
                self.call("DELETE", f"edits/{edit}")
            except PlayError:
                pass  # an uncommitted edit expires by itself
            raise
        self.log(f"Committed: {version} ({code}) is on the {track} track ({status})")
        return code


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bundle", required=True, help="the signed .aab")
    parser.add_argument("--version", required=True, help="the release name, and {version} in the notes")
    parser.add_argument("--track", default="internal", choices=TRACKS)
    parser.add_argument("--status", default="completed", choices=STATUSES)
    parser.add_argument("--user-fraction", type=float, help="with --status inProgress: the share of users (0-1)")
    parser.add_argument("--notes-dir", help="release notes, <language>.txt")
    parser.add_argument("--version-code", type=int, help="fail unless Google Play reads this versionCode")
    parser.add_argument("--package", default=PACKAGE)
    parser.add_argument("--credentials", help="service account JSON file (default: $PLAY_SERVICE_ACCOUNT_JSON)")
    parser.add_argument("--changes-not-sent-for-review", action="store_true")
    parser.add_argument("--dry-run", action="store_true", help="check everything, send nothing")
    args = parser.parse_args(argv)
    if (args.status == "inProgress") != (args.user_fraction is not None):
        parser.error("--user-fraction goes with --status inProgress, and only with it")
    if args.user_fraction is not None and not 0 < args.user_fraction < 1:
        parser.error("--user-fraction must be between 0 and 1 (exclusive)")
    return args


def main(argv=None, http=None, environ=os.environ):
    args = parse_args(argv)
    try:
        size = check_bundle(args.bundle)
        notes = release_notes(args.notes_dir, args.version)
        account = load_credentials(args.credentials, environ)
        if args.dry_run and account:
            assertion(account)  # proves the key signs; the JWT itself is not shown
    except (OSError, ValueError) as error:
        fail(str(error))
    if args.dry_run:
        print(f"Dry run, nothing sent. {args.package}: {Path(args.bundle).name} ({size / 1e6:.1f} MB)")
        print(f"  credentials: {account['client_email'] if account else 'none (PLAY_SERVICE_ACCOUNT_JSON is empty)'}")
        print(f"  POST edits; POST edits/<id>/bundles; PUT edits/<id>/tracks/{args.track}; POST edits/<id>:commit")
        body = release_body(args.track, args.version_code or "<from the bundle>", args.version, args.status, notes,
                            args.user_fraction)
        print(json.dumps(body, indent=2, ensure_ascii=False))
        return 0
    if account is None:
        fail("no credentials: set PLAY_SERVICE_ACCOUNT_JSON or pass --credentials")
    uploader = Uploader(http or Http(), account, args.package)
    print(f"Google Play: {args.package}, {args.track} track, status {args.status}")
    try:
        uploader.publish(args.bundle, args.track, args.status, args.version, notes, args.version_code,
                         args.user_fraction, args.changes_not_sent_for_review)
    except PlayError as error:
        print(f"play-upload: Google Play refused the upload: {error}", file=sys.stderr)
        if why := error.explanation():
            print(f"play-upload: {why}", file=sys.stderr)
        return DRAFT_APP if error.draft_app else 1
    except ValueError as error:  # the key could not sign
        fail(str(error))
    return 0


if __name__ == "__main__":
    sys.exit(main())
