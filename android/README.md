# Reins for Android

A thin Kotlin + Jetpack Compose shell around the Rust `rewarden-core` library (`crates/rewarden-core`), which owns all
logic, networking, grant evaluation and encrypted storage. Kotlin only does UI, the Android Keystore, Google's
`AuthorizationClient` (Gmail tokens, on this phone only), FCM, WorkManager, notifications and `BiometricPrompt`.

## Build

Needs JDK 21, the Android SDK (platform 36, NDK `27.2.12479018`), Rust (`rustup`, targets `aarch64-linux-android`
and `x86_64-linux-android`) and `cargo-ndk`.

```bash
export ANDROID_HOME=$HOME/Android/Sdk
cd android
./gradlew assembleFullDebug                    # builds the Rust core (cargo-ndk) and generates the Kotlin bindings
./gradlew assembleFullRelease                  # fat-LTO Rust + R8; signed with the debug key for the MVP
./gradlew assembleFullDebug -Prewarden.rustProfile=release-low   # faster Rust build while iterating
```

Gradle drives `cargo ndk` and `uniffi-bindgen` itself (`CargoNdkTask`, `UniffiBindgenTask` in `app/build.gradle.kts`).
Nothing generated is committed. Native libraries are linked with 16 KB page alignment.

### Firebase (optional)

`google-services.json` is copied from `~/.config/rewarden/google-services.json` (or `-Prewarden.googleServicesJson=...`)
into `app/google-services.json` (git-ignored) when present. Without it the app builds and runs, gets no push
notifications, and relies on the foreground long-poll.

### Gmail

The app asks Google Play services for Gmail tokens (`gmail.readonly`, `gmail.send`). That needs an **Android OAuth
client** for package `dev.rewarden.android` and the signing certificate's SHA-1 in the Google Cloud project, plus a
configured consent screen. Until then the app shows "Gmail needs setup" and everything else works.

Any number of Google accounts can be added (Activity → Integrations → Gmail → Add account, which uses Android's own
account picker and then Google's consent screen for that account). Tokens are requested per account; removing an account
deletes its grants and revokes the app's access.

### Other integrations

Activity → Integrations lists every service. Telegram (own account: phone number, code, optional password), GitHub
(pasted token), Google Calendar / Contacts (like Gmail), the phone's own calendar, contacts and text messages (Android
permissions) and the password vault (master password once) are each added from their own screen; see
`docs/deployment.md` for what each needs. Telegram's credentials come from `rewarden.telegramApiId` /
`rewarden.telegramApiHash` in `~/.gradle/gradle.properties`.

### Icons and avatars

Provider logos in `app/src/main/assets/providers/` are the real brand SVGs (see `NOTICE.md` there), rasterised with
AndroidSVG. Connections without a known provider get a blobatar (https://github.com/Alain00/blobatar, MIT):
`design/blobatar/Blobatar.kt` is a line-for-line port of its renderer, and `BlobatarGoldenTest` checks it against the
library's own golden corpus (1000 names plus backdrops, by SHA-256 of the markup).

## Tests

```bash
./gradlew testFullDebugUnitTest      # JVM: policy/logic tests + Robolectric Compose flow tests (no device needed)
./gradlew connectedFullDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=dev.rewarden.android.RealCoreTest
scripts/device-smoke.sh          # real server + simulated AI on this machine, real core on the emulator
```

* `AppFlowTest` (Robolectric) drives every screen against an in-memory `FakeCore`: sign-in, 2FA, approvals (once,
  standing, public-domain warning), biometric fail-closed paths, pairing, deep-link spoofing, `FLAG_SECURE`, grants,
  activity.
* `RealCoreTest` (instrumented) loads the real Rust library and the real Keystore on a device.
* `LiveServerTest` + `scripts/device-smoke.sh` run the whole connection flow over `adb reverse`.
* Some emulator images crash `system_server` when it snapshots a task (`hasReadColorBufferDma` assertion), which is why
  the UI tests run on the JVM and the device tests avoid UI.

## Design review screenshots

```bash
./gradlew testFullDebugUnitTest --tests '*Screenshots*' -Drewarden.screenshots=/tmp/shots   # dark and light PNGs of every screen
```

Rendered through Robolectric with frozen clocks, so they are reproducible. `FLAG_SECURE` is off while testing
(`-Prewarden.secureScreens=true` turns it back on).

## Layout

```
app/src/main/java/dev/rewarden/android/
  ReinsApp, AppContainer, MainActivity
  core/       MainSafeCore (every core call on Dispatchers.IO), CoreProvider (test seam)
  platform/   KeystoreKeyWrapper, GmailAuthorizer (per account), Authenticator (BiometricPrompt), AppNotifier, GrantReminders
              ("ends soon" alarms), Foreground, FirebaseSupport
  push/       FCM service (not exported), expedited PushWorker, RegisterDeviceWorker
  sync/       foreground long-poll with backoff
  state/      AppState (pending, activity, grants, connections, unseen counter), DeviceStatusStore
  design/     palette, Geist type, components, action icons, provider logos + blobatars, countdown border, grant clocks
  ui/         activity (home tab + details), grants (tab, details, new grant), sheet (80% approval/pairing sheet),
              approval (pure grant builder, sheet content), settings (account, AI connections + icons, integrations),
              main (floating nav bar), signin, nav
```
