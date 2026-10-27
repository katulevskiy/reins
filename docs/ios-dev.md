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
  commit. The iOS job writes no cache at all; it only restores the Swift cache that `ios.yml` saves from main.

**The app is built once per job, and tested many times.**
- `ios/scripts/ios-job-run.sh` runs one `xcodebuild build-for-testing`.
- It then runs one `test-without-building -only-testing:` per slice.
- The settings are Debug, simulator, only the active architecture, `dwarf` (no dSYM), no index store, and Swift
  compilation caching (`COMPILATION_CACHE_ENABLE_CACHING`).

**Compiled Swift is reused.**
- On GitHub the job restores the Swift compilation cache, module cache and packages that `ios.yml` saves from main,
  with the same paths and build settings. Every module the branch did not change is a cache hit: the Swift compile
  of a job took 41 s of task time instead of 422 s.
- On GitHub a branch's own DerivedData is not kept. It was tried, and it made the next job slower (660 s against
  426 s):
  - a fresh checkout gives every file a new mtime, so Xcode recompiles all of them anyway;
  - the restored incremental state splits the compile jobs differently from the jobs main's cache holds, so the cache
    then misses;
  - the repository's Actions caches sit at GitHub's 10 GB limit, so a branch's cache was evicted within the hour.
- On the Mac, DerivedData is a fixed folder per checkout (`~/Library/Developer/Xcode/DerivedData/Reins-job-<checkout>-<hash>`)
  that stays between jobs, so a Swift-only change builds incrementally.

**One simulator.**
- A simulator named "Reins Job" is created once and reused.
- On the Mac it boots while the app builds.
- On a runner with 4 cores or fewer it boots after the build. On GitHub's 3 cores, a first boot beside the compiler
  made the same cached build take 426 s, against about 160 s for ios.yml's build alone.

## Timings

Measured on GitHub's `xcode-27` runner (M1, 3 cores, 7 GB) on 2026-10-27. These are runs 40420651747 (cold),
40438794679 (warm) and 40442183045 (a restored DerivedData, which is not used any more). The times are the job's
own steps. Waiting for a free macOS runner comes on top.

| Step | Before | Cold job | Warm job |
|---|---|---|---|
| Rust core | 9–11 min to build, or the Mac's shared build | 9 min to build (first time for a hash) | 1–2 s: download and verify |
| Build for testing | the Mac: 5–10 min, more under load | 14 min (841 s) | 7 min (426 s) |
| Simulator | the Mac: one per agent, five at once | 2–5 min, alongside the build | 2 min, alongside the build |
| One screen (`snapshots:Approve`) | an XCUITest launch per screen | 51–72 s | 51–72 s |
| All 11 screens, light and dark | — | 97–140 s | 97–140 s |
| One unit-test class | in a full ReinsTests run | 14–28 s | 14–28 s |

End to end, from push to PNGs:

| | Before (the Mac) | GitHub job, warm |
|---|---|---|
| Check one screen | 20–50 min | about 9 min |
| Run one unit-test class | 20–50 min | about 8.5 min |
| Build after a Swift-only change | 5–10 min | about 7 min |

These warm runs booted the simulator beside the build. The job now boots it after the build on GitHub. That should
bring a warm build down to about 3 minutes, since ios.yml's build job takes 2 min 41 s with the same cache. A job has
not measured this yet.

Each GitHub job adds the queue for a macOS runner: 0–20 minutes, longest while a release builds. The Mac does none of
this work, and runs no simulators for it.

The `--mac` path has not been measured yet, because SSH to the Mac is not set up. It keeps DerivedData and a booted
simulator between jobs, so a Swift-only change should be an incremental build plus about a minute of tests.

### Limits

- **The GitHub path takes the load off the Mac, but it is not 10x.**
  - A fresh runner compiles every module the branch changed.
  - It boots a simulator from scratch, which takes 2–5 minutes.
  - It waits for a free macOS runner.

  The 10x loop is `ios-typecheck.sh` on Linux in seconds, plus `--mac` with a warm DerivedData folder and a booted
  simulator.
- **Five macOS jobs at once,** shared with CI (`ios.yml`, the desktop macOS jobs) and releases. Jobs queue when they
  are busy.
- **Before a core change reaches main,** each job on that branch builds the core itself (about 9 minutes), because
  only main publishes cores.
- **The snapshots are not the boards.**
  - They draw no status bar, and they use the demo data.
  - Relative times ("1m", "Yesterday") follow the real clock.
  - Some live effects (blur, Liquid Glass) look as they do in a window drawn off-screen.
  - Nothing compares them with a reference image.
- **UI tests run in CI only.** `ui:Class` runs one class, but a UI test result never blocks a merge.

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
- A pull request: the core, one build, then the `unit` shard (ReinsTests). No UI shard.
- A push to main (a newer push cancels the older run), the nightly and a hand run (`gh workflow run ios.yml --ref
  <branch>`, for a pull request whose UI needs a look): the same, plus the five UI shards.
- The `iOS` check gates on the core, the build and the unit shard.
- The UI shards continue on error. A failing UI test shows as a warning on a green job, with its `.xcresult` as an
  artifact. It never holds a merge.
- Releases wait only for `ci.yml` (`scripts/wait-for-ci.sh`), so iOS UI tests never hold a release either.
- Before the core and before the tests, a Linux job waits while a release waits for its macOS runner
  (`.github/workflows/macos-gate.yml`, at most an hour). `ios-core.yml`, `ios-job.yml` and `ci.yml`'s macOS job
  wait the same way.

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
