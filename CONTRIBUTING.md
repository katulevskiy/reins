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
| `src/`, `macros/`, `migrations/`, `tests/` | the server: Vaultwarden plus the AI permission relay (`src/api/rewarden/`) | AGPL-3.0-only |
| `crates/rewarden-proto` | types on the wire between server, phone and desktop | Apache-2.0 |
| `crates/rewarden-policy` | standing permissions: scopes and their evaluation | Apache-2.0 |
| `crates/rewarden-core` | the phone's core (Rust, exposed to Kotlin through UniFFI): vault, connectors, approvals, Autopilot | Apache-2.0 |
| `crates/rewarden-desktop` | the desktop app (`rewarden`): daemon, git/SSH/API proxies, harness hooks | Apache-2.0 |
| `crates/rewarden-laya` | Autopilot's model on the desktop (ONNX Runtime) and `laya-try` | Apache-2.0 |
| `crates/rewarden-e2e` | end-to-end tests: the real server, the real phone core, the desktop daemon | Apache-2.0 |
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
cargo test -p rewarden-proto -p rewarden-policy -p rewarden-core -p rewarden-desktop -p rewarden-laya

# End to end: builds target/debug/vaultwarden and runs it against the phone core and the desktop daemon
cargo build --features sqlite
cargo test -p rewarden-e2e

# Lints, as CI runs them
cargo fmt --all -- --check
cargo clippy --workspace --exclude vaultwarden --all-targets -- -D warnings
cargo clippy --features sqlite --all-targets -- -D warnings
cargo deny --workspace check licenses bans      # https://github.com/EmbarkStudios/cargo-deny
scripts/check-doc-links.py                      # relative links in README.md, the top-level *.md and docs/
```

Some tests need tools on the machine (`git`, for the git proxy tests) or skip themselves when optional data is absent
(the Laya model package: set `REWARDEN_LAYA_PKG`).

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
  log secrets or message contents (`crates/rewarden-core/tests/no_secrets_in_logs.rs` checks the core).
* Kotlin: the existing code style of `android/` (Compose, view models, `MainSafeCore` for every core call). UI changes
  come with a Robolectric test; the screenshot test renders every screen.
* Tests: new behaviour comes with tests, and a bug fix with a test that fails without it. Security-relevant code (the
  relay, policy evaluation, sealing, approvals) needs tests for the hostile cases too.
* Commits: one logical change each, with a subject that names the area, like the existing history
  (`core: ...`, `android: ...`, `server: ...`, `desktop: ...`, `docs: ...`).
* Do not commit secrets, real account data or personal information, including in test fixtures; use `example.com`
  addresses.

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
