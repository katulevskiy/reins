#!/usr/bin/env python3
"""Exercise scripts/play-upload.py against a fake Google Play (no network), and check the store listing's limits."""

import base64
import contextlib
import http.server
import importlib.util
import io
import json
import struct
import subprocess
import sys
import tempfile
import threading
import zipfile
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("play_upload", ROOT / "scripts/play-upload.py")
play = importlib.util.module_from_spec(spec)
spec.loader.exec_module(play)

checks = 0


def check(condition, message):
    global checks
    checks += 1
    assert condition, message


class FakePlay:
    """Google's token endpoint and the androidpublisher API, as far as the uploader uses them."""

    def __init__(self, fail_on=None, error=(400, "error"), version_code=29858583):
        self.calls = []
        self.fail_on = fail_on
        self.error = error
        self.version_code = version_code

    def request(self, method, url, headers=None, body=None, timeout=120):
        if hasattr(body, "read"):
            body = body.read()
        self.calls.append((method, url, headers or {}, body))
        if self.fail_on and self.fail_on(method, url):
            raise play.PlayError(*self.error)
        if url == play.TOKEN_URI:
            return {"access_token": "ya29.secret-token", "expires_in": 3599}
        if method == "POST" and url.endswith("/edits"):
            return {"id": "edit-1"}
        if "/bundles?" in url:
            return {"versionCode": self.version_code, "sha256": "ab" * 32}
        return {}

    def routes(self):
        return [(method, url.split("/applications/")[-1]) for method, url, _, _ in self.calls[1:]]


def run(argv, http=None, environ=None):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            code = play.main(argv, http=http, environ=environ or {})
        except SystemExit as exit_:
            code = exit_.code
    return code, out.getvalue(), err.getvalue()


def b64decode(part):
    return base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))


with tempfile.TemporaryDirectory(prefix="reins-play-upload-") as directory:
    root = Path(directory)
    key = root / "key.pem"
    subprocess.run(
        ["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", str(key)],
        check=True,
        capture_output=True,
    )
    private_key = key.read_text()
    account = {
        "type": "service_account",
        "client_email": "reins-release@reins-play.iam.gserviceaccount.com",
        "private_key_id": "0123abcd",
        "private_key": private_key,
        "token_uri": play.TOKEN_URI,
    }
    environ = {"PLAY_SERVICE_ACCOUNT_JSON": json.dumps(account)}

    # The JWT: RS256 over the header and claims Google expects, verifiable with the key's public half.
    jwt = play.assertion(account, now=1_800_000_000)
    header, claims, signature = jwt.split(".")
    check(json.loads(b64decode(header)) == {"alg": "RS256", "typ": "JWT", "kid": "0123abcd"}, "JWT header")
    claims = json.loads(b64decode(claims))
    check(claims["iss"] == account["client_email"] and claims["aud"] == play.TOKEN_URI, "JWT issuer, audience")
    check(claims["scope"] == play.SCOPE and claims["exp"] - claims["iat"] == 3600, "JWT scope, lifetime")
    (root / "signature").write_bytes(b64decode(signature))
    public = subprocess.run(["openssl", "pkey", "-in", str(key), "-pubout"], check=True, capture_output=True).stdout
    (root / "public.pem").write_bytes(public)
    verified = subprocess.run(
        ["openssl", "dgst", "-sha256", "-verify", str(root / "public.pem"), "-signature", str(root / "signature")],
        input=f"{header}.{jwt.split('.')[1]}".encode(),
        capture_output=True,
        check=False,
    )
    check(verified.returncode == 0, "the JWT signature verifies with the public key")

    # Credentials that are not a service account key are refused without echoing them.
    for raw in ("not json {private", json.dumps({"type": "authorized_user", "private_key": "x"})):
        try:
            play.load_credentials(environ={"PLAY_SERVICE_ACCOUNT_JSON": raw})
        except ValueError as error:
            check("private" not in str(error).replace("private_key", ""), "the key text is not echoed")
        else:
            raise AssertionError("accepted invalid credentials")
    check(play.load_credentials(environ={"PLAY_SERVICE_ACCOUNT_JSON": "  "}) is None, "empty means unconfigured")

    bundle = root / "reins-0.3.0-play.aab"
    with zipfile.ZipFile(bundle, "w") as archive:
        for name in ("BundleConfig.pb", "base/manifest/AndroidManifest.xml", "META-INF/UPLOAD.RSA"):
            archive.writestr(name, b"x")
    notes = root / "notes"
    notes.mkdir()
    (notes / "en-US.txt").write_text("Reins {version}: fixes and improvements.\n")
    base = ["--bundle", str(bundle), "--version", "0.3.0", "--notes-dir", str(notes)]

    # A completed release on the internal track: token, edit, upload, track, commit, in that order.
    fake = FakePlay()
    code, out, err = run(base + ["--version-code", "29858583"], fake, environ)
    check(code == 0, f"upload succeeds: {err}")
    check(
        fake.routes()
        == [
            ("POST", "com.reins2fa.app/edits"),
            ("POST", "com.reins2fa.app/edits/edit-1/bundles?uploadType=media"),
            ("PUT", "com.reins2fa.app/edits/edit-1/tracks/internal"),
            ("POST", "com.reins2fa.app/edits/edit-1:commit"),
        ],
        f"request sequence: {fake.routes()}",
    )
    token_call = fake.calls[0]
    check(b"grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer" in token_call[3], "JWT bearer grant")
    upload = fake.calls[2]
    check(upload[1].startswith(play.UPLOAD_API), "the bundle goes to the upload endpoint")
    check(upload[2]["Content-Type"] == "application/octet-stream", "upload content type")
    check(upload[2]["Content-Length"] == str(bundle.stat().st_size) and upload[3] == bundle.read_bytes(), "upload body")
    check(all(c[2].get("Authorization") == "Bearer ya29.secret-token" for c in fake.calls[1:]), "bearer token")
    track = json.loads(fake.calls[3][3])
    check(
        track
        == {
            "track": "internal",
            "releases": [
                {
                    "name": "0.3.0",
                    "versionCodes": ["29858583"],
                    "status": "completed",
                    "releaseNotes": [{"language": "en-US", "text": "Reins 0.3.0: fixes and improvements."}],
                }
            ],
        },
        f"track body: {track}",
    )
    for secret in ("ya29.secret-token", "PRIVATE KEY", private_key.splitlines()[1]):
        check(secret not in out + err, "no secret in the output")

    # The first uploads: a draft on another track; production rollouts need a fraction.
    fake = FakePlay()
    code, _, _ = run(base + ["--track", "beta", "--status", "draft"], fake, environ)
    track = json.loads(fake.calls[3][3])
    check(code == 0 and track["track"] == "beta" and track["releases"][0]["status"] == "draft", "draft on beta")
    fake = FakePlay()
    code, _, _ = run(base + ["--track", "production", "--status", "inProgress", "--user-fraction", "0.1"], fake, environ)
    check(code == 0 and json.loads(fake.calls[3][3])["releases"][0]["userFraction"] == 0.1, "staged rollout")
    for bad in (["--status", "inProgress"], ["--user-fraction", "0.5"], ["--track", "nightly"]):
        code, _, _ = run(base + bad, FakePlay(), environ)
        check(code == 2, f"refused {bad}")

    # Several tracks in one edit; a track Google Play does not open to the app yet is skipped with a warning.
    closed = "Precondition check failed."
    fake = FakePlay(
        fail_on=lambda method, url: method == "PUT" and url.endswith(("/beta", "/production")), error=(400, closed)
    )
    code, out, err = run(base + ["--track", "internal,alpha,beta,production"], fake, environ)
    check(code == 0, f"closed tracks are skipped: {err}")
    puts = [url.rsplit("/", 1)[-1] for method, url in fake.routes() if method == "PUT"]
    check(puts == ["internal", "alpha", "beta", "production"], f"every track tried: {puts}")
    check(fake.routes()[-1] == ("POST", "com.reins2fa.app/edits/edit-1:commit"), "the open tracks are committed")
    check("beta track yet" in err and "production track yet" in err, f"skipped tracks are reported: {err}")
    check("internal, alpha (completed)" in out, f"the committed tracks are named: {out}")
    code, _, err = run(base + ["--track", "beta"], FakePlay(fail_on=lambda m, u: m == "PUT", error=(400, closed)), environ)
    check(code == 1 and "no track took the release" in err, "a release no track takes fails")
    code, _, err = run(base + ["--track", "beta"], FakePlay(fail_on=lambda m, u: m == "PUT", error=(400, closed)),
                       dict(environ, GITHUB_ACTIONS="true"))
    check("::warning::Google Play does not take releases on the beta track" in err, "a warning annotation on CI")
    denied = (403, "The caller does not have permission")
    fake = FakePlay(fail_on=lambda method, url: method == "PUT" and url.endswith("/production"), error=denied)
    code, _, err = run(base + ["--track", "alpha,production"], fake, environ)
    check(code == 0 and "permission to release there" in err, f"a track the account may not release to: {err}")
    code, _, _ = run(base, FakePlay(fail_on=lambda m, u: u.endswith("/edits"), error=denied), environ)
    check(code == 1, "no access to the app at all still fails")
    fake = FakePlay(fail_on=lambda method, url: method == "PUT" and url.endswith("/alpha"), error=(400, "Bad notes."))
    code, _, _ = run(base + ["--track", "internal,alpha"], fake, environ)
    check(code == 1 and fake.routes()[-1][0] == "DELETE", "any other refusal still fails the whole edit")
    for bad in ("internal,nightly", ",", ""):
        check(run(base + ["--track", bad], FakePlay(), environ)[0] == 2, f"refused --track {bad!r}")
    code, _, _ = run(base + ["--track", "alpha, internal,alpha"], fake := FakePlay(), environ)
    check([u.rsplit("/", 1)[-1] for m, u in fake.routes() if m == "PUT"] == ["alpha", "internal"], "each track once")

    # The app is still a draft: exit 3, the explanation, and the edit is deleted rather than committed.
    message = "Only releases with status draft may be created on draft app."
    fake = FakePlay(fail_on=lambda method, url: method == "PUT", error=(400, message))
    code, _, err = run(base, fake, environ)
    check(code == play.DRAFT_APP, "draft app exit code")
    check("--status draft" in err and "PLAY_RELEASE_STATUS" in err, "draft app explanation")
    check(fake.routes()[-1] == ("DELETE", "com.reins2fa.app/edits/edit-1"), "the failed edit is deleted")
    check(not any(url.endswith(":commit") for _, url in fake.routes()), "nothing committed")

    # Before the first manual upload Google answers 404: explained, and there is no edit to delete.
    fake = FakePlay(fail_on=lambda method, url: url.endswith("/edits"), error=(404, "Package not found: com.reins2fa.app."))
    code, _, err = run(base, fake, environ)
    check(code == 1 and "upload the first bundle there by hand" in err, "package not found explanation")
    check(not any(method == "DELETE" for method, _ in fake.routes()), "no edit to delete")

    # Managed publishing wants changesNotSentForReview; the flag sends it.
    fake = FakePlay()
    code, _, _ = run(base + ["--changes-not-sent-for-review"], fake, environ)
    check(code == 0 and fake.routes()[-1][1].endswith(":commit?changesNotSentForReview=true"), "not sent for review")
    review = "Changes cannot be sent for review automatically. Please set the query parameter changesNotSentForReview."
    fake = FakePlay(fail_on=lambda method, url: url.endswith(":commit"), error=(400, review))
    code, _, err = run(base, fake, environ)
    check(code == 1 and "--changes-not-sent-for-review" in err, "review explanation")

    # Google Play read another versionCode from the bundle than the build had.
    fake = FakePlay(version_code=1)
    code, _, err = run(base + ["--version-code", "29858583"], fake, environ)
    check(code == 1 and fake.routes()[-1][0] == "DELETE" and "versionCode 1" in err, "versionCode mismatch")

    # Dry run: no request at all, the planned release, and no secret.
    class Offline:
        def request(self, *args, **kwargs):
            raise AssertionError("the dry run used the network")

    code, out, err = run(base + ["--dry-run", "--track", "alpha"], Offline(), environ)
    check(code == 0 and '"track": "alpha"' in out and account["client_email"] in out, f"dry run: {err}")
    check("PRIVATE KEY" not in out + err and "eyJ" not in out + err, "the dry run shows no key or JWT")
    code, out, _ = run(base + ["--dry-run", "--track", "internal,alpha"], Offline(), environ)
    check(code == 0 and '"track": "internal"' in out and '"track": "alpha"' in out, "dry run of two tracks")
    code, out, _ = run(base + ["--dry-run"], Offline(), {})
    check(code == 0 and "none (PLAY_SERVICE_ACCOUNT_JSON is empty)" in out, "dry run without credentials")
    broken = dict(account, private_key="-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n")
    code, _, err = run(base + ["--dry-run"], Offline(), {"PLAY_SERVICE_ACCOUNT_JSON": json.dumps(broken)})
    check(code == 2 and "could not sign" in err, "the dry run proves the key signs")

    # Inputs it refuses before any request.
    code, _, err = run(base, Offline(), {})
    check(code == 2 and "no credentials" in err, "credentials are required to upload")
    (notes / "de-DE.txt").write_text("x" * 501)
    code, _, err = run(base, Offline(), environ)
    check(code == 2 and "500" in err, "release notes over 500 characters")
    (notes / "de-DE.txt").unlink()
    for files in ({"BundleConfig.pb": b"x"}, {"BundleConfig.pb": b"x", "base/manifest/AndroidManifest.xml": b"x"}):
        with zipfile.ZipFile(bundle, "w") as archive:
            for name, contents in files.items():
                archive.writestr(name, contents)
        code, _, _ = run(base, Offline(), environ)
        check(code == 2, "not a signed bundle")
    bundle.write_bytes(b"PK not really")
    check(run(base, Offline(), environ)[0] == 2, "not a zip")

    # The real HTTP client against a loopback stand-in for Google: the bundle is streamed with its length, errors are
    # read from Google's JSON.
    received = []

    class Google(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def answer(self, status, body):
            data = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def handle_any(self):
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length)
            received.append((self.command, self.path, self.headers.get("Content-Type"), len(body)))
            if self.path == "/token":
                self.answer(200, {"access_token": "loopback-token"})
            elif self.path.endswith("/edits"):
                self.answer(200, {"id": "e1"})
            elif "/bundles" in self.path:
                self.answer(200, {"versionCode": 42, "sha256": "00"})
            elif "/tracks/" in self.path and json.loads(body)["releases"][0]["status"] != "draft":
                self.answer(400, {"error": {"code": 400, "message": message, "status": "INVALID_ARGUMENT"}})
            else:
                self.answer(200, {})

        do_GET = do_POST = do_PUT = do_DELETE = handle_any

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Google)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    loopback = f"http://127.0.0.1:{server.server_port}"
    real = (play.API, play.UPLOAD_API)
    play.API, play.UPLOAD_API = f"{loopback}/api", f"{loopback}/upload"
    local = {"PLAY_SERVICE_ACCOUNT_JSON": json.dumps(dict(account, token_uri=f"{loopback}/token"))}
    with zipfile.ZipFile(bundle, "w") as archive:
        for name in ("BundleConfig.pb", "base/manifest/AndroidManifest.xml", "META-INF/UPLOAD.RSA"):
            archive.writestr(name, b"x" * 70_000)
    try:
        code, _, err = run(base, play.Http(), local)
        check(code == play.DRAFT_APP and message in err, f"loopback: Google's error message reaches the user: {err}")
        check(received[-1][0] == "DELETE", "loopback: the refused edit is deleted")
        received.clear()
        code, out, err = run(base + ["--status", "draft"], play.Http(), local)
        check(code == 0, f"loopback upload: {err}")
        upload = next(r for r in received if "/bundles" in r[1])
        check(upload[2] == "application/octet-stream" and upload[3] == bundle.stat().st_size, "loopback: streamed")
        check(received[-1][:2] == ("POST", "/api/com.reins2fa.app/edits/e1:commit"), "loopback: committed")
        check("loopback-token" not in out + err, "loopback: the token is not shown")
    finally:
        play.API, play.UPLOAD_API = real
        server.shutdown()

    # Google's two error shapes.
    check(play.error_message('{"error": {"code": 403, "message": "nope"}}') == "nope", "API error message")
    check(play.error_message('{"error": "invalid_grant", "error_description": "bad"}') == "invalid_grant bad", "token")

# The store listing in android/play: Google Play's limits.
listing = ROOT / "android/play"
limits = {"title.txt": 30, "short-description.txt": 80, "full-description.txt": 4000}
for language in sorted(p for p in (listing / "listing").iterdir() if p.is_dir()):
    for name, limit in limits.items():
        text = (language / name).read_text(encoding="utf-8").strip()
        check(0 < len(text) <= limit, f"{language.name}/{name}: {len(text)} characters (limit {limit})")
    check("\n" not in (language / "title.txt").read_text().strip(), "one-line title")
for notes in (listing / "release-notes").glob("*.txt"):
    text = notes.read_text(encoding="utf-8").replace("{version}", "10.10.10").strip()
    check(0 < len(text) <= play.NOTES_LIMIT, f"{notes.name}: {len(text)} characters")
# Text messages and self-updating are only in the APK from reins2fa.com.
for path in (listing / "listing").rglob("*.txt"):
    text = path.read_text(encoding="utf-8").lower()
    for word in ("sms", "text message", "self-update", "apk"):
        check(word not in text, f"{path.relative_to(ROOT)} mentions {word!r}")


def png_size(path):
    data = path.read_bytes()[:24]
    check(data[:8] == b"\x89PNG\r\n\x1a\n", f"{path.name} is a PNG")
    return struct.unpack(">II", data[16:24])


graphics = listing / "graphics"
check(png_size(graphics / "icon.png") == (512, 512), "512 x 512 icon")
check((graphics / "icon.png").stat().st_size <= 1024 * 1024, "icon at most 1 MB")
check(png_size(graphics / "feature-graphic.png") == (1024, 500), "1024 x 500 feature graphic")
screens = sorted((graphics / "phone-screenshots").glob("*.png"))
check(2 <= len(screens) <= 8, f"2 to 8 phone screenshots, not {len(screens)}")
for screen in screens:
    width, height = png_size(screen)
    check(1080 <= min(width, height) and max(width, height) <= 2 * min(width, height) <= 3840 * 2, screen.name)
    check(screen.stat().st_size <= 8 * 1024 * 1024, f"{screen.name} at most 8 MB")

# The release workflow builds, checks, attaches and uploads the bundle.
workflow = (ROOT / ".github/workflows/release.yml").read_text()
for needle in (
    ":app:bundlePlayRelease",
    'reins-$VERSION-play.aab',
    "scripts/play-upload.py",
    "/signing/play",
    "--notes-dir android/play/release-notes",
    "--cert-sha256",
):
    check(needle in workflow, f"release.yml: {needle}")

print(f"{checks} Google Play upload and listing checks passed")
