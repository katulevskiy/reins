# CI

Pull requests and `main` go through one workflow, [`ci.yml`](../../.github/workflows/ci.yml). Its jobs, in order:

1. **plan** ([`scripts/ci/plan.py`](../../scripts/ci/plan.py)) picks the suites the run needs:
   - on a pull request, the suites whose inputs the change touches;
   - on `main`, every suite, unless `main` already passed it on the same inputs.

   It drops the suites that already passed on the same inputs (see [Reuse](#reuse)) and counts the macOS jobs the rest
   will start. If anything goes wrong, it runs every suite and reuses nothing.
2. **macOS admission** ([`macos-gate.yml`](../../.github/workflows/macos-gate.yml), running
   [`scripts/ci/admit.py`](../../scripts/ci/admit.py)) waits until macOS runners are free for all of the run's macOS
   jobs.
   - Linux and Windows jobs don't wait for it.
   - Pushes to `main` don't call it, because their release waits for them.
   - The macOS jobs run even if it fails: it is a queue, not a check.
3. **The suites** run.
4. **CI result** passes when the plan succeeded and no suite failed or was cancelled. A skipped or reused suite counts
   as passed. This is the one check to require.

Releases wait for `ci.yml` on their commit ([`scripts/wait-for-ci.sh`](../../scripts/wait-for-ci.sh)), never for
`ios.yml`. `ci.yml`'s iOS job (the build and ReinsTests) runs on pushes to `main` too, so the release needs it. `ios.yml`
runs iOS on `main`, at night and by hand, with the UI tests, in the background (see [Admission](#admission)).

## Measurements (2026-10-16 to 2026-10-27)

Taken from the Actions API before this design: 638 runs of `ci.yml`, `ios.yml`, `ios-core.yml`, `ios-job.yml` and
`release.yml`, and their 7,306 jobs. "Today" is the last 22 hours (2026-10-26 12:00 to 2026-10-27 10:30 UTC), after
#212 and #227.

### Where the time goes

| | median | p90 |
|---|---|---|
| Pull request push → all checks done (`ci.yml` + `ios.yml`), 11 days | 9 min | 63 min |
| ... today | 11 min | 107 min |
| `main` push → all checks done, 11 days | 15 min | 125 min |
| ... today | 88 min | 153 min |
| `ci.yml` alone, pull request (wall) | 10 min | 16 min (today 29) |
| `ios.yml` pull request: time queued on the core → build → unit chain | 0.2 min | 31 min (today 66) |
| `ios.yml` pull request: time running on that chain | 4 min | 14 min |
| `ios.yml` `main`: queued on the chain / running on it | 23 / 10 min | 50 / 14 min |
| Linux job queue | 0.03 min | 0.05 min (p99 0.6) |
| Windows job queue | 0.03 min | 0.05 min (p99 0.5) |
| macOS job queue, pull requests | 0.1 min | 22 min (today 36) |
| macOS job queue, `main` | 14 min | 50 min (today 22 / 59) |

- **Only macOS queues.** Linux and Windows jobs start within seconds (p99 under a minute).
- The account runs five macOS jobs at once. During the minutes any macOS job ran or waited, all five were busy **51%
  of the time** over 11 days, and **97% today** (608 of 624 minutes). Up to 26 macOS jobs were queued at once.
- The iOS check is a chain of three macOS jobs: the core, the build and the unit tests. Each joins the back of the
  queue when the one before it ends, so under load a pull request waits three times.

### Runner time per push

Pushes whose runs weren't cancelled. Before #227 the UI shards also ran on pull requests, which is most of "iOS
tests" here.

| | Pull request push | `main` push |
|---|---|---|
| Jobs (non-trivial) | 14 (p90 23) | 14 (p90 23) |
| Runner-minutes | 39 (p90 154) | 41 (p90 161) |
| ... macOS | 34 | 36 (today 72) |
| ... Linux | 26 | 25 |
| ... Windows | 4 | 4 |

The biggest single items per push: iOS tests 26 min on pull requests and 29 on `main` (today 65 on `main`: five UI
shards of 14 to 28 min each), server and core tests 6, the Windows desktop job 4, the Android emulator smoke test 2 to
6.

macOS minutes over the 11 days by workflow: `ios.yml` pull requests 4,192, `ios.yml` on `main` 2,014,
`release.yml` 1,274, `ci.yml` pull requests 1,107, the `ios.yml` nightly 584, `ios-job.yml` 137.

### Cancelled runs

| | cancelled | runner-minutes spent in them |
|---|---|---|
| `ci.yml` pull requests | 74 of 263 (28%) | 2,320 |
| `ios.yml` pull requests | 120 of 193 (62%, mostly pending runs replaced) | 886 |
| `release.yml` on `main` | 30 of 62 (48%, superseded by a newer push) | 1,516, of which 444 macOS |

- 5% of pull request pushes were followed by another within a minute, 14% within five minutes.
- 42% of `main` pushes were followed by another within ten minutes.

### Releases

| | median | p90 |
|---|---|---|
| Merge → published, 11 days | 20 min | 49 min |
| ... today | 26 min | 50 min |
| Merge → TestFlight, 11 days | 37 min | 72 min |
| ... today | 40 min | 74 min |

Publication waited for CI on its commit for 0 min (median; at most 5). The release's own build is the critical path,
not CI. #227 made it one macOS job ("macOS builds", 24 min).

### Caches

10.9 GB in 16 entries, all on `main`, over the 10 GB limit. The entries ci.yml and ios.yml restore most were missing:
the iOS Swift cache, the simulator core and the Android debug natives had been evicted. #229 brings the budget to
about 8.4 GB and prunes superseded entries.

### The planner over the last 30 merged pull requests

The dry run replays `plan.py` over recorded history. Reuse counts only passes the recorded runs really had.

- **Pull requests:** the 68 pushes of the last 30 merged pull requests, each against `main` as it was then
  (`git merge-tree`).
- **`main`:** the 109 pushes since 2026-10-15. Every suite runs unless `main` passed it on the same inputs.

Per push, mean:

| | jobs | macOS jobs | Linux and Windows runner-min | macOS runner-min |
|---|---|---|---|---|
| Pull request, before (`ci.yml` + `ios.yml`) | 16.7 | 2.9 | 31 | 12.0 |
| Pull request, selection from the Cargo graph | 11.9 | 1.2 | 28 | 10.9 |
| ... and the fast tier | 11.9 | 1.2 | 28 | 10.9 |
| ... and reuse | **8.7** | **1.0** | **17** | **10.2** |
| `main`, before (selection by the push's diff) | 10.9 | 0.5 | 31 | 2.3 |
| `main`, after (every suite, unless passed on the same inputs) | 9.8 | 0.4 | 28 | 1.7 |

- **macOS jobs per pull request push drop from 2.9 to 1.0.**
  - The iOS chain becomes one job, so a pull request queues once, not three times.
  - The desktop macOS job stops running for `reins-core` changes: the desktop crates don't depend on it.
- **macOS minutes barely drop**, because the iOS build and tests are the work itself. The gain is in waiting, and in
  who waits (see [Admission](#admission)).
- **Reuse skipped at least one suite on 36 of 68 pushes.** Per suite: Linux packages 24, Android 18, server and core
  tests 14, clippy 14, each of the three desktop suites 11.
- **The fast tier changed nothing in this sample.** No pull request changed `reins-proto` or `reins-policy` without
  also changing a desktop crate.
- **`main` runs slightly less, and is safer.** Before, a red suite was skipped by the next push if that push didn't
  touch it, and its release shipped the breakage. Now it runs again on every push until it passes.

## Which suites run

On a pull request, a suite runs when a changed file is one of its inputs. The inputs are the same list its
[reuse](#reuse) fingerprint hashes, so what selects a suite and what decides whether its pass still holds can't drift
apart. On `main`, every suite runs unless `main` recorded a pass on its exact inputs.

| Suite | Jobs | Inputs besides `ci.yml`, `macos-gate.yml` and `scripts/ci` |
|---|---|---|
| `rust`, `clippy` | Server and core tests (nextest + doctests), clippy | every workspace package but the desktop app (read from the manifests, so a new member is in), with the lockfile, toolchain and cargo config |
| `replay` | Replay gate | `reins-replay` and its dependencies |
| `packages` | Linux packages and repositories | `scripts/package`, the packaging test, the installers, `release.yml`, the app's packaging files |
| `desktop-linux` | Desktop UI on Linux | `reins-desktop-app` and its dependencies |
| `desktop-macos` | Desktop app on macOS | `reins-desktop`, `reins-desktop-app` and their dependencies |
| `desktop-windows` | Desktop app on Windows | the same, and `scripts/install.ps1` |
| `android` | natives (3), unit tests (2), debug APK, minified smoke test | `reins-core` and its dependencies, `android/`, the native build script |
| `ios` | iOS (one macOS job: the core, the build, ReinsTests) | `reins-core` and its dependencies, `ios/`, `scripts/ios-ui-plan.py`, `ios.yml`, `ios-core.yml`, `ios-job.yml` |

- **Rust inputs come from the Cargo graph:** each suite's packages, and every workspace package they depend on (path
  dependencies of every kind). `scripts/test-ci-reuse.py` checks that every member `cargo metadata` lists is built by
  some suite.
- **Files outside those packages that the Rust sources name are inputs too.** For example:
  - `include_bytes!("../../../ios/Reins/Resources/Sounds/...")`;
  - `CARGO_MANIFEST_DIR` joined with `"../.."` and then a path constant.

  A named path counts whether it exists or not, so deleting or renaming a file that a source still names selects the
  suite. The sources are scanned on every run, and the plan summary lists the files found.
- **Build files:** `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` and `.cargo/` run every Rust suite. So does a
  changed Rust build file that no suite's inputs cover.
- **CI itself:** a change to `ci.yml`, `macos-gate.yml` or `scripts/ci/` runs everything.
- **Docs:** documentation, `design/` and `marketing/` run nothing but the checks job, which always runs.

### Fast tier (pull requests)

- macOS is the only scarce runner, so `desktop-macos` is the only suite the fast tier holds back.
- On a pull request it runs when:
  - a desktop crate changes;
  - the lockfile, toolchain or cargo config changes;
  - a change in its other crates (`reins-proto`, `reins-policy`) looks macOS specific: a macOS file name, or changed
    code (not comments) with `target_os`, `cfg(unix)`, `objc`, Cocoa, `core_foundation`, ...
- `main` runs it unless it passed on the same inputs.
- To run it on a pull request anyway, add the `ci:full` label, then push or re-run CI. That also turns reuse off for
  that pull request.
- The repository variable `CI_PR_TIER=full` turns the fast tier off for every pull request, but keeps reuse.

### Flaky tests

- **The Android minified Trust smoke test retries once** (`scripts/android-trust-smoke.sh`).
  - `TrustReleaseTest.trustSurvivesAsyncRefreshesAndItsSheetsStillWork` sometimes fails on the CI emulator with the
    app not in front: 9 pull request failures in 11 days, while the same code passed on `main`.
  - A failed run warns and runs again after waking and unlocking the screen.
  - A real regression fails both times, so the job still gates pull requests and `main`.

### iOS

- Pull requests, manual runs and pushes to `main` of `ci.yml` run iOS as one macOS job:
  1. the prebuilt core (`ios-core.yml` publishes one per content hash), or a fresh build of it;
  2. the check of the committed bindings;
  3. the build for testing;
  4. ReinsTests.
- That replaces the three-job chain, which queued three times.
- On `main` that job is part of **CI result**, so the release waits for it: a release publishes only a commit whose
  iOS build and ReinsTests passed (or passed on the same inputs). It skips admission, like the rest of `main`'s CI,
  and `main`'s runs are never cancelled.
- On `main`, `ios.yml` also runs the core, the build, ReinsTests and the five UI shards in the background. The UI
  shards never gate a merge or a release.

## Reuse

After a new push, a suite runs again only if something it reads changed. That is checked by content, not by what the
push touched:

- a suite reruns when `main` changed something it reads;
- a suite is skipped when the push only touched code it doesn't read.

For each suite, `plan` hashes the inputs in the tree being tested ([`scripts/ci/reuse.py`](../../scripts/ci/reuse.py)).
For a pull request, that tree is its merge with the current `main`.

- The hash covers each input file's `(path, mode, git object id)`, so an executable bit counts.
- It also covers the suite's variant: for iOS, the repository variables `IOS_RUNNER`, `IOS_XCODE_VERSION` and
  `IOS_DEVELOPER_DIR`.

When a job passes, its last step saves a tiny Actions cache entry: `ci-pass-v1-<suite>-<hash>-<job>`. `plan` skips a
suite when every one of its jobs has such an entry on a ref it trusts:

| Run | Reuses passes from |
|---|---|
| Pull request | its own earlier runs, and `main` |
| `main` | `main` only |
| Manual run, schedule, `ci:full` label | nothing |

Reuse fails closed:

- If a pull request changes a Rust build file that no suite's inputs cover, nothing is reused, and every Rust suite
  runs.
- Every use of `CARGO_MANIFEST_DIR` must take one of these forms:
  - `.join("<literal>")`;
  - `.join("../..").join(CONSTANT)`, with `CONSTANT` a string const of the same file;
  - `concat!(env!("CARGO_MANIFEST_DIR"), "<literal>")`.

  Any other form (a variable, `.parent()`, a runtime `env::var`, a bare `"../.."`) makes the suite count the whole
  repository as its input.
- One such use is declared instead (`KNOWN_PARENT_READS`): `reins-e2e` builds the server from the repository root, so
  it reads the server package. `scripts/test-ci-reuse.py` checks that the file still has exactly that one use.

A pull request could fake a pass in its own runs, but it could just as well edit its own workflows. No other ref sees
its cache entries.

Limits:

- A path assembled from variables isn't found by the scan. Write such references as one `../` literal, as a join of
  a `../` literal and a path constant, or add the path to `SUITE_INPUTS`.
- A test that depends on the network or on timing and passed by luck stays passed until its inputs change. Re-run with
  `ci:full` to check everything again.

## Admission

GitHub starts a job as soon as a runner frees up, so a run's macOS jobs trickle in behind everyone else's. Every macOS
job of `ci.yml` (on pull requests and manual runs), `ios.yml`, `ios-core.yml` and `ios-job.yml` needs a gate job
(`macos-gate.yml`). The gate waits until macOS runners are free for all of the jobs it gates. Who goes first:

1. **Releases.**
   - A release run whose "macOS builds" job doesn't exist yet holds one runner.
   - Once that job exists, it is busy like any other job, queued or running.
   - Release runs are never gated.
   - A queued TestFlight upload is busy too, so nothing admitted after it can get ahead of it.
2. **`main`'s CI** calls no gate: the release of that commit waits for it. Its macOS jobs still count in everyone
   else's budget.
3. **The oldest `ci:lock`**, then **`ci:priority`** (and any later `ci:lock`).
4. **Everyone else**: pull requests, iOS jobs and core builds, first come, first served.
5. **Background**: `ios.yml` on `main` and at night.
   - It goes only when nobody else waits, and it leaves one runner free.
   - The UI shards of a push to `main` check again once they have a runner (`admit.py --yield`). They skip their
     tests with a warning while a release, a queued job or a waiting run wants a macOS runner.
   - Under sustained load, a background run may rarely be admitted, or only when `MAX_WAIT_MINUTES` lets it go.
   - The nightly never steps aside and runs every test.

A waiter goes when this fits in the budget (`CI_BUDGET_MACOS`, 5):

- the busy macOS jobs;
- what admitted runs reserved but haven't created yet;
- the releases' runners;
- what the waiters ahead of it need;
- its own request.

Nobody is overtaken on the runners they wait for. A pull request's run asks for at most two runners: the desktop
macOS job and iOS. Both start together.

All the state lives in the GitHub API, so admission works with the read-only token that pull requests from forks get:

- The gate's job name carries the request, for example `admit [macos=2] #232` or `admit [macos=1 bg]`.
- Busy runners are the macOS jobs (labels `macos-*` and `xcode-*`) that are queued or running in any active run.
- A request read from another run is capped at three, since a pull request can edit its own copy of the workflows.

### Priority and locks

Adding a label needs the triage role or above, so GitHub decides who can use these. A fork's author can't label their
own pull request.

- **`ci:priority`**: the pull request's runs go to the front of the queue, behind releases and the locked pull
  request.
- **`ci:lock`**: while the pull request is open, two macOS runners are kept for it, whether or not it is running
  anything. Its runs go as soon as their runners are free.
  - Only the oldest lock holds. A pull request labelled later counts as `ci:priority` until the first lock ends.
  - The lock ends when the label is removed, when the pull request is merged or closed, or when `CI_LOCK_HOURS` (6)
    have passed since the label was added. Add the label again to extend.
- A run claims its pull request in its gate's name. The claim only counts when that pull request's head is the run's
  commit.
- **CI emergency** ([`ci-emergency.yml`](../../.github/workflows/ci-emergency.yml)) is a workflow you start by hand
  from the Actions tab, with a pull request number. It:
  - adds `ci:lock` to the pull request;
  - cancels every other pull request's CI run;
  - cancels `ios.yml`'s background runs, which gate nothing (turn this off with `cancel-background`);
  - re-runs the pull request's own CI if none is running and the last run didn't pass.

  It never cancels `main`'s CI or a release. If the label change or the re-run fails, it warns and carries on.

### Closed pull requests

When a pull request is closed or merged, `cancel-closed-pr-runs.yml` cancels its runs that are still queued or
running: CI, iOS, the iOS core cache and the agents' iOS jobs on its branch (`scripts/ci/cancel-closed.py`).

- A bundle merge closes many pull requests at once, and their old runs had kept taking the macOS runners a release
  needed.
- Only that job has `actions: write`.
- It never touches a push run (`main`'s CI) or a release, nor a fork's branch of the same name.
- Pull requests from forks are skipped: their read-only token can't cancel anything.

### Never blocking CI

Admission lets the run go on anyway:

- after `MAX_WAIT_MINUTES` (180);
- after five API errors in a row, or any unexpected error;
- when the rate limit resets more than ten minutes away;
- when fewer than 500 API requests are left. The rest is kept for the release's wait for CI.

The `GITHUB_TOKEN` gets 1,000 API requests an hour per repository (more on some plans). To stay under that, each
poll:

- lists the repository's latest runs once, plus a conditional read of any run it saw active before that has dropped
  off that page;
- reads the jobs only of the runs of the workflows with macOS jobs, and only when such a run changed;
- uses conditional requests, so a `304` doesn't count;
- comes at least a minute after the last: a minute at the front of the queue, a minute more per run ahead, five at
  most; background runs poll every two minutes or more, and everyone every five when fewer than 1,000 requests are
  left.

`scripts/wait-for-ci.sh`, the release's wait for CI, waits out a spent rate limit until its reset (three times at
most) instead of giving up, and otherwise polls for up to 90 minutes (CI on `main` includes the iOS job). The
publish job allows for that.

Pull request runs that need macOS wait `CI_DEBOUNCE_SECONDS` (60) first. A newer push cancels the run, so a quick
series of pushes only takes a macOS runner for the last one.

### Repository variables

| Variable | Default | |
|---|---|---|
| `CI_BUDGET_MACOS` | 5 | macOS runners all runs may hold at once |
| `CI_ADMISSION` | on | `off`: every gate passes at once |
| `CI_DEBOUNCE_SECONDS` | 60 | how long a pull request run waits for a newer push before asking |
| `CI_PR_TIER` | fast | `full`: no fast tier on pull requests (reuse stays on) |
| `CI_LOCK_HOURS` | 6 | how long a `ci:lock` label holds runners |

## What was adopted from Zeron's CI (zeronsh/zeron#1077), and what wasn't

| Strategy | Here |
|---|---|
| Measure first | Yes: above |
| One entry workflow, `plan`, a single `CI result` | Yes. iOS on pull requests moved into `ci.yml`. On `main`, `ci.yml` runs the iOS build and ReinsTests too, so releases wait for them; `ios.yml` keeps the UI shards in the background. |
| Admission: all of a run's runners free, first come, first served, reservation in the job name, fails open | For macOS only: Linux and Windows never queue here. Releases come first, `main`'s CI is never gated, and background work goes last and steps aside. |
| Fast PR tier | For `desktop-macos` only, the one platform suite on a scarce runner. Windows and Android run as on `main`. |
| Reuse on unchanged inputs, failing closed | Yes, the same scheme. The inputs also drive selection, and come from the Cargo graph. On `main` the fingerprint, not the diff, decides. |
| Priority and lock labels, emergency workflow | Yes, in macOS terms. The emergency workflow never cancels `main` or a release. |
| One nextest build instead of several `cargo test` builds | Already the case for server and core tests. The iOS chain became one job instead. |
| Caches saved only from `main` | Already the case (and #229) |
| Cancel superseded runs | Already the case for pull requests everywhere. `main`'s CI is never cancelled: each commit's release waits for it. |
| Debounce | For pull request runs that need macOS only (60 s, in the gate). Linux feedback isn't delayed. |
| Merge queue | Not used here |

## Locally

```sh
python3 scripts/test-ci-plan.py && python3 scripts/test-ci-admit.py && python3 scripts/test-ci-reuse.py
```
