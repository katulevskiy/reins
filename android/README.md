# Reins for Android

A thin Kotlin + Jetpack Compose shell around the Rust `reins-core` library (`crates/reins-core`), which owns all
logic, networking, grant evaluation and encrypted storage. Kotlin only does UI, the Android Keystore, Google's
`AuthorizationClient` (Gmail tokens, on this phone only), FCM, WorkManager, notifications and `BiometricPrompt`.

## Build

Needs JDK 21, the Android SDK (platform 37, NDK `27.2.12479018`), Rust (`rustup`, targets `aarch64-linux-android`
and `x86_64-linux-android`) and `cargo-ndk`.

```bash
export ANDROID_HOME=$HOME/Android/Sdk
cd android
./gradlew assembleFullDebug                    # builds the Rust core (cargo-ndk) and generates the Kotlin bindings
./gradlew assembleFullRelease                  # fat-LTO Rust + R8; unsigned without explicit production signing
./gradlew assembleFullDebug -Preins.rustProfile=release-low   # faster Rust build while iterating
```

Gradle drives `cargo ndk` and `uniffi-bindgen` itself (`CargoNdkTask`, `UniffiBindgenTask` in `app/build.gradle.kts`).
Nothing generated is committed. Native libraries are linked with 16 KB page alignment.

For signed releases, supply `REINS_RELEASE_KEYSTORE`, `REINS_RELEASE_KEYSTORE_PASSWORD`,
`REINS_RELEASE_KEY_ALIAS`, and `REINS_RELEASE_KEY_PASSWORD` from private settings. The publishing script
`../scripts/release-android.sh` also accepts the four Android signing settings injected by Infisical and
checks the production certificate before building. Debug keys are used only for debug builds; they cannot be used
for store publication. See [signing setup](../CONTRIBUTING.md#maintainers-signing-the-apk-in-releases) and
[Google Play](PLAY_STORE.md).

### Firebase (optional)

`google-services.json` is copied from `~/.config/reins/google-services.json` (or `-Preins.googleServicesJson=...`)
into `app/google-services.json` (git-ignored) when present. Without it the app builds and runs, gets no push
notifications, and relies on the foreground long-poll.

### Gmail

The app asks Google Play services for Gmail tokens (`gmail.readonly`, `gmail.send`). That needs an **Android OAuth
client** for package `com.reins2fa.app` and the signing certificate's SHA-1 in the Google Cloud project, plus a
configured consent screen. Until then the app shows "Gmail needs setup" and everything else works.

Any number of Google accounts can be added (Activity → Integrations → Gmail → Add account, which uses Android's own
account picker and then Google's consent screen for that account). Tokens are requested per account; removing an account
deletes its grants and revokes the app's access.

### Other integrations

Activity → Integrations lists every service. Telegram (own account: phone number, code, optional password), GitHub
(pasted token), Google Calendar / Contacts (like Gmail), the phone's own calendar, contacts and text messages (Android
permissions) and the password vault (master password once) are each added from their own screen; see
`docs/deployment.md` for what each needs. Telegram's credentials come from `reins.telegramApiId` /
`reins.telegramApiHash` in `~/.gradle/gradle.properties`.

### Onboarding, pairing codes and links

Signed out, the app shows a welcome with "Continue" (`ui/signin`). It calls the core's `ssoBegin` for
`BuildConfig.DEFAULT_SERVER` (`reins.defaultServer`, default `https://app.reins2fa.com`) and opens the server's
WorkOS AuthKit sign-in page in a Custom Tab, preferring a browser with Custom Tabs support even if the default
browser has none. AuthKit presents the email, social and passkey methods enabled in its WorkOS environment.
Passkeys currently require WorkOS's hosted UI; Google OAuth also uses the browser session. Other servers can keep
their own SSO policy. The page sends the browser to
`com.reins2fa.app://sso-callback`, which `platform/SsoRedirectActivity` hands to `MainActivity` (closing the tab), and the
sign-in screen finishes it with `ssoFinish`. `platform/SsoSignIn` keeps the server, `state` and PKCE verifier in the
app's private storage while the page is open, so the sign-in still finishes when Android stopped the app meanwhile.
"Use another server" reveals the address field for self-hosters, with "Continue" for that server and, there only, the
email and master password forms ("Sign in", "Create account").

`ssoFinish` says whether this phone can open the account's keys. A new account (`Created`) or one whose secret this
phone keeps (`Unlocked`) finishes like a password sign-in: this phone becomes the approval device and the setup
follows. `Locked` (the keys are on another phone) shows the Unlock screen instead and does not register this phone:
"Ask my other phone" (`joinBegin` with `Build.MODEL`, the code shown large, `joinPoll` every 2 seconds) or "Enter
recovery code" (`unlockAccount`, which also takes the master password of an account made with one). The locked state
is kept in `state/DeviceStatusStore`, so a relaunch comes back to the Unlock screen; unlocking or signing out clears it.
On the approval device the request is a `PendingKind.JOIN` item (push `t=join`): its sheet shows the code the new
phone shows, and approving asks for biometrics first. Settings > Account shows the recovery code (after biometrics)
when the core has one for the account. Before normal app screens can appear, every passwordless account must
record its recovery code, check the acknowledgement, and type the final group from its written copy. Back, outside
taps, and copying the code cannot skip this gate. Restarting resumes it; only a server/secret fingerprint is persisted
in `state/RecoveryRecord`, so a WorkOS email change does not repeat the step and a new recovery code does. The core
remembers the authenticated account id to read the encrypted recovery secret offline after subsequent restarts.
Older installations may need one online refresh for that migration; a failure shows a retry screen.

A fresh sign-in from these screens then shows a short setup once per account (`state/OnboardingStore`):
notifications, "connect your computer" and the `/mcp` address for Claude.ai or ChatGPT. People who were signed in
before it existed never see it.

A computer pairs by the code it shows ("BCDF-GHJK", as a QR code of `https://app.reins2fa.com/pair?code=...`, or
`reins://pair?code=...`). The phone scans it with Google's code scanner (`play-services-code-scanner`: Play services shows
the camera, so the app needs no camera permission), or the user types it; `ui/pairing/PairingCode` reduces whatever
arrives to a well-formed code, the core's `pairingByCode` fetches the pairing from the user's own server, and the usual
pairing sheet confirms it. The same link opens the app directly (an App Link on `app.reins2fa.com/pair`, which needs the
server's `/.well-known/assetlinks.json` to list this package and its signing certificate's SHA-256; and the `reins`
scheme). Settings > AI connections > Connect a computer does the same later.

### Icons and avatars

Provider logos in `app/src/main/assets/providers/` are the real brand SVGs (see `NOTICE.md` there), rasterised with
AndroidSVG. Connections without a known provider get a blobatar (https://github.com/Alain00/blobatar, MIT):
`design/blobatar/Blobatar.kt` is a line-for-line port of its renderer, and `BlobatarGoldenTest` checks it against the
library's own golden corpus (1000 names plus backdrops, by SHA-256 of the markup).

## Tests

```bash
./gradlew testFullDebugUnitTest      # JVM: policy/logic tests + Robolectric Compose flow tests (no device needed)
./gradlew connectedFullDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=dev.reins.android.RealCoreTest
scripts/device-smoke.sh          # real server + simulated AI on this machine, real core on the emulator
```

* `OnboardingFlowTest` covers the welcome, new accounts, the setup after signing in, and connecting a computer by a
  scanned, typed or linked code (the scanner is replaced through `QrScannerProvider`).
* `PasswordlessFlowTest` covers "Continue" (the Custom Tab, the callback, a callback nobody waits for), the Unlock
  screen (asking the other phone, the recovery code, relaunching while locked), the approval device's side of "Add
  another phone" and the recovery code in Settings.
* `AppFlowTest` (Robolectric) drives every screen against an in-memory `FakeCore`: sign-in, 2FA, approvals (once,
  standing, public-domain warning), biometric fail-closed paths, pairing, deep-link spoofing, `FLAG_SECURE`, grants,
  activity.
* `RealCoreTest` (instrumented) loads the real Rust library and the real Keystore on a device.
* `LiveServerTest` + `scripts/device-smoke.sh` run the whole connection flow over `adb reverse`.
* Some emulator images crash `system_server` when it snapshots a task (`hasReadColorBufferDma` assertion), which is why
  the UI tests run on the JVM and the device tests avoid UI.

## Design review screenshots

```bash
./gradlew testFullDebugUnitTest --tests '*Screenshots*' -Dreins.screenshots=/tmp/shots   # dark and light PNGs of every screen
```

Rendered through Robolectric with frozen clocks, so they are reproducible. `FLAG_SECURE` (no screenshots of the
screens that show secrets) is off in debug builds, which the tests use, and on in release builds;
`-Preins.secureScreens=true|false` overrides it for either.

## Layout

```
app/src/main/java/dev/reins/android/
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
              main (floating nav bar), signin (welcome, password forms, Unlock), join (another phone asks for the
              account's keys), nav
```
