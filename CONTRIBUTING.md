# Contributing

Thanks for helping. Bug reports, fixes, tests, documentation and new connectors are all welcome. For anything large
(a new service, a change to the protocol or to how approvals work), open an issue first so we can agree on the shape
before you write the code.

Security problems: do not open a public issue; see [SECURITY.md](SECURITY.md).

## Contributor License Agreement

Every contributor signs the [Contributor License Agreement](CLA.md) (CLA) once, before their first pull request can be
merged. It is modeled on the Apache ICLA: you keep the copyright of your work and grant the project owner a copyright
and patent license, including the right to license your contribution under other terms (for example a commercial
license). [LICENSING.md](LICENSING.md) explains the
licenses.

How to sign: open your pull request. The CLA bot comments on it; reply with exactly

    I have read the CLA Document and I hereby sign the CLA

The bot records your GitHub account in the signatures branch and the check turns green. If it does not, comment
`recheck`. Every author of every commit in the pull request must sign, so make sure your commits use an email address
linked to your GitHub account.

We do not use the Developer Certificate of Origin (DCO): no `Signed-off-by:` line is needed.

Code that is plain upstream Vaultwarden (a fix to the vault server that has nothing to do with Reins) is best sent
to [Vaultwarden](https://github.com/dani-garcia/vaultwarden) directly; we pick it up from there.

## Repository layout

| Path | What | License |
| --- | --- | --- |
| `src/`, `macros/`, `migrations/`, `tests/` | the server: Vaultwarden plus the AI permission relay (`src/api/reins/`) | AGPL-3.0-only |
| `crates/reins-proto` | types on the wire between server, phone and desktop | Apache-2.0 |
| `crates/reins-policy` | standing permissions: scopes and their evaluation | Apache-2.0 |
| `crates/reins-core` | the phone's core (Rust, exposed to Kotlin through UniFFI): vault, connectors, approvals, Autopilot | Apache-2.0 |
| `crates/reins-desktop` | the desktop app (`reins`): daemon, git/SSH/API proxies, harness hooks | Apache-2.0 |
| `crates/reins-desktop-app` | the Reins app (GPUI): pairing window, setup, tray icon; installers via `scripts/package/` | Apache-2.0 |
| `crates/reins-laya` | Autopilot's model on the desktop (ONNX Runtime) and `laya-try` | Apache-2.0 |
| `crates/reins-e2e` | end-to-end tests: the real server, the real phone core, the desktop daemon | Apache-2.0 |
| `android/` | the Android app (Kotlin, Jetpack Compose) | Apache-2.0 |
| `tools/laya/` | training and export of the Autopilot model (Python) | Apache-2.0 |
| `docs/` | deployment guide, design specs and plans | |

## Building and testing

Get the code with `git clone https://github.com/katulevskiy/reins`. The Rust toolchain is pinned in
`rust-toolchain.toml`; use rustup so the pinned version is picked up. The server needs at least one database backend
feature; SQLite is the easiest locally.

```sh
# Server
cargo build --features sqlite
cargo test --features sqlite

# Reins crates (fast, no database)
cargo test -p reins-proto -p reins-policy -p reins-core -p reins-desktop -p reins-laya

# End to end: builds target/debug/vaultwarden and runs it against the phone core and the desktop daemon
cargo build --features sqlite
cargo test -p reins-e2e

# Lints, as CI runs them
cargo fmt --all -- --check
cargo clippy --workspace --exclude vaultwarden --exclude reins-desktop-app --all-targets -- -D warnings
cargo clippy --features sqlite -- -D warnings
cargo clippy -p reins-desktop-app --all-targets -- -D warnings
cargo deny --workspace check licenses bans      # https://github.com/EmbarkStudios/cargo-deny
scripts/check-doc-links.py                      # relative links in README.md, the top-level *.md and docs/
```

Some tests need tools on the machine (`git`, for the git proxy tests) or skip themselves when optional data is absent
(the Laya model package: set `REINS_LAYA_PKG`).

`scripts/workos-live.sh` checks WorkOS sign-in against a real WorkOS **staging** environment (it makes and deletes a
test user; needs Google Chrome and a free port 8765). Maintainers sign in with `infisical login` (the
project is set in `.infisical.json`) and run `infisical run --env=dev -- scripts/workos-live.sh`. Anyone else sets `WORKOS_CLIENT_ID` and `WORKOS_API_KEY`
(`sk_test_...`) in the environment, or puts them in a git-ignored `workos.env` at the repository root.

The desktop app's Windows code: CI runs its clippy and tests on a Windows runner (`desktop-windows` in
`.github/workflows/ci.yml`), including `tests/windows_service.rs`, which installs and removes the real background
service and so runs only with `REINS_TEST_WINDOWS_SERVICE=1`. From Linux or macOS, `rustup target add
x86_64-pc-windows-gnu` and MinGW-w64 let you check it: `cargo clippy -p reins-desktop --all-targets --target
x86_64-pc-windows-gnu -- -D warnings`. Code that only Windows runs lives behind `cfg(windows)`; what can be a plain
function (paths, quoting, the PE header, `reg` output) lives in `src/win.rs` without one, so its tests run everywhere.

CI (`.github/workflows/ci.yml`) runs once per pull request update and on every push to `main`, not on pushes
to other branches; `gh workflow run ci.yml --ref <branch>` runs it on a branch without a pull request. Pull requests that
only touch the docs skip the Rust jobs. Rust build caches are written by `main`, manual branch runs and same-repository pull requests; fork pull requests only
restore them. Server/core tests use Nextest's prebuilt binary and one workspace feature set, with doctests retained
in a separate Cargo pass. Clippy runs on its own runner to avoid Cargo's build-directory lock.

Android (needs the Android SDK and NDK; see [android/README.md](android/README.md)):

```sh
cd android
./gradlew assembleFullDebug        # builds the Rust core with cargo-ndk and generates the Kotlin bindings
./gradlew testFullDebugUnitTest      # JVM and Robolectric tests, no device needed
```

`tools/laya` (Python): see [tools/laya/README.md](tools/laya/README.md).

[pre-commit](https://pre-commit.com) runs the formatting, spelling and lint checks before each commit:
`pre-commit install`.

## Style

* Rust: `rustfmt.toml` (120 columns) and the workspace lints in `Cargo.toml` (`[workspace.lints]`); clippy warnings
  are errors. Prefer small, typed functions over stringly-typed plumbing, return errors instead of panicking, and never
  log secrets or message contents (`crates/reins-core/tests/no_secrets_in_logs.rs` checks the core).
* Kotlin: the existing code style of `android/` (Compose, view models, `MainSafeCore` for every core call). UI changes
  come with a Robolectric test; the screenshot test renders every screen.
* Tests: new behaviour comes with tests, and a bug fix with a test that fails without it. Security-relevant code (the
  relay, policy evaluation, sealing, approvals) needs tests for the hostile cases too.
* Commits: one logical change each, with a subject that names the area, like the existing history
  (`core: ...`, `android: ...`, `server: ...`, `desktop: ...`, `docs: ...`), or a
  [Conventional Commits](https://www.conventionalcommits.org) type (`feat(desktop): ...`, `fix: ...`). The subject
  decides the next version number; see [Commit messages and releases](#commit-messages-and-releases).
* Do not commit secrets, real account data or personal information, including in test fixtures; use `example.com`
  addresses.

## Commit messages and releases

Release builds start alongside CI on every push to `main`; publication waits for successful CI on that exact commit
and all required assets. Only a successful push CI run on `main` qualifies. Obsolete runs are cancelled when a newer
push arrives. Every push to `main` that passes CI is released: `.github/workflows/release.yml` tags the commit CI tested as
`vX.Y.Z` and publishes a [GitHub release](https://github.com/katulevskiy/reins/releases) with the desktop app (Linux
x86_64/aarch64, static; macOS Apple silicon/Intel; Windows x86_64/Arm as zips), the server binary, the server image
`ghcr.io/katulevskiy/reins-server`, the Android APK and `SHA256SUMS`, with notes generated from the commits. Nobody
bumps a version by hand: the tags are the source of truth, and `scripts/next-version.sh` computes the next one from
the commits since the latest tag:

| Commit message | Bump | Example |
| --- | --- | --- |
| `!` after the type or scope, or a `BREAKING CHANGE:` footer | major (minor while the version is 0.x) | `feat(proto)!: version 2 of the wire format` |
| `feat: ...` or `feat(scope): ...` | minor | `feat(desktop): macOS builds` |
| anything else: `fix`, `perf`, `docs`, `ci`, `chore`, `refactor`, `test`, and the `area: message` style | patch | `android: fix the settings screen` |

The highest level among the commits wins, so a release with one feature and ten fixes is a minor release. A commit
whose message contains `[skip release]` does not count; when nothing since the last tag counts, nothing is released
(the next counting commit releases everything). Release notes group the commits into Breaking changes, Features
(`feat`), Fixes (`fix`) and Other.

Try it locally: `scripts/next-version.sh --describe` (what would be released from `HEAD`), `scripts/release-notes.sh
--version X.Y.Z --previous vA.B.C --repo https://github.com/katulevskiy/reins` (the notes), and
`scripts/test-next-version.sh` (the tests CI runs).

Maintainers can release by hand from the Actions tab: run **Release** on `main` and pick a bump level (`auto` uses the
commits). It releases the tip of `main` if CI passed on it.

To measure the complete release pipeline before merging, run **Release** on a CI-tested branch with `verify_only`
enabled (`gh workflow run release.yml --ref <branch> -f verify_only=true`). It builds and verifies the signed APK,
installers, archives and both server image architectures, but creates no GitHub release and does not update image
version/latest tags. Build caches are refreshed, including the per-architecture GHCR caches.

Server, CLI and Android release builds use Thin LTO and 16 codegen units. `cargo build --profile release-fat` retains the previous fat
LTO/one-codegen-unit configuration for explicit optimization benchmarks. The installer profile keeps optimization
level 3 with LTO disabled to avoid long GPUI links and large bitcode caches. Installers compile the GUI and bundled CLI
under one profile to share dependencies; Windows CLI downloads reuse the installer binary. macOS GUI architectures
compile on separate native runners before universal bundling and signing. Standalone macOS CLI
builds remain separate to preserve macOS 11 support (the GUI requires macOS 12). Android builds each ABI and the Kotlin bindings on separate workers; native
outputs are reused only for an exact native source/configuration hash. CI uses separate debug output caches and
retains both Android flavor test suites. Every packaged APK verifies both ELF
architectures as well as the production signing certificate and installed identity.

## Pull requests

* Keep them focused; explain what changes for the user and why.
* Make sure the checks above pass locally.
* A maintainer reviews every pull request. Changes to security-sensitive areas may take longer and may need more tests.

## Maintainers: setting up the CLA check

The CLA check is `.github/workflows/cla.yml`, using
[contributor-assistant/github-action](https://github.com/contributor-assistant/github-action). Once per repository:

1. Create the branch that stores the signatures, with no history of its own:
   `git switch --orphan cla-signatures && git commit --allow-empty -m "CLA signatures" && git push origin cla-signatures`.
   The action writes `signatures/version1/cla.json` there.
2. Settings → Actions → General → Workflow permissions: allow read and write (the workflow asks only for what it needs:
   contents to commit signatures, pull requests and statuses to comment and set the check).
3. Protect `cla-signatures` against deletion and force pushes, without blocking pushes by GitHub Actions (a branch
   ruleset that targets that branch and lists the GitHub Actions app as a bypass actor).
4. Settings → Branches (or rulesets) for the default branch: require the `CLAAssistant` status check before merging.
5. When the text of CLA.md changes in substance, bump the signature path in the workflow
   (`signatures/version2/cla.json`) so everyone signs the new version.

Bots and maintainers who do not need to sign are listed in the workflow's `allowlist`.

## Maintainers: signing the APK in releases

Infisical is the source of the Android release signing secrets. They live in their own project, `reins-release`
(id `e21d41de-d3f4-41bf-accb-b4c2df9f164d`, set in `.github/actions/release-secrets/action.yml`), not in the
`.infisical.json` project with the server's settings: Infisical's free plan has no folder-scoped roles, so a separate
project is what keeps the release identity from reading anything else. The release workflow reads `/signing/android`
in its `prod` environment through GitHub OIDC as the machine identity `github-release-android` (Viewer on
`reins-release` only, no organization access). Its OIDC login only accepts tokens with audience
`https://github.com/katulevskiy`, subject `repo:katulevskiy@84909978/reins@1400899427:ref:refs/heads/main` (the repository
uses GitHub's immutable subject format: owner and repository ids, which survive renames), repository and owner ids
`1400899427` / `84909978`, and `job_workflow_ref` `katulevskiy/reins/.github/workflows/release.yml@refs/heads/main`. The GitHub Actions variable
`INFISICAL_RELEASE_IDENTITY_ID` holds its identity id. `INFISICAL_RELEASE_ENVIRONMENT` and
`INFISICAL_ANDROID_SIGNING_PATH` override the environment and folder when needed. The workflow never needs an
Infisical API key. See the
[Infisical GitHub OIDC setup](https://infisical.com/docs/documentation/platform/identities/oidc-auth/github).

| Infisical secret | Value |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | the dedicated production keystore, base64-encoded |
| `ANDROID_KEYSTORE_PASSWORD` | its password |
| `ANDROID_KEY_ALIAS` | the key alias |
| `ANDROID_KEY_PASSWORD` | the key password |

Import a private dotenv file with
`infisical secrets set --projectId=e21d41de-d3f4-41bf-accb-b4c2df9f164d --env=prod --path=/signing/android --file=android-release.env`.
Keep the keystore and import file outside the repository, with mode `0600`. The master copy is the 1Password item
"Reins Android release key" (the `.p12`, its passwords, and the `android-release.env` import file). Existing GitHub repository secrets of the
same names remain a fallback while migrating. Without complete signing credentials, the release fails before
building or publishing. Every release must include the signed Android APK.

The same identity reads `/signing/release` (`RELEASE_SIGNING_KEY_BASE64`: the Ed25519 release key, PKCS#8 DER,
base64) in the publish job, which signs the update feed with it (`scripts/release-feed.sh`; the key is written to a
file for that one step, never exported). The master copy is the 1Password item "Reins release signing key (Ed25519)";
its public half is pinned in `crates/reins-desktop/src/update.rs`.

The same identity reads `/signing/apple` for the macOS app: `MACOS_CERT_P12` (the "Developer ID Application:
Daniil Katulevskiy (5DRCQV8HY2)" certificate and key, `.p12`, base64), `MACOS_CERT_PASSWORD`, and the App Store Connect
API key "Reins CI" (App Manager) for notarization and TestFlight: `AC_API_KEY_P8` (the `.p8` text), `AC_API_KEY_ID`,
`AC_API_ISSUER_ID`, plus `APPLE_TEAM_ID`. The signing step reads them from a file the Infisical action writes and
removes it; repository secrets of the same names are the fallback. The master copies are the 1Password items "Reins
Developer ID Application certificate" and "Reins App Store Connect API key". The server's APNs key ("Reins APNs key" in
1Password) is `/etc/reins/AuthKey_<key id>.p8` on the server, configured by `REINS_APNS_*` in the `reins` project.

The same identity reads `/signing/play` for Google Play: `PLAY_SERVICE_ACCOUNT_JSON`, a Google Cloud service
account's JSON key (the whole file as the value) with the Play Console permission to release com.reins2fa.app to
testing tracks. The `play` job (after the release is published) uploads `reins-<version>-play.aab` to the track in the
repository variable `PLAY_TRACK` (default `internal`) with the status in `PLAY_RELEASE_STATUS` (default
`completed`; `draft` until the app's first release is rolled out) through `scripts/play-upload.py`, and skips the
upload with a notice while the folder or the secret is missing; a repository secret of the same name is the fallback.
`INFISICAL_PLAY_PATH` overrides the folder. Set up and first upload: `android/PLAY_STORE.md`.

## Maintainers: the update feed

Every release carries `reins-feed-<version>.tar.gz` (listed in `SHA256SUMS`): the signed `latest.json` and
`latest-<platform>.txt` for `reins update` and the install scripts, the command-line programs they name, the signed
`app.json` for the desktop app's one-click update, `android/latest.json` for the APK's updater, `install.sh`,
`install.ps1`, `fetch.txt`, which names the installers and the APK (already release assets) by SHA-256 instead of
copying them, and `feed.json`, the index of all of these with their SHA-256, signed with the release key (context
`reins-feed/1`). The server's `reins-releases-sync` timer (reins-site, `deploy/server/`) mirrors the newest release's
feed into `/srv/reins-releases` every 15 minutes and publishes only files the verified index lists, so a
release reaches `reins2fa.com/releases`, `/install.sh` and `/install.ps1` without anyone logging in to the server.
`scripts/release-feed.sh` also builds a feed by hand from a directory of release assets.

Production signing uses a dedicated non-debug RSA-4096 identity, alias `reinsrelease`. Its SHA-256 certificate is
`61edfc4c65cbdfa1b7a07109a9df347de93500b52012c3380b38c7c4a4e3f1c8`, pinned in
`android/release-signing.sha256`. Both the local release script and CI reject another certificate before compiling.
Release variants without explicit signing credentials are unsigned; the local publishing script refuses missing
credentials. The private keystore and prepared Infisical dotenv are outside the repository. Import that production
bundle; the historical `androiddebugkey` is retained separately for recovery/testing and must not be uploaded to a
store. Repackaging a debug key with a stronger password does not make its certificate suitable for publishing.
See [Android's signing guide](https://developer.android.com/studio/publish/app-signing).

Earlier Android builds used a different package id; Reins is `com.reins2fa.app`, so it installs as a separate app.
Any `com.reins2fa.app` test/direct APK signed with the historical debug certificate also cannot update to the new
production certificate. Save and verify the vault recovery code before reinstalling; plan authenticated migration
rather than discarding encrypted data. Later production updates must keep the new identity.

For the initial Google Play upload, this production key can also be configured as the upload key through
`reins.uploadKeystore`, `reins.uploadKeystorePassword`, `reins.uploadKeyAlias`, `reins.uploadKeyPassword` (or the
corresponding `REINS_UPLOAD_*` environment settings). Google Play's app signing key may differ from the upload key;
register the actual Play certificate with Firebase/Google. A separate upload key can be registered later. Building a
signed debug APK is signing verification, not a production release (see `android/PLAY_STORE.md`).

Locally the same signing is
`REINS_RELEASE_KEYSTORE=... REINS_RELEASE_KEYSTORE_PASSWORD=... REINS_RELEASE_KEY_ALIAS=... ./gradlew
assembleFullRelease` (or the `reins.releaseKeystore` Gradle properties; see `android/app/build.gradle.kts`).

The server image is published to GitHub Packages as `ghcr.io/katulevskiy/reins-server` (public, linked to this
repository), tagged with the version and `latest`.
