# iOS development: the fast loop

Write Swift on Linux and let a Mac build and test it. Nobody waits on a simulator they started by hand. Screens are
checked from PNGs rendered in the unit-test bundle, not from UI tests.

```
edit on Linux ─> scripts/ios-typecheck.sh ─> push ─> scripts/ios-job.sh snapshots / unit:Class ─> look at the PNGs ─> PR
                   (~10 s, no Mac)                    (a GitHub macOS runner, or the Mac with --mac)
```

XCUITest is for flows: CI runs it, or one class at a time with `ui:Class`. A pull request never waits on a local UI
test run, and the UI shards in CI never block a merge or a release.

## 1. Type-check on Linux

```bash
scripts/ios-typecheck.sh                  # every target, ~10 s warm
scripts/ios-typecheck.sh Reins ReinsTests
```

This catches type and API errors in seconds. It does not link, run, or check assets: see
[ios/README.md](../ios/README.md#compile-check-on-linux).

## 2. Run a job

`scripts/ios-job.sh` runs the job on GitHub's macOS runners. It builds the branch's pushed head, so push first.
It waits for the job, then copies the results to `~/Documents/dev/tmp/ios-jobs/<run>/`. It prints PASS or FAIL,
each step's time and the paths, and exits 0 only when everything passed.

```bash
scripts/ios-job.sh                                  # snapshots: every screen, light and dark
scripts/ios-job.sh snapshots:Approve                # one screen
scripts/ios-job.sh unit:GrantLogicTests             # one unit-test class (or unit:Class/testMethod)
scripts/ios-job.sh unit                             # all of ReinsTests
scripts/ios-job.sh build                            # does it build and link
scripts/ios-job.sh ui:SoundsScreenUITests           # one UI test class, only for a flow you changed
scripts/ios-job.sh unit:GrantLogicTests,snapshots:Trust   # one build, then both
```

| Option | What it does |
|---|---|
| `--device "iPhone 17 Pro"` | Picks the simulator device type. The snapshots are 390×844 whatever the device. |
| `--no-wait` | Prints the run and returns. |
| `--watch <run id>` | Waits for a run that is already started, and downloads it. |
| `--mac` | Runs the job on the user's Mac (see [the Mac](#the-mac---mac)). |

The result folder has:
- `summary.txt` and `summary.json`: each step with its time and status, and the failing tests with their messages;
- `snapshots/*.png`;
- `logs/`: `build.log` and one log per slice;
- `results/`: an `.xcresult` and a JSON summary per slice.

On GitHub the job is `.github/workflows/ios-job.yml`, and you can also start it from the Actions tab:
- Its inputs are `ref`, `run`, `device` and `id`.
- Asking again for the same slices on the same branch cancels the older run.
- Different slices and branches run side by side, five macOS jobs at most for the whole account, shared with CI.

### Snapshot tests

`ios/ReinsTests/ScreenSnapshotTests.swift` is the iOS counterpart of Android's `ScreenshotsTest`. It renders these
screens with the demo core, in light and dark, at the boards' 390×844 (`design/next/iphone/previews/`):
- Activity: waiting and quiet;
- Approve: a git push;
- Trust;
- Autopilot: Bypass running, a profile, and Try it;
- Connections;
- Vault;
- Settings;
- the System gallery.

How it works:
- Each screen is a SwiftUI view in a window of the test host, drawn after its data loads. There is no UI-test app
  launch.
- There are no reference images, so the tests never fail on a pixel. They fail only when a screen cannot be drawn.
- They are skipped unless `REINS_SNAPSHOTS` names a directory. `ios-job.sh` sets it. With `xcodebuild test` on a Mac,
  set `TEST_RUNNER_REINS_SNAPSHOTS=/some/dir`.
- To add a screen, add a `test<Name>` method. `snapshots:<Name>` then runs it alone.

`ImageRenderer` was not used, because it draws a placeholder for every view that UIKit backs: scroll views,
navigation stacks, lists and text fields. That is most of a screen. swift-snapshot-testing was not used either: what
it adds over this window is reference-image comparison, which these tests do without.

## What makes it fast

**The Rust core is prebuilt.**
- `ios/scripts/core-cache.sh` hashes the core's inputs as they are on disk: the three crates, `Cargo.toml`,
  `Cargo.lock`, the toolchain, `.cargo/` and `build-core.sh`.
- On main, `.github/workflows/ios-core.yml` builds a missing simulator or device core once per hash. It attests the
  core and uploads it to the rolling `ios-core-cache` prerelease (`core-<hash>-<platform>.tar.gz`), keeping the
  newest twelve per platform.
- The iOS job, CI's iOS core job and the Mac (`core-cache.sh ensure iphonesimulator`) download it and verify it (see
  [Trust](#trust)). They build the core only when no verified build exists for that hash.
- A branch that changes the core builds its own core in each job, as an artifact of that run, until the change
  reaches main.
- It is a release asset rather than an Actions cache:
  - Actions caches are scoped to a branch and main, and are evicted after 7 days unused or past 10 GB.
  - Actions caches cannot be downloaded from a Mac or this PC.
  - Release assets are public and permanent.
- The TestFlight archive builds its own Release core.

### Trust

A prebuilt core is a binary linked into the app, so nothing installs one it cannot trace to a build of main:
- **Only main publishes.** The publish job runs only when the run is on `refs/heads/main` and builds the run's own
  commit. A branch's job, a dispatch with another `ref`, or a pull request builds the core for that run alone, with a
  read-only token.
- **The publish job runs no repository code.** It downloads the archive the macOS build job made, checks its name,
  attests it with `actions/attest-build-provenance`, and uploads it with an inline script. It does no checkout.
- **An asset is never replaced.** A name that is already published is skipped, not overwritten.
- **Every download is verified.** `core-cache.sh fetch` runs `gh attestation verify`. It requires:
  - the certificate identity of `.github/workflows/ios-core.yml@refs/heads/main` in `katulevskiy/reins`;
  - the source ref `refs/heads/main`;
  - a GitHub-hosted runner.

  If `gh` is missing, or the check fails, it installs nothing, and `ensure` builds the core locally.
- **The bindings must match.** A prebuilt core whose Swift bindings differ from `ios/Core/Generated/reins_core.swift`
  is refused. Fix it by running `ios/scripts/build-core.sh` and committing its output.
- **Caches are written only for main's own commit.** The Rust build cache in `ios-core.yml` is saved only for that
  commit. The job's DerivedData cache is saved only when the job built the run's own commit, never for a dispatched
  `ref`.

**The app is built once per job, and tested many times.**
- `ios/scripts/ios-job-run.sh` runs one `xcodebuild build-for-testing`.
- It then runs one `test-without-building -only-testing:` per slice.
- The settings are Debug, simulator, only the active architecture, `dwarf` (no dSYM), no index store, and Swift
  compilation caching (`COMPILATION_CACHE_ENABLE_CACHING`).

**DerivedData is kept between runs.**
- On GitHub it is an Actions cache keyed on the Xcode version and the project and package pins. The branch's last
  cache is used first, then main's, and one cache is kept per branch.
- On the Mac it is a fixed folder per checkout: `~/Library/Developer/Xcode/DerivedData/Reins-job-<checkout>-<hash>`.

**One simulator.**
- A simulator named "Reins Job" is created once and reused.
- It boots while the app builds.

## Timings

Measured on GitHub's `xcode-27` runner (M1, 3 cores, 7 GB) on 2026-10-26/27. See the pull request that added this
page for the run ids.

| | Before (the Mac, by hand) | Job, cold | Job, warm |
|---|---|---|---|
| Check one screen | 20–50 min (core, Xcode build, XCUITest launch) | TBD | TBD |
| Run one unit-test class | 20–50 min | TBD | TBD |
| Build after a Swift-only change | 5–10 min, plus the core when the shared one was stale | TBD | TBD |

## The Mac (`--mac`)

The same job can run on the user's Mac over SSH instead of GitHub's runners:
- The Mac fetches the pushed commit into a checkout kept for jobs, and runs `ios/scripts/ios-job-run.sh` there.
- It runs one job at a time. A second job waits for the first, and the lock also covers the checkout.
- It uses one simulator, and a DerivedData folder that stays between jobs.
- The results are copied back to `~/Documents/dev/tmp/ios-jobs/mac-<time>/`.

This path is opt-in, and needs this one-time setup:

1. On the Mac, turn on System Settings > General > Sharing > Remote Login, for your user only.
2. On the Mac, make a checkout that only jobs use, so nobody's work is ever switched under them:
   `git clone https://github.com/katulevskiy/reins ~/reins-jobs`. A `git worktree add ~/reins-jobs` of an existing
   checkout also works.
3. On the Mac, make sure `xcodebuild` finds the Xcode to use. If it is not the selected one, set
   `REINS_MAC_DEVELOPER_DIR` on the PC, for example `/Applications/Xcode-27.1-beta.app/Contents/Developer`.
   `rustup` only matters when no prebuilt core exists for a hash.
4. On the PC, give the Mac this PC's key:
   ```bash
   ssh-keygen -t ed25519 -f ~/.ssh/reins-mac -C "reins jobs"   # once
   ssh-copy-id -i ~/.ssh/reins-mac.pub you@your-mac.local
   ```
   Then add a `Host` entry to `~/.ssh/config` with that `IdentityFile`. `ssh you@your-mac.local true` must work with
   no prompt (`BatchMode`).
5. On the PC: `export REINS_MAC_SSH=you@your-mac.local`. Set `REINS_MAC_REPO` if the checkout is not `~/reins-jobs`.

You must be logged in on the Mac, because the simulator runs in your GUI session.

## UI tests in CI

`.github/workflows/ios.yml` runs on pull requests, main and nightly:
- One build, then a `unit` shard (ReinsTests) and five UI shards.
- The `iOS` check gates on the core, the build and the unit shard.
- The UI shards continue on error. A failing UI test shows as a warning on a green job, with its `.xcresult` as an
  artifact. It never holds a merge.
- Releases wait only for `ci.yml` (`scripts/wait-for-ci.sh`), so iOS UI tests never hold a release either.

`scripts/test-ci-routing.py` checks all of this.

Agents never gate a pull request on a local UI test run. If a flow needs checking:
- run its class with `scripts/ios-job.sh ui:Class`;
- or let CI run it, and read the warning if it fails.

## Runner configuration

The macOS runner is set in one place: these repository variables, read by `ios.yml`, `ios-job.yml` and `ios-core.yml`.
A self-hosted Mac or a third-party macOS runner can be dropped in by changing them.

| Variable | Default |
|---|---|
| `IOS_RUNNER` | `xcode-27`: GitHub's free macOS image with Xcode 27, on this public repository |
| `IOS_DEVELOPER_DIR` | `/Applications/Xcode_27.0.app/Contents/Developer` |
| `IOS_XCODE_VERSION` | `27.0` (CI checks it) |

The TestFlight archive in `release.yml` stays on `xcode-27` with Xcode 27.0.
