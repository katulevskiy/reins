#!/usr/bin/env python3
"""The Linux packages, their signed APT and RPM repositories, the update feed that carries them, the AUR package and the
release workflow's signing step, from a fake release signed with throwaway keys.

Runs scripts/package/linux-packages.sh (nfpm), linux-repos.sh, aur.sh and scripts/release-feed.sh --packages, checks
what they make and that they refuse what they must (a key other than the pinned one, a tampered repository, a package
that is not the release asset, a release without the build). Needs nfpm (NFPM=/path/to/nfpm, else on the PATH), gpg,
gpgv, apt-ftparchive, createrepo_c, rpm, rpmsign, rpmkeys, dpkg-deb, cc, strip, openssl, jq, xxd, unzip and perl.
Where they exist, also:
  apt-get    `apt-get update` and downloads from the repository over HTTP, as amd64 and arm64, and refusals of a
             tampered index and a foreign key (APT_PREFIX: where an apt unpacked elsewhere than / lives)
  lintian    the .deb packages with the shipped overrides (`--fail-on error`)
  makepkg    .SRCINFO as makepkg prints it (not as root)
  docker     installs from the repository in containers (scripts/package/test-linux-repos.sh; REINS_TEST_IMAGES,
             default "debian:12 fedora:44", "none" to skip; DOCKER=podman)
"""

import base64
import functools
import gzip
import hashlib
import http.server
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import threading
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parent.parent
VERSION = "9.8.7"
BUILD = f"{VERSION}-202610090000-abcdef12"
TIME = 1791504000
PASSPHRASE = "correct horse battery staple"
DEBS = [f"reins_{VERSION}_amd64.deb", f"reins_{VERSION}_arm64.deb", f"reins-app_{VERSION}_amd64.deb"]
RPMS = [f"reins-{VERSION}-1.x86_64.rpm", f"reins-{VERSION}-1.aarch64.rpm", f"reins-app-{VERSION}-1.x86_64.rpm"]
checks = 0
skipped = []


def check(condition, message):
    global checks
    checks += 1
    if not condition:
        raise AssertionError(message)


def run(*args, ok=True, **kwargs):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True, **kwargs)
    if ok and result.returncode != 0:
        raise AssertionError(f"{' '.join(map(str, args))} failed ({result.returncode}):\n{result.stdout}\n{result.stderr}")
    return result


def refuses(why, *args, expect, **kwargs):
    result = run(*args, ok=False, **kwargs)
    output = result.stdout + result.stderr
    check(result.returncode != 0, f"{why}: accepted\n{output}")
    check(expect in output, f"{why}: expected {expect!r} in\n{output}")


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def tool(name):
    return shutil.which(name)


nfpm = os.environ.get("NFPM") or tool("nfpm")
needed = ["gpg", "gpgv", "apt-ftparchive", "createrepo_c", "rpm", "rpmsign", "rpmkeys", "dpkg-deb", "cc", "strip",
          "openssl", "jq", "xxd", "unzip", "perl"]
missing = [n for n in needed if not tool(n)] + ([] if nfpm and Path(nfpm).exists() else ["nfpm"])
if missing:
    sys.exit(f"test-linux-packages: missing {', '.join(missing)}")

# The nfpm the release uses is the one CI tests with.
release_yml = (ROOT / ".github/workflows/release.yml").read_text()
ci_yml = (ROOT / ".github/workflows/ci.yml").read_text()
for name in ("NFPM_VERSION", "NFPM_SHA256"):
    pins = {re.search(rf"^ +{name}: (\S+)", text, re.M).group(1) for text in (release_yml, ci_yml)}
    check(len(pins) == 1, f"{name} differs between release.yml and ci.yml: {pins}")

work = Path(tempfile.mkdtemp(prefix="reins-linux-packages-"))
try:
    # ── a fake release: the CLI archives and the app's tarball, with programs that print this build ─────────────────
    dist = work / "dist"
    dist.mkdir()
    source = work / "fake.c"
    source.write_text('#include <stdio.h>\nint main(void) { puts(NAME " %s (%s)"); return 0; }\n' % (VERSION, BUILD))
    # reins is static (as the musl build is); reins-app links glibc, as the real one does.
    run("cc", "-static", "-O2", '-DNAME="reins"', "-o", work / "reins", source)
    run("cc", "-O2", '-DNAME="Reins"', "-o", work / "reins-app", source)
    for arch in ("x86_64", "aarch64"):
        name = f"reins-desktop-{VERSION}-{arch}-unknown-linux-musl"
        folder = work / "cli" / name
        folder.mkdir(parents=True)
        shutil.copy(work / "reins", folder / "reins")
        for doc in ("LICENSE", "LICENSE-AGPL", "LICENSING.md", "NOTICE", "README.md"):
            shutil.copy(ROOT / doc, folder / doc)
        run("tar", "-C", work / "cli", "-czf", dist / f"{name}.tar.gz", name)
    app_name = f"Reins-{VERSION}-Linux-x86_64"
    app = work / "app" / app_name
    app.mkdir(parents=True)
    shutil.copy(work / "reins-app", app / "reins-app")
    shutil.copy(work / "reins", app / "reins")
    shutil.copy(ROOT / "crates/reins-desktop-app/packaging/linux/reins.desktop", app / "reins.desktop")
    shutil.copy(ROOT / "crates/reins-desktop-app/assets/icons/app-256.png", app / "reins.png")
    for doc in ("LICENSE", "NOTICE"):
        shutil.copy(ROOT / doc, app / doc)
    shutil.copy(ROOT / "ios/Shared/Fonts/FONTS-NOTICE.txt", app / "FONTS-NOTICE.txt")
    run("tar", "-C", work / "app", "-czf", dist / f"{app_name}.tar.gz", app_name)

    # ── the packages ───────────────────────────────────────────────────────────────────────────────────────────────
    packages = work / "packages"
    environment = os.environ | {"NFPM": nfpm}
    build = [ROOT / "scripts/package/linux-packages.sh", "--version", VERSION, "--build", BUILD, "--time", TIME,
             "--dist", dist]
    run(*build, "--out", packages, env=environment)
    check(sorted(p.name for p in packages.iterdir()) == sorted(DEBS + RPMS), f"packages: {sorted(packages.iterdir())}")
    refuses("another build", *build[:4], f"{VERSION}-202610090000-00000000", *build[5:], "--out", work / "x",
            env=environment, expect="does not carry the build id")
    (dist / f"{app_name}.tar.gz").rename(work / "app.tar.gz")
    refuses("no app", *build, "--out", work / "x", env=environment, expect=f"{app_name}.tar.gz is missing")
    (work / "app.tar.gz").rename(dist / f"{app_name}.tar.gz")

    def deb_field(deb, field):
        return run("dpkg-deb", "-f", packages / deb, field).stdout.strip()

    for deb, package, arch in zip(DEBS, ("reins", "reins", "reins-app"), ("amd64", "arm64", "amd64")):
        check(deb_field(deb, "Package") == package, f"{deb}: Package")
        check(deb_field(deb, "Version") == VERSION, f"{deb}: Version {deb_field(deb, 'Version')}")
        check(deb_field(deb, "Architecture") == arch, f"{deb}: Architecture")
        check(deb_field(deb, "Maintainer") == "Daniil Katulevskiy <support@reins2fa.com>", f"{deb}: Maintainer")
        check(deb_field(deb, "Homepage") == "https://reins2fa.com", f"{deb}: Homepage")
    contents = run("dpkg-deb", "-c", packages / DEBS[0]).stdout
    check(re.search(r"^-rwxr-xr-x root/root .* \./usr/bin/reins$", contents, re.M), f"reins .deb: {contents}")
    for path in ("usr/share/doc/reins/copyright", "usr/share/doc/reins/changelog.gz",
                 "usr/share/lintian/overrides/reins"):
        check(f"./{path}\n" in contents, f"reins .deb lacks {path}")
    check("systemd" not in contents, "reins ships no systemd unit (`reins resume` writes the user's own)")
    contents = run("dpkg-deb", "-c", packages / DEBS[2]).stdout
    for path in ("usr/bin/reins-app", "usr/share/applications/reins.desktop",
                 "usr/share/icons/hicolor/256x256/apps/reins.png", "usr/share/doc/reins-app/copyright"):
        check(f"./{path}\n" in contents, f"reins-app .deb lacks {path}")
    check("./usr/bin/reins\n" not in contents, "reins-app must not carry reins (the reins package does)")
    depends = deb_field(DEBS[2], "Depends")
    for dependency in (f"reins (= {VERSION})", "libc6 (>= 2.35)", "libxkbcommon0", "libvulkan1", "libwayland-client0"):
        check(dependency in depends, f"reins-app Depends lacks {dependency}: {depends}")
    check(deb_field(DEBS[0], "Depends") == "", "reins depends on nothing")
    with tempfile.TemporaryDirectory() as unpacked:
        run("dpkg-deb", "-x", packages / DEBS[0], unpacked)
        out = run(Path(unpacked) / "usr/bin/reins").stdout
        check(out.strip() == f"reins {VERSION} ({BUILD})", f"packaged reins prints {out!r}")
        run("dpkg-deb", "-e", packages / DEBS[0], Path(unpacked) / "control")
        for script in ("postinst", "prerm"):
            run("sh", "-n", Path(unpacked) / "control" / script)
        changelog = gzip.decompress((Path(unpacked) / "usr/share/doc/reins/changelog.gz").read_bytes()).decode()
        check(changelog.startswith(f"reins ({VERSION}) stable; urgency=medium"), changelog)

    rpmdb = work / "rpmdb"
    rpmdb.mkdir()

    def rpm_query(rpm, *query):
        return run("rpm", "--dbpath", rpmdb, "-q", "-p", *query, rpm).stdout

    for rpm, package, arch in zip(RPMS, ("reins", "reins", "reins-app"), ("x86_64", "aarch64", "x86_64")):
        got = rpm_query(packages / rpm, "--qf", "%{NAME} %{VERSION} %{RELEASE} %{ARCH} %{LICENSE} %{URL}")
        license_ = "Apache-2.0 AND OFL-1.1" if package == "reins-app" else "Apache-2.0"
        check(got == f"{package} {VERSION} 1 {arch} {license_} https://reins2fa.com", f"{rpm}: {got}")
    requires = rpm_query(packages / RPMS[2], "--requires")
    for dependency in (f"reins = {VERSION}-1", "libxkbcommon.so.0()(64bit)", "libvulkan.so.1()(64bit)"):
        check(dependency in requires, f"reins-app requires lacks {dependency}: {requires}")
    files = rpm_query(packages / RPMS[0], "--list").split()
    check(files == ["/usr/bin/reins", "/usr/share/doc/reins/NOTICE", "/usr/share/doc/reins/README.md",
                    "/usr/share/licenses/reins/LICENSE"], f"reins .rpm files: {files}")
    check("try-restart reins.service" in rpm_query(packages / RPMS[0], "--scripts"), "reins .rpm %post")
    unsigned = {p.name: sha(p) for p in packages.iterdir()}

    if tool("lintian"):
        # The amd64 ones: the fake arm64 package holds this machine's program (lintian: binary-from-other-architecture).
        result = run("lintian", "--fail-on", "error", packages / DEBS[0], packages / DEBS[2], ok=False)
        check(result.returncode == 0, f"lintian:\n{result.stdout}\n{result.stderr}")
    else:
        skipped.append("lintian (not installed)")

    # ── the repositories, signed with a throwaway key that has a passphrase ──────────────────────────────────────────
    gnupg = work / "gnupg"
    gnupg.mkdir(mode=0o700)
    gpg_env = os.environ | {"GNUPGHOME": str(gnupg)}

    def new_key(home, uid, passphrase=""):
        home.mkdir(mode=0o700, exist_ok=True)
        env = os.environ | {"GNUPGHOME": str(home)}
        run("gpg", "--batch", "--pinentry-mode", "loopback", "--passphrase", passphrase, "--quick-gen-key", uid,
            "rsa3072", "sign", "never", env=env)
        fingerprint = run("gpg", "--batch", "--with-colons", "--list-secret-keys", env=env).stdout
        fingerprint = next(line.split(":")[9] for line in fingerprint.splitlines() if line.startswith("fpr:"))
        public = home / "public.asc"
        public.write_text(run("gpg", "--batch", "--armor", "--export", fingerprint, env=env).stdout)
        return fingerprint, public

    key, public = new_key(gnupg, "Reins packages test <test@example.com>", PASSPHRASE)
    other_key, other_public = new_key(work / "gnupg-other", "Someone else <other@example.com>")
    passphrase_file = work / "passphrase"
    passphrase_file.write_text(PASSPHRASE)
    repo_out = work / "repo"
    repos = [ROOT / "scripts/package/linux-repos.sh", "--packages", packages, "--key", key]
    run(*repos, "--out", repo_out, "--public-key", public, "--passphrase-file", passphrase_file, env=gpg_env)
    refuses("another public key", *repos, "--out", work / "x", "--public-key", other_public, "--passphrase-file",
            passphrase_file, env=gpg_env, expect="is not")
    wrong = work / "wrong-passphrase"
    wrong.write_text("wrong")
    run("gpgconf", "--kill", "gpg-agent", env=gpg_env)  # it would remember the right passphrase
    refuses("a wrong passphrase", *repos, "--out", work / "x", "--public-key", public, "--passphrase-file", wrong,
            env=gpg_env, expect="cannot sign")
    check({p.name: sha(p) for p in packages.iterdir()} == unsigned, "linux-repos.sh changed its input packages")

    repo = repo_out / "packages"
    tree = sorted(str(p.relative_to(repo)) for p in repo.rglob("*") if p.is_file())
    for path in ["reins.gpg", "reins.asc", "apt/dists/stable/InRelease", "apt/dists/stable/Release",
                 "apt/dists/stable/Release.gpg", "apt/dists/stable/main/binary-amd64/Packages",
                 "apt/dists/stable/main/binary-amd64/Packages.gz", "apt/dists/stable/main/binary-arm64/Packages",
                 f"apt/pool/main/r/reins/{DEBS[0]}", f"apt/pool/main/r/reins/{DEBS[1]}",
                 f"apt/pool/main/r/reins-app/{DEBS[2]}", "rpm/repodata/repomd.xml", "rpm/repodata/repomd.xml.asc",
                 f"rpm/x86_64/{RPMS[0]}", f"rpm/aarch64/{RPMS[1]}", f"rpm/x86_64/{RPMS[2]}", "rpm/reins.repo"]:
        check(path in tree, f"the repository lacks {path}: {tree}")
    check((repo / "reins.asc").read_text() == public.read_text(), "reins.asc is the public key given")
    keyring = repo / "reins.gpg"
    for signed in (["apt/dists/stable/InRelease"], ["apt/dists/stable/Release.gpg", "apt/dists/stable/Release"],
                   ["rpm/repodata/repomd.xml.asc", "rpm/repodata/repomd.xml"]):
        run("gpgv", "--keyring", keyring, *(repo / s for s in signed))
    for deb in DEBS:
        check(sha(next(repo.rglob(deb))) == unsigned[deb], f"{deb} in the repository is not the package")
    rpmkeys = ["rpmkeys", "--dbpath", work / "keys-db", "--define", f"_keyringpath {work / 'keys'}"]
    run(*rpmkeys, "--import", repo / "reins.asc")
    for rpm in RPMS:
        signed = next(repo.rglob(rpm))
        verdict = run(*rpmkeys, "--checksig", signed).stdout
        check("signatures OK" in verdict, f"{rpm}: {verdict}")
        check(sha(signed) != unsigned[rpm], f"{rpm} in the repository is not signed")
        verdict = run(*rpmkeys, "--checksig", packages / rpm, ok=False).stdout
        check("signatures" not in verdict, f"the input {rpm} got signed: {verdict}")

    # The APT indexes: each architecture's packages, names, sizes and hashes as in the pool; Release over them.
    release = (repo / "apt/dists/stable/Release").read_text()
    for field, value in (("Suite", "stable"), ("Codename", "stable"), ("Components", "main"),
                         ("Architectures", "amd64 arm64"), ("Origin", "Reins")):
        check(f"\n{field}: {value}\n" in f"\n{release}", f"Release: {field}")
    check("Valid-Until" not in release, "Release must not expire")
    for arch, expected in (("amd64", {"reins", "reins-app"}), ("arm64", {"reins"})):
        index = repo / f"apt/dists/stable/main/binary-{arch}/Packages"
        stanzas = [dict(re.findall(r"^(\S+): (.*)$", s, re.M)) for s in index.read_text().strip().split("\n\n")]
        check({s["Package"] for s in stanzas} == expected, f"{arch} Packages: {stanzas}")
        for stanza in stanzas:
            check(stanza["Architecture"] == arch and stanza["Version"] == VERSION, f"{arch}: {stanza}")
            pool = repo / "apt" / stanza["Filename"]
            check(pool.is_file() and sha(pool) == stanza["SHA256"], f"{stanza['Filename']} does not match")
            check(int(stanza["Size"]) == pool.stat().st_size, f"{stanza['Filename']}: Size")
        for name in ("Packages", "Packages.gz"):
            line = f" {sha(index.parent / name)} {(index.parent / name).stat().st_size:>16} main/binary-{arch}/{name}"
            check(line in release, f"Release lacks {line!r}")
    gz = repo / "apt/dists/stable/main/binary-amd64/Packages.gz"
    check(gzip.decompress(gz.read_bytes()) == gz.with_suffix("").read_bytes(), "Packages.gz is Packages")

    # The RPM metadata: every file repomd.xml names, with its checksum; every package primary.xml names, likewise.
    ns = {"repo": "http://linux.duke.edu/metadata/repo", "common": "http://linux.duke.edu/metadata/common"}
    repomd = ET.parse(repo / "rpm/repodata/repomd.xml").getroot()
    kinds = set()
    for data in repomd.findall("repo:data", ns):
        kinds.add(data.get("type"))
        location = repo / "rpm" / data.find("repo:location", ns).get("href")
        check(sha(location) == data.find("repo:checksum", ns).text, f"{location.name} does not match repomd.xml")
        check(location.suffix == ".gz", f"{location.name}: gzip, which every dnf and zypper reads")
    check({"primary", "filelists", "other"} <= kinds, f"repomd.xml: {kinds}")
    primary_href = repomd.find("repo:data[@type='primary']/repo:location", ns).get("href")
    primary = ET.fromstring(gzip.decompress((repo / "rpm" / primary_href).read_bytes()))
    listed = {}
    for package in primary.findall("common:package", ns):
        location = package.find("common:location", ns).get("href")
        listed[Path(location).name] = package.find("common:arch", ns).text
        check(sha(repo / "rpm" / location) == package.find("common:checksum", ns).text, f"{location} checksum")
    check(listed == {RPMS[0]: "x86_64", RPMS[1]: "aarch64", RPMS[2]: "x86_64"}, f"primary.xml: {listed}")
    definition = (repo / "rpm/reins.repo").read_text().splitlines()
    for line in ("[reins]", "baseurl=https://reins2fa.com/releases/packages/rpm", "gpgcheck=1", "repo_gpgcheck=1",
                 "gpgkey=https://reins2fa.com/releases/packages/reins.asc"):
        check(line in definition, f"reins.repo lacks {line}")

    # ── apt itself against the repository, over HTTP ─────────────────────────────────────────────────────────────────
    serve = work / "serve"
    (serve / "releases").mkdir(parents=True)
    (serve / "releases/packages").symlink_to(repo)
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(serve)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"

    def apt(arch, keyring_file, tag, *command):
        prefix = Path(os.environ.get("APT_PREFIX", "/"))
        root = work / f"apt-{tag}-{arch}"
        for d in ("etc/apt/apt.conf.d", "etc/apt/preferences.d", "etc/apt/sources.list.d", "var/lib/apt/lists/partial",
                  "var/cache/apt/archives/partial", "var/lib/dpkg", "download"):
            (root / d).mkdir(parents=True, exist_ok=True)
        (root / "var/lib/dpkg/status").touch()
        (root / "etc/apt/sources.list.d/reins.sources").write_text(
            f"Types: deb\nURIs: {site}/releases/packages/apt\nSuites: stable\nComponents: main\n"
            f"Signed-By: {keyring_file}\n")
        settings = {
            "Dir": f"{root}/", "Dir::Etc": f"{root}/etc/apt/", "Dir::State": f"{root}/var/lib/apt/",
            "Dir::State::status": f"{root}/var/lib/dpkg/status", "Dir::Cache": f"{root}/var/cache/apt/",
            "Dir::Bin::methods": f"{prefix}/usr/lib/apt/methods/", "APT::Architecture": arch,
            "APT::Sandbox::User": run("id", "-un").stdout.strip(), "Debug::NoLocking": "true",
            "Acquire::Languages": "none",
        }
        for table in ("cputable", "tupletable"):
            if (prefix / "usr/share/dpkg" / table).exists():
                settings[f"Dir::dpkg::{table}"] = f"{prefix}/usr/share/dpkg/{table}"
        if tool("dpkg"):
            settings["Dir::Bin::dpkg"] = tool("dpkg")
        conf = root / "apt.conf"
        conf.write_text("".join(f'{k} "{v}";\n' for k, v in settings.items()) + f'APT::Architectures {{ "{arch}"; }};\n')
        return run(*command, ok=False, cwd=root / "download", env=os.environ | {"APT_CONFIG": str(conf)})

    if tool("apt-get"):
        for arch, names in (("amd64", ["reins", "reins-app"]), ("arm64", ["reins"])):
            update = apt(arch, keyring, "ok", "apt-get", "update")
            output = update.stdout + update.stderr
            check(update.returncode == 0 and not re.search(r"^(W|E):", output, re.M), f"apt-get update:\n{output}")
            policy = apt(arch, keyring, "ok", "apt-cache", "policy", *names).stdout
            check(policy.count(f"Candidate: {VERSION}") == len(names), f"apt-cache policy ({arch}):\n{policy}")
            download = apt(arch, keyring, "ok", "apt-get", "download", *names)
            check(download.returncode == 0, f"apt-get download ({arch}):\n{download.stdout}{download.stderr}")
            got = sorted(p.name for p in (work / f"apt-ok-{arch}/download").iterdir())
            check(got == sorted(d for d in DEBS if d.endswith(f"_{arch}.deb")), f"downloaded ({arch}): {got}")
        foreign = work / "foreign.gpg"
        run("gpg", "--batch", "--dearmor", "--output", foreign, other_public)
        update = apt("amd64", foreign, "foreign", "apt-get", "update")
        check(update.returncode != 0 or re.search(r"^(W|E):", update.stdout + update.stderr, re.M),
              "apt-get update accepted a repository signed with another key")
        check(not list((work / "apt-foreign-amd64/var/lib/apt/lists").glob("*Packages")), "apt kept an unverified index")
        packages_file = repo / "apt/dists/stable/main/binary-amd64/Packages"
        original = packages_file.read_bytes()
        packages_file.write_bytes(original.replace(b"Phone approval", b"Phone approvaI", 1))
        (packages_file.parent / "Packages.gz").rename(work / "Packages.gz")
        update = apt("amd64", keyring, "tampered", "apt-get", "update")
        check(update.returncode != 0 and "E:" in update.stdout + update.stderr,
              f"apt-get update accepted a changed Packages:\n{update.stdout}{update.stderr}")
        packages_file.write_bytes(original)
        (work / "Packages.gz").rename(packages_file.parent / "Packages.gz")
    else:
        skipped.append("apt-get (not installed)")

    # ── installs in containers ────────────────────────────────────────────────────────────────────────────────────────
    docker = os.environ.get("DOCKER", "docker")
    images = os.environ.get("REINS_TEST_IMAGES", "debian:12 fedora:44")
    if images == "none":
        skipped.append("container installs (REINS_TEST_IMAGES=none)")
    elif not tool(docker) or run(docker, "info", ok=False).returncode != 0:
        skipped.append(f"container installs ({docker} is not available)")
    else:
        result = subprocess.run([str(ROOT / "scripts/package/test-linux-repos.sh"), "--repo", str(repo_out),
                                 "--version", VERSION, "--build", BUILD, "--images", images])
        check(result.returncode == 0, "installing from the repositories failed in a container")

    # ── the update feed with the repositories ─────────────────────────────────────────────────────────────────────────
    # A copy of the scripts with this test's release key pinned in place of the real one, and the throwaway
    # repository key pinned as packages-key.asc.
    fake_root = work / "root"
    (fake_root / "scripts/package/linux-packages").mkdir(parents=True)
    (fake_root / "crates/reins-desktop/src").mkdir(parents=True)
    for script in ("release-feed.sh", "install.sh", "install.ps1"):
        shutil.copy(ROOT / "scripts" / script, fake_root / "scripts" / script)
    run("openssl", "genpkey", "-algorithm", "ed25519", "-outform", "DER", "-out", work / "release.der")
    release_public = subprocess.run(["openssl", "pkey", "-inform", "DER", "-in", str(work / "release.der"), "-pubout",
                                     "-outform", "DER"], capture_output=True, check=True).stdout[-32:]
    (fake_root / "crates/reins-desktop/src/update.rs").write_text(
        f'pub const RELEASE_KEY: &str = "{release_public.hex()}";\n')
    pinned = fake_root / "scripts/package/linux-packages/packages-key.asc"

    # The rest of the release's assets release-feed.sh expects, and the packages as released (the .rpm signed).
    def cli_archive(triple, program="reins"):
        name = f"reins-desktop-{VERSION}-{triple}"
        folder = work / "other" / name
        folder.mkdir(parents=True)
        (folder / program).write_bytes(b"\0program\0" + f"{VERSION} ({BUILD})".encode())
        if program.endswith(".exe"):
            with zipfile.ZipFile(dist / f"{name}.zip", "w") as z:
                z.write(folder / program, f"{name}/{program}")
        else:
            run("tar", "-C", work / "other", "-czf", dist / f"{name}.tar.gz", name)

    for triple in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
        cli_archive(triple)
    cli_archive("x86_64-pc-windows-msvc", "reins.exe")
    for asset in (f"Reins-{VERSION}-macOS.dmg", f"Reins-{VERSION}-Windows-x64.msi",
                  f"Reins-{VERSION}-Linux-x86_64.AppImage", f"reins-{VERSION}-android.apk"):
        (dist / asset).write_bytes(asset.encode() * 100)
    for deb in DEBS:
        shutil.copy(packages / deb, dist / deb)
    for rpm in RPMS:
        shutil.copy(next(repo.rglob(rpm)), dist / rpm)
    feed_args = [fake_root / "scripts/release-feed.sh", "--version", VERSION, "--build", BUILD, "--time", TIME,
                 "--android-version-code", "1", "--dist", dist, "--key", work / "release.der"]

    def feed(out, *extra, ok=True):
        return run(*feed_args, "--out", out, *extra, ok=ok)

    refuses("no pinned key", *feed_args, "--out", work / "x", "--packages", repo_out, expect="needs the repositories")
    shutil.copy(public, pinned)
    feed(work / "feed")
    archive = work / "feed" / f"reins-feed-{VERSION}.tar.gz"
    with tarfile.open(archive) as tar:
        names = tar.getnames()
    check(not any(n.startswith("feed/packages") for n in names), "a feed without --packages has no repositories")
    feed(work / "feed", "--packages", repo_out)
    unpacked = work / "feed-unpacked"
    with tarfile.open(archive) as tar:
        tar.extractall(unpacked, filter="data")
    staged = {str(p.relative_to(unpacked / "feed")) for p in (unpacked / "feed").rglob("*") if p.is_file()}
    for path in ("packages/reins.gpg", "packages/reins.asc", "packages/apt/dists/stable/InRelease",
                 "packages/apt/dists/stable/main/binary-arm64/Packages.gz", "packages/rpm/repodata/repomd.xml.asc",
                 "packages/rpm/reins.repo"):
        check(path in staged, f"the feed lacks {path}")
    check(not any(p.endswith((".deb", ".rpm")) for p in staged), "the feed must fetch the packages, not carry them")
    fetches = [line.split() for line in (unpacked / "feed/fetch.txt").read_text().splitlines()]
    fetched = {dest: (asset, digest, int(size)) for dest, asset, digest, size in fetches}
    for deb in DEBS:
        dest = f"packages/apt/pool/main/r/{deb.split('_')[0]}/{deb}"
        check(fetched.get(dest) == (deb, sha(dist / deb), (dist / deb).stat().st_size), f"fetch.txt: {dest}")
    for rpm, arch in zip(RPMS, ("x86_64", "aarch64", "x86_64")):
        dest = f"packages/rpm/{arch}/{rpm}"
        check(fetched.get(dest) == (rpm, sha(dist / rpm), (dist / rpm).stat().st_size), f"fetch.txt: {dest}")
    # The signed index: as the server checks it (reins-releases-sync), every staged file and every fetched one, by
    # SHA-256, under the release key's signature with the reins-feed/1 context.
    signed = json.loads((unpacked / "feed/feed.json").read_text())
    message = work / "index-message"
    message.write_bytes(b"reins-feed/1\n" + signed["index"].encode())
    signature = work / "index-signature"
    signature.write_bytes(base64.urlsafe_b64decode(signed["signature"] + "=" * (-len(signed["signature"]) % 4)))
    (work / "release-public.der").write_bytes(bytes.fromhex("302a300506032b6570032100") + release_public)
    run("openssl", "pkeyutl", "-verify", "-pubin", "-inkey", work / "release-public.der", "-keyform", "DER",
        "-rawin", "-in", message, "-sigfile", signature)
    index = json.loads(signed["index"])["files"]
    check(set(index) == (staged - {"feed.json"}) | set(fetched), "the index is not exactly the feed's files")
    for path in staged - {"feed.json"}:
        check(index[path] == sha(unpacked / "feed" / path), f"index: {path}")
    for dest, (_, digest, _) in fetched.items():
        check(index[dest] == digest, f"index: {dest}")

    # What the feed refuses: a repository signed with another key, changed after signing, or with a package that is
    # not the release asset of that name.
    def broken_repo(change):
        copy = work / "broken"
        shutil.rmtree(copy, ignore_errors=True)
        shutil.copytree(repo_out, copy)
        change(copy / "packages")
        return copy

    refuses("another repository key", *feed_args, "--out", work / "x", "--packages",
            broken_repo(lambda r: shutil.copy(other_public, r / "reins.asc")), expect="is not the pinned key")
    refuses("a changed Release", *feed_args, "--out", work / "x", "--packages",
            broken_repo(lambda r: (r / "apt/dists/stable/Release").write_text(release + "X: y\n")),
            expect="does not verify")
    refuses("a changed repomd.xml", *feed_args, "--out", work / "x", "--packages",
            broken_repo(lambda r: (r / "rpm/repodata/repomd.xml").write_text("<repomd/>")), expect="does not verify")
    refuses("a package that is not the asset", *feed_args, "--out", work / "x", "--packages",
            broken_repo(lambda r: shutil.copy(packages / RPMS[0], r / "rpm/x86_64" / RPMS[0])),
            expect="is not the release asset")

    # ── the AUR package ──────────────────────────────────────────────────────────────────────────────────────────────
    sums = work / "SHA256SUMS"
    sums.write_text("".join(f"{sha(p)}  {p.name}\n" for p in sorted(dist.iterdir()) if p.is_file()))
    aur = work / "aur"
    run(ROOT / "scripts/package/aur.sh", "--version", VERSION, "--sums", sums, "--out", aur)
    pkgbuild = (aur / "PKGBUILD").read_text()
    run("bash", "-n", aur / "PKGBUILD")
    for name in (f"reins-desktop-{VERSION}-x86_64-unknown-linux-musl.tar.gz",
                 f"reins-desktop-{VERSION}-aarch64-unknown-linux-musl.tar.gz", f"Reins-{VERSION}-Linux-x86_64.tar.gz"):
        check(sha(dist / name) in pkgbuild, f"PKGBUILD lacks the SHA-256 of {name}")
        check(f"releases/download/v{VERSION}/{name}" in (aur / ".SRCINFO").read_text(), f".SRCINFO lacks {name}")
    check(f"pkgver={VERSION}\n" in pkgbuild and "pkgname=reins-bin\n" in pkgbuild, "PKGBUILD header")
    if tool("makepkg") and os.geteuid() != 0:
        printed = run("makepkg", "--printsrcinfo", cwd=aur).stdout
        check(printed == (aur / ".SRCINFO").read_text(), f".SRCINFO differs from makepkg's:\n{printed}")
    else:
        skipped.append("makepkg --printsrcinfo (no makepkg, or root)")
    partial = work / "SHA256SUMS.partial"
    partial.write_text("".join(line for line in sums.read_text().splitlines(True) if "Linux-x86_64.tar.gz" not in line))
    refuses("a release without the app", ROOT / "scripts/package/aur.sh", "--version", VERSION, "--sums", partial,
            "--out", work / "x", expect="is not in")

    # ── the release workflow's signing step ───────────────────────────────────────────────────────────────────────────
    step = release_yml.split("      - name: Sign the packages and build the repositories\n", 1)[1]
    body = []
    for line in step.split("        run: |\n", 1)[1].splitlines():
        if line.strip() and not line.startswith("          "):
            break
        body.append(line)
    shell = textwrap.dedent("\n".join(body))
    secret = run("gpg", "--batch", "--pinentry-mode", "loopback", "--passphrase", PASSPHRASE, "--armor",
                 "--export-secret-keys", key, env=gpg_env).stdout

    def signing_step(case, pinned_key, dotenv=None, environment=None):
        workspace = work / f"step-{case}"
        temp = work / f"step-{case}-temp"
        temp.mkdir()
        shutil.copytree(ROOT / "scripts/package", workspace / "scripts/package")
        (workspace / "scripts/package/linux-packages/packages-key.asc").unlink(missing_ok=True)
        if pinned_key:
            shutil.copy(pinned_key, workspace / "scripts/package/linux-packages/packages-key.asc")
        shutil.copytree(packages, workspace / "packages")
        if dotenv is not None:
            (workspace / ".packages-key.env").write_text(dotenv)
        env = {k: v for k, v in os.environ.items() if k not in ("GNUPGHOME",)}
        env |= {"GITHUB_WORKSPACE": str(workspace), "RUNNER_TEMP": str(temp), "GITHUB_OUTPUT": str(temp / "out"),
                "PACKAGES_GPG_PRIVATE_KEY": "", "PACKAGES_GPG_PASSPHRASE": ""} | (environment or {})
        result = run("bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", shell, ok=False, cwd=workspace, env=env)
        check(not (workspace / ".packages-key.env").exists(), f"{case}: the key file was left behind")
        check(not (temp / "gnupg").exists(), f"{case}: the keyring was left behind")
        return workspace, result

    def signed_rpms(workspace):
        return [rpm for rpm in RPMS if "signatures OK" in run(*rpmkeys, "--checksig", workspace / "packages" / rpm,
                                                                  ok=False).stdout]

    workspace, result = signing_step("unpinned", None)
    check(result.returncode == 0, f"unpinned: {result.stdout}{result.stderr}")
    check("::warning::No repository key is pinned" in result.stdout, "unpinned: no warning")
    check(signed_rpms(workspace) == [], "unpinned: the released .rpm files must stay unsigned")
    check((workspace / "repo/packages/apt/dists/stable/InRelease").exists(), "unpinned: no test repository")
    infisical = f"PACKAGES_GPG_PRIVATE_KEY='{secret.strip()}'\nPACKAGES_GPG_PASSPHRASE='{PASSPHRASE}'\n"
    workspace, result = signing_step("infisical", public, dotenv=infisical)
    check(result.returncode == 0, f"infisical: {result.stdout}{result.stderr}")
    check(signed_rpms(workspace) == RPMS, "infisical: the released .rpm files must be the signed ones")
    check("::add-mask::-----BEGIN PGP PRIVATE KEY BLOCK-----" in result.stdout, "infisical: the key is not masked")
    check(f"::add-mask::{PASSPHRASE}" in result.stdout, "infisical: the passphrase is not masked")
    run("gpgv", "--keyring", keyring, workspace / "repo/packages/apt/dists/stable/InRelease")
    workspace, result = signing_step("fallback", public, environment={"PACKAGES_GPG_PRIVATE_KEY": secret,
                                                                       "PACKAGES_GPG_PASSPHRASE": PASSPHRASE})
    check(result.returncode == 0, f"repository secrets: {result.stdout}{result.stderr}")
    workspace, result = signing_step("missing", public)
    check(result.returncode != 0 and "::error::" in result.stdout, "a pinned key without its secret key must fail")
    other_secret = run("gpg", "--batch", "--armor", "--export-secret-keys", other_key,
                       env=os.environ | {"GNUPGHOME": str(work / "gnupg-other")}).stdout
    workspace, result = signing_step("other", public, dotenv=f"PACKAGES_GPG_PRIVATE_KEY='{other_secret.strip()}'\n")
    check(result.returncode != 0, "a secret key other than the pinned one must fail")
    server.shutdown()
finally:
    subprocess.run(["gpgconf", "--homedir", str(work / "gnupg"), "--kill", "gpg-agent"], capture_output=True)
    subprocess.run(["gpgconf", "--homedir", str(work / "gnupg-other"), "--kill", "gpg-agent"], capture_output=True)
    shutil.rmtree(work, ignore_errors=True)

print(f"{checks} Linux package, repository, feed and AUR checks passed")
for what in skipped:
    print(f"  not run: {what}")
