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

AI coding agents (such as Cursor, Claude, Copilot, Codex, Devin or Jules) never sign, even when they appear as a
commit author or co-author. Under section 5 of the CLA, you, the person who submits the pull request, are the
contributor for the whole pull request, including the parts an agent wrote.

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
to other branches; `gh workflow run ci.yml --ref <branch>` runs it on a branch without a pull request. Its `plan` job
(`scripts/ci/plan.py`) runs, on a pull request, the suites whose inputs the change touches, taken from the Cargo graph
(a `reins-core` change runs the server, Android and iOS suites, not the desktop ones), and on `main` every suite.
Either way it skips a suite that already passed on the same inputs. On a pull request the desktop macOS job runs only
when the change can be macOS specific; add the `ci:full` label and re-run to run it anyway. The one check to look at is **CI result**. macOS jobs wait for free runners in
turn (`macos-gate.yml`, releases first); `ci:priority` moves a pull request to the front, and the **CI emergency**
workflow locks the runners for one. See [docs/reference/ci.md](docs/reference/ci.md). Caches (Rust,
Gradle, Robolectric, the Android native outputs) are written only by runs on `main`, pushes and a weekly run that
selects every lane; pull requests and branch runs restore `main`'s and write none but their tiny pass records. A newer
push to a pull request cancels its run in progress; runs on `main` are never cancelled. Server/core tests use Nextest's
prebuilt binary and one workspace feature set, with doctests retained in a separate Cargo pass. Clippy runs on its own
runner to avoid Cargo's build-directory lock.

Android (needs the Android SDK and NDK; see [android/README.md](android/README.md)):

```sh
cd android
./gradlew assembleFullDebug        # builds the Rust core with cargo-ndk and generates the Kotlin bindings
./gradlew testFullDebugUnitTest      # JVM and Robolectric tests, no device needed
```

iOS: building, running and testing need a Mac with Xcode (see [ios/README.md](ios/README.md)). On Linux,
`scripts/ios-typecheck.sh` type-checks every target's Swift against the iPhoneOS SDK in about ten seconds, with the
compilation conditions Xcode uses; run it after changing Swift and before pushing. It needs Swift (swiftly) and xtool's
Darwin SDK, made from an Xcode .xip, so CI cannot run it: see
[ios/README.md, "Compile check on Linux"](ios/README.md#compile-check-on-linux).
Then `scripts/ios-job.sh` builds and tests on a macOS runner (or the Mac with `--mac`) and copies the results back:
snapshot PNGs of the main screens, or one unit-test class. UI tests run in CI and never block a merge. See
[docs/ios-dev.md](docs/ios-dev.md).

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
and all required assets. Only a successful push CI run on `main` qualifies. A newer push's release starts at once and
cancels older release runs that have not reached publication (it releases their commits too); a run that has begun
publishing is never cancelled, the newer one waits for its tag before computing its version, and two runs never publish
at once (`scripts/supersede-releases.py`). Runs started by hand are never cancelled. Every push to `main` that passes CI is released: `.github/workflows/release.yml` tags the commit CI tested as
`vX.Y.Z` and publishes a [GitHub release](https://github.com/katulevskiy/reins/releases) with the desktop app (Linux
x86_64/aarch64, static; macOS Apple silicon/Intel; Windows x86_64/Arm as zips), its Linux `.deb` and `.rpm` packages
(see [below](#maintainers-linux-packages-their-repositories-and-the-aur)), the server binary, the server image
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
version/latest tags. It restores the build caches and writes none: they, and the per-architecture GHCR image caches,
are written from `main` only. On `main`, a `verify_only` run also builds and signs the iOS app (`ios-testflight`) and
uploads nothing.

Server, CLI, Android and iOS release builds use Thin LTO and 16 codegen units. `cargo build --profile release-fat` retains the previous fat
LTO/one-codegen-unit configuration for explicit optimization benchmarks. The installer profile keeps optimization
level 3 with LTO disabled to avoid long GPUI links and large bitcode caches. Installers compile the GUI and bundled CLI
under one profile to share dependencies; Windows CLI downloads reuse the installer binary. Both macOS GUI architectures
compile in one cargo run on an Apple silicon runner (x86_64 cross-compiled) before universal bundling and signing.
Standalone macOS CLI builds (both architectures) remain a separate cargo run to preserve macOS 11 support (the GUI
requires macOS 12), before any signing secret is fetched. All of that is one job, `macos`: the account runs
five GitHub-hosted macOS jobs at once, shared with CI, so a release takes one macOS runner before publication (and one
after it, for TestFlight). CI's macOS jobs wait while a release still waits for its runner
(`.github/workflows/macos-gate.yml`; docs/reference/ci.md, "Admission"). The iOS core's uniffi-bindgen is a host tool built with the unoptimized `host-tool` profile. Android builds each ABI and the Kotlin bindings on separate workers; native
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

Bots, AI coding agents and maintainers who do not need to sign are listed in the workflow's `allowlist`.

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
testing tracks. The `play` job (after the release is published) uploads `reins-<version>-play.aab` to every track that takes it:
the repository variable `PLAY_TRACK`, comma-separated (default `internal,alpha,beta,production`; a track Google Play
does not open to the app yet is skipped with a warning), with the status in `PLAY_RELEASE_STATUS` (default
`completed`; `draft` until the app's first release is rolled out) through `scripts/play-upload.py`, and skips the
upload with a notice while the folder or the secret is missing; a repository secret of the same name is the fallback.
`INFISICAL_PLAY_PATH` overrides the folder. Set up and first upload: `android/PLAY_STORE.md`.

## Maintainers: TestFlight

Once the release is published, the `ios-testflight` job (runner `xcode-27`, its released Xcode 27.0) archives and
signs the iOS app, then exports it for the App Store and uploads it to App Store Connect ("Reins 2FA",
`com.reins2fa.app`). It is one job after publication, not a build beside the others: its signing material is the
`testflight` environment's, which the `macos` job, bound to the main ref's Infisical identity, cannot have, and a
second macOS job before publication waited in the account's macOS queue on its own. Then
`scripts/testflight.py` waits for processing, sets export compliance (`usesNonExemptEncryption` false, as the
Info.plist says), sets "What to Test" from the release notes, adds the build to the TestFlight groups and submits it for
beta app review. A refused submission (a build already in review, for example) is only a warning, and a failure here
never touches the GitHub release: rerun the failed job. A newer release's upload cancels an older one still waiting or
running. To test or redo one release, run the Release workflow from main with `testflight_tag` = `vX.Y.Z`; only this
job runs.

The build number is the release's Android versionCode (minutes since 1970), or one more than the highest build App
Store Connect already has, whichever is larger. The version is the release's.

Its signing material comes only from the secrets of the GitHub environment `testflight` (Settings > Environments >
testflight); the job fails naming any that is missing. They are copies: the master copies are in Infisical
`reins-release` prod `/signing/apple` and in 1Password, so rotating one means updating all three.

| Environment secret | Value |
| --- | --- |
| `IOS_DIST_CERT_P12`, `IOS_DIST_CERT_PASSWORD` | the Apple Distribution certificate and key (`.p12`, base64) and its password |
| `IOS_PROFILE_APP`, `IOS_PROFILE_WIDGETS`, `IOS_PROFILE_NOTIFICATIONS` | the App Store profiles (`.mobileprovision`, base64) of `com.reins2fa.app`, `.widgets`, `.notifications` |
| `AC_API_KEY_P8`, `AC_API_KEY_ID`, `AC_API_ISSUER_ID`, `APPLE_TEAM_ID` | the "Reins CI" API key (App Manager), as for notarization |

The job does not use Infisical (its plan allows no second identity, and the release identity `github-release-android`
keeps only its main-ref subject; an environment's OIDC subject would not match it), and asks for no OIDC token.

Only a commit on main that carries a published release tag is built, and only from main (a release push, or the
dispatch, which checks the tag before anything is checked out). The Rust core and the Swift packages (pinned to the
revisions in `Package.resolved`, which the job checks, with their plugins listed) build before any step is given a
secret. Each secret is mapped only into the step that needs it, written to a file in a `0700` folder, never to
`GITHUB_ENV` or a command line: the certificate and its password into the keychain step, which removes their files once
the certificate is in the keychain (non-extractable, for `codesign` only); the profiles into the profile step; the API
key into the build-number step, which removes it, and again after the archive for the upload and TestFlight. So the
archive's build phases and packages find only the keychain, the profiles and the settings (a step checks). The keychain
is locked after the upload, and the last step deletes it with everything else. The third-party actions it uses are
pinned to commits. Restrict the `testflight` environment to `main` (Deployment branches and tags).

Repository variables: `TESTFLIGHT_EXTERNAL_GROUP_ID` (the external group's id; two groups are named "Testers", so a
name alone fails), `TESTFLIGHT_INTERNAL_GROUPS` (default `Internal`, which gets every build by itself) and, optionally,
`IOS_GOOGLE_CLIENT_ID` (Google sign-in in the app).

One-time setup on the Mac. Done: the app record, the Apple Distribution certificate and the three App Store profiles
(App Groups `group.com.reins2fa.app` on all three; Push Notifications, Associated Domains and Time Sensitive
Notifications on the app), stored base64 in `/signing/apple` with master copies in 1Password, and the groups
"Internal" and "Testers" (public link on). Still to do:

1. `gh variable set TESTFLIGHT_EXTERNAL_GROUP_ID -R katulevskiy/reins --body <id of the "Testers" group with the public link>`.
2. App Store Connect > Pricing and Availability: exclude France.
3. The nine secrets above, copied from `/signing/apple` into the `testflight` environment's secrets, e.g.
   `gh secret set AC_API_KEY_P8 -R katulevskiy/reins --env testflight < AuthKey.p8` (values from files or stdin,
   never on the command line).
4. Settings > Environments > `testflight` > Deployment branches and tags: `main` only.

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

## Maintainers: the Homebrew cask

`brew install --cask katulevskiy/tap/reins` comes from https://github.com/katulevskiy/homebrew-tap (`Casks/reins.rb`).
After each release the `homebrew` job writes the cask from the release's `SHA256SUMS`
(`scripts/package/homebrew-cask.sh`: the versioned `.dmg`, its SHA-256, the `reins` command linked from the app) and
pushes it with the tap's deploy key, which can write to that repository only (Infisical reins-release prod
`/signing/homebrew`: `HOMEBREW_TAP_DEPLOY_KEY`; master copy in 1Password, "Reins Homebrew tap deploy key"). The cask
says `auto_updates true`, as the app updates itself. A failure there does not block the release; rerun the job.

## Maintainers: Linux packages, their repositories and the AUR

Every release also carries `reins_<version>_{amd64,arm64}.deb`, `reins-<version>-1.{x86_64,aarch64}.rpm` (the static
command-line program) and `reins-app_<version>_amd64.deb`, `reins-app-<version>-1.x86_64.rpm` (the desktop app, which
needs `reins`), made with nfpm (version and SHA-256 pinned in `release.yml` and `ci.yml`) by
`scripts/package/linux-packages.sh` from the archives and the app's tarball; the definitions are in
`scripts/package/linux-packages/`. The release's `linux-packages` job lints them (lintian, rpmlint, with the accepted
findings in `lintian/` and `rpmlint.toml` there), signs the `.rpm` files with `rpmsign`, builds the signed APT and RPM
repositories (`scripts/package/linux-repos.sh`) and installs from them in Ubuntu, Debian and Fedora containers
(`scripts/package/test-linux-repos.sh`, in `linux-packages-amd64` and `linux-packages-arm64`). A failure there holds the
release back, as any other asset's does. `release-feed.sh --packages` puts the repositories in the update feed: their
metadata as files, the `.deb` and `.rpm` files as `fetch.txt` lines naming the release assets, all of it in the signed
index. Users find them at `reins2fa.com/releases/packages/` ([Linux packages](docs/linux-packages.md)).
`scripts/test-linux-packages.py` tests all of this on a fake release with throwaway keys (CI runs it).

The repositories hold one release, the one in the feed: each feed replaces them whole. apt and dnf only ever install
the newest version, and every version stays a GitHub release asset for a downgrade by hand
(`apt install ./reins_<version>_amd64.deb`). Carrying the previous version too would need the server to fetch assets
of another release than the feed's, which `fetch.txt` cannot name; what it would buy, a client with an index from
before a release that downloads after it, the server covers by keeping replaced files for 14 days (below).

**The repository key** is an OpenPGP key, RSA 4096 (what every apt, dnf and rpm verifies). Make it once, outside the
repository:

```sh
export GNUPGHOME="$(mktemp -d)"
gpg --quick-gen-key "Reins packages <support@reins2fa.com>" rsa4096 sign never   # asks for a passphrase
gpg --armor --export "Reins packages" > scripts/package/linux-packages/packages-key.asc
gpg --armor --export-secret-keys "Reins packages" > reins-packages-private.asc  # mode 0600, not in the repository
```

and store it in Infisical `reins-release`, `prod`, `/signing/packages`: `PACKAGES_GPG_PRIVATE_KEY` (the whole armored
block of `reins-packages-private.asc`) and `PACKAGES_GPG_PASSPHRASE` (leave it out for a key without one). The same
release identity reads it, through a file the next step removes; repository secrets of the same names are the
fallback. The master copy belongs in 1Password next to the release key. Committing the public key,
`scripts/package/linux-packages/packages-key.asc`, is what switches the repositories on: until then the release signs
test repositories with a throwaway key, releases the `.rpm` files unsigned and leaves them out of the feed (with a
warning); from then on a missing secret key, or one that is not the committed key, fails the release. Rotating the
key means users import the new one, so give it no expiry and keep it safe instead.

Before committing the key, the server's `reins-releases-sync` (reins-site, `deploy/server/`) has to know the
repositories, or it refuses the feed (its `fetch.txt` destinations are only `files/` and `android/files/` today):

1. accept `fetch.txt` destinations `packages/apt/pool/main/<letter>/<package>/<file>.deb` and
   `packages/rpm/<arch>/<file>.rpm` (each still a release asset with that SHA-256, and in the signed index);
2. publish every `packages/` file of the signed index into `/srv/reins-releases/packages/`: the packages and
   `repodata/*`, `Packages*` first, then with atomic renames the entry points that name them,
   `apt/dists/stable/{Release,Release.gpg,InRelease}`, `rpm/repodata/repomd.xml{,.asc}`, `rpm/reins.repo`,
   `reins.gpg` and `reins.asc`, so no client sees an index whose files are not there yet;
3. keep replaced files under `packages/` for 14 days, as it does for `files/`.

**The AUR package** `reins-bin` is not published yet. `scripts/package/aur.sh` writes its PKGBUILD and .SRCINFO for
each release, from the release's `SHA256SUMS`. The release's `aur` job checks .SRCINFO and the sources with makepkg
in an Arch Linux container. A push to `ssh://aur@aur.archlinux.org/reins-bin.git` needs an AUR account with an SSH
key: `ssh-keygen -t ed25519 -N '' -C reins-bin -f aur_ed25519`, the public half in the account's settings, the private
half (the whole OpenSSH key file) in Infisical `/signing/aur` as `AUR_SSH_PRIVATE_KEY`. Without that key the job only
says so. The job cannot fail or hold back a release. Until the package is on the AUR, Arch users install with
`curl -fsSL https://reins2fa.com/install.sh | sh`.

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
