# Reins Plan 4: Android App — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Reins Android app in `android/`: a thin Kotlin + Jetpack Compose shell (screens, Keystore, Google authorization, FCM, biometrics, notifications) around the Rust `reins-core` library, which owns all logic, networking and storage.

**Architecture:** One Gradle app module, package `dev.reins.android`. Every screen talks to a small Kotlin `CoreApi` interface (app-owned mirror of contracts §D); a `FakeCoreApi` backs Compose previews and instrumented UI tests, and `UniffiCoreApi` (last tasks, after Plan 3 merges) adapts the UniFFI-generated `dev.reins.core.ReinsCore`. A `MainSafeCoreApi` decorator moves every core call onto `Dispatchers.IO`, so no Rust code — even the synchronous prefix of an `async fn` — ever runs on the main thread. Navigation is a hand-rolled back stack of a sealed `Route` type; state is `StateFlow` in per-screen `ViewModel`s.

**Tech Stack:** Gradle 9.8.0 (wrapper), AGP 9.4.1 (built-in Kotlin), Kotlin 2.4.20 (Compose compiler plugin), JDK 21, Compose BOM 2026.06.01 (Compose 1.11.4, Material 3 1.4.0), activity-compose 1.13.0, lifecycle 2.10.0, biometric 1.1.0, work-runtime-ktx 2.12.0, firebase-messaging 25.1.3, play-services-auth 22.0.0, kotlinx-coroutines 1.11.0, JNA 5.19.1 (`@aar`), google-services plugin 4.5.0; Rust side: cargo-ndk 4.1.2, NDK 27.2.12479018, UniFFI 0.32.2 bindgen.

**Spec:** `docs/superpowers/specs/2026-09-28-reins-mvp-design.md` (§2 trust model, §6 Android, §7 errors, §10 risks). **Binding interface:** `docs/superpowers/specs/2026-09-29-reins-contracts.md` §A (push kinds), §D (UniFFI surface — Plan 3 produces, this plan consumes). Roadmap: `docs/superpowers/plans/2026-09-29-reins-roadmap.md`.

## Global Constraints

- Repo root `<repo>`, branch `reins-mvp`. Android project root `android/`. This plan creates and modifies files only under `android/`; it never touches upstream Vaultwarden files or any crate (it only *builds* `crates/reins-core`).
- Package / applicationId `dev.reins.android`; `minSdk 31`, `compileSdk 36`, `targetSdk 36`; Kotlin + Jetpack Compose only. NEVER Flutter, no XML layouts (only the manifest, `res/values*` and `res/xml`).
- Dependencies limited to (spec §6): Compose (BOM, material3), `activity-compose`, `lifecycle-viewmodel-compose` (+ `lifecycle-runtime-compose`), `androidx.biometric`, `work-runtime-ktx`, `firebase-messaging`, `play-services-auth` (AuthorizationClient), `kotlinx-coroutines-android`, JNA (`net.java.dev.jna:jna:5.19.1@aar`). Test-only: JUnit 4, kotlinx-coroutines-test, androidx.test (runner, ext-junit), compose ui-test-junit4. No DI framework, no Room/Retrofit/OkHttp, no navigation library, no `kotlinx-coroutines-play-services` (a 15-line `Task.await()` is hand-written).
- All logic, networking and storage live in Rust (`reins-core`). Kotlin only: UI, Android Keystore, Google `AuthorizationClient`, FCM, WorkManager, notifications, `BiometricPrompt`.
- Nothing on the main thread: every `CoreApi` call goes through `MainSafeCoreApi` (`withContext(Dispatchers.IO)`); `ReinsCore` is constructed on `Dispatchers.IO`; debug builds enable `StrictMode` thread policy (`detectDiskReads`, `detectDiskWrites`, `detectNetwork`, `penaltyLog`).
- Toolchain paths (verbatim): Android SDK `$HOME/Android/Sdk` (the environment's `ANDROID_HOME=/opt/android-sdk` is stale — always `export ANDROID_HOME=$HOME/Android/Sdk`), NDK `$HOME/Android/Sdk/ndk/27.2.12479018`, JDK 21, Rust via `export PATH="$HOME/.cargo/bin:$PATH"` (rustc 1.98.1, targets `aarch64-linux-android`, `x86_64-linux-android`), `cargo-ndk 4.1.2`. Gradle distribution unpacked at `~/.local/share/gradle-dist/gradle-9.8.0`.
- Emulator: instrumented tests and the smoke run target `emulator-5554` (API 36, x86_64, Play services); always `export ANDROID_SERIAL=emulator-5554` (other emulators may be attached and `connectedDebugAndroidTest` would otherwise run on all of them). If `adb devices` does not list it, start one: `ANDROID_AVD_HOME=~/.config/.android/avd $HOME/Android/Sdk/emulator/emulator -avd zeron36 -read-only -port 5554 -no-window -no-audio -no-boot-anim -no-snapshot &` then `adb -s emulator-5554 wait-for-device` and wait until `adb -s emulator-5554 shell getprop sys.boot_completed` prints `1`. A one-off `system_server` hiccup on this emulator can fail a UI test; rerun once before debugging.
- Every Gradle command runs in `android/` with `export ANDROID_HOME=$HOME/Android/Sdk` (shown as `$ENV` below: `export ANDROID_HOME=$HOME/Android/Sdk ANDROID_SERIAL=emulator-5554 PATH="$HOME/.cargo/bin:$PATH"`).
- ABIs: `arm64-v8a` and `x86_64` only. 16 KB pages: Rust links with `-C link-arg=-Wl,-z,max-page-size=16384` (NDK 27.2 < r28).
- Gmail scopes (verbatim): `https://www.googleapis.com/auth/gmail.readonly`, `https://www.googleapis.com/auth/gmail.send`.
- FCM data payload: `{t: "req" | "pair" | "replaced", id}` → passed unchanged to `core.handlePush(t, id)` (contracts §A). Push is a hint only; only high-priority messages get expedited work.
- `google-services.json` source: `~/.config/reins/google-services.json` (override with Gradle property `reins.googleServicesJson`); copied to `android/app/google-services.json` (gitignored) when present. The app MUST build and run without it (no FCM; foreground long-poll via `core.sync(25)` only).
- The build must not need an OAuth client: with no Android OAuth client in the Google Cloud project, the app shows "Gmail needs setup" (from `GmailStatus.Unavailable`) and everything else works.
- Lifetime choices (verbatim, spec §6): Once · 1 h · 24 h · 7 days · until revoked · N uses. Once creates no grant (`standing = null`).
- `GrantScopeChoice.subjectPattern` is literal "subject contains" text, never a regex; the UI labels it "Subject contains".
- Approval screen warns when a standing grant uses a whole-domain rule for a public mail domain (list in `PublicMailDomains.kt`, Task 7).
- Untrusted text (sender, recipients, subject, snippet, body, labels, client names) is shown with bidi controls stripped; addresses are always laid out LTR.
- `FLAG_SECURE` on Approval, Pairing, Grants and Activity screens.
- Approve (requests and pairings) requires `BiometricPrompt` with `BIOMETRIC_STRONG or DEVICE_CREDENTIAL`; Deny is one tap and needs no authentication.
- Exported components: only `MainActivity` (launcher). `ReinsMessagingService` is `exported="false"`. Notification `PendingIntent`s are `FLAG_IMMUTABLE` with an explicit component.
- Commits: one per task, message prefix `feat(android):` / `test(android):` / `build(android):`; never commit `android/app/google-services.json`, `local.properties`, build outputs, generated bindings or `.so` files.

## Review Focus

- **Main-thread blocking:** a core call whose Rust `async fn` does CPU work (Argon2 KDF in `login`) before its first `.await` must not run on the main thread; the UI stays responsive. Pinned in Task 3 (`MainSafeCoreApiTest`) and Task 12 (instrumented login responsiveness test).
- **FLAG_SECURE:** Approval, Pairing, Grants and Activity windows carry `FLAG_SECURE` while shown and Home does not after navigating back. Pinned in Task 6 (`SecureScreenTest`).
- **Biometric bypass:** Approve must never reach `core.approve`/`core.answerPairing(approve=true)` unless the authenticator returned success; cancel, error, and "no screen lock enrolled" all fail closed. Pinned in Tasks 8 and 9.
- **Notification deep-link spoofing / exported components:** any app can start `MainActivity`; a crafted `OPEN_ITEM` intent with an unknown or malformed id must land on Home, never on an Approval screen for a non-pending id; the FCM service is not exported. Pinned in Task 10.
- **No google-services.json:** the build succeeds, the app launches, sign-in renders, and no Firebase call crashes when Firebase is not initialized. Pinned in Tasks 1 and 10.

---

## Decisions

Every open question is decided here; nothing is left for the executor to choose.

1. **Toolchain (verified 2026-09-29 in a throwaway project on this machine):** Gradle 9.8.0, AGP 9.4.1, Kotlin 2.4.20, JDK 21 build and run a Compose app with every dependency below; `connectedDebugAndroidTest` passes on the API 36 x86_64 emulator. AGP 9 has **built-in Kotlin**: do NOT apply `org.jetbrains.kotlin.android`; apply only `org.jetbrains.kotlin.plugin.compose`.
2. **compileSdk 36 pins Compose to BOM 2026.06.01 and lifecycle to 2.10.0.** BOM 2026.08.00+ (Compose 1.12) and lifecycle 2.11.0 fail `checkDebugAarMetadata` with "requires compileSdk 37"; only platforms 34–36 are installed and the spec fixes 36. Upgrade both together with compileSdk 37 later.
3. **Espresso 3.7.0 is pinned in `androidTestImplementation`.** The version pulled in transitively by Compose ui-test calls `InputManager.getInstance()`, removed on API 36 → every UI test fails with `NoSuchMethodException`. Compose tests use the non-deprecated `androidx.compose.ui.test.junit4.v2.*` rules.
4. **UniFFI 0.32.2 Kotlin cannot compile an error variant with a field named `message`** (verified: "Conflicting declarations … 'message' hides member of supertype 'Throwable'"). Contracts §D uses `message` in `CoreError::{Network, Server, Gmail, Invalid, Storage}` and `ForeignError::Failed`. This plan's adapter code (Task 12) assumes those fields are renamed to **`reason`** (see "Contract changes" at the end); `GmailStatus::Unavailable { message }` is a plain enum and is fine.
5. **Generated-type facts (verified with a crate mirroring §D):** `Vec<u8>` → `ByteArray` (so `PairingView.choices` is a *signed* `ByteArray`: read codes with `it.toInt() and 0xFF`); `Option<u8>` → `UByte?`; `u16`/`u32`/`u64` → `UShort`/`UInt`/`ULong`; enums → `PendingKind.REQUEST`; errors → `CoreException.X` / `ForeignException.X`; the async foreign trait method is `suspend fun accessToken(): String`; the constructor is a *synchronous* `ReinsCore(dataDir, keys, google, notifier)`.
6. **App-owned model mirror + adapter.** The UI never imports `dev.reins.core.*`. `core/Models.kt` mirrors §D with Kotlin-friendly types (`Int`/`Long`, no unsigned) and `core/CoreFailure.kt` mirrors `CoreError`. Only `core/UniffiCoreApi.kt` and `core/FfiMappers.kt` (Task 12) touch generated code. This lets Tasks 1–11 build, run and test with no Rust at all while Plan 3 is in progress.
7. **Main-thread safety is structural.** UniFFI polls a Rust future on the calling thread, so the synchronous prefix of an `async fn` (e.g. Argon2 in `login`) would run on the main thread if called from `viewModelScope`. `MainSafeCoreApi` wraps every call in `withContext(Dispatchers.IO)` and constructs the delegate lazily on that dispatcher. The container hands out only the wrapped instance.
8. **No navigation library.** `Navigator` is a `mutableStateListOf<Route>` back stack in an activity-scoped `AppViewModel`; `BackHandler` pops. ViewModels are activity-scoped and keyed per route (`"approval:<id>"`); no per-destination stores (the app has 7 screens).
9. **Test injection without DI:** `CoreProvider.factory` (a `fun interface CoreFactory`) is read once in `ReinsApp.onCreate`. The instrumented runner `ReinsTestRunner` sets it to return a shared `FakeCoreApi` *before* `Application.onCreate` runs. `FakeCoreApi` lives in `main` (used by previews and, until Task 12, as the default core); R8 strips it from release once Task 12 switches the default.
10. **Deep links are validated against local state, not trusted.** `MainActivity` must be exported (launcher), so any app can send it `ACTION_OPEN_ITEM`. The id must match `[A-Za-z0-9_-]{1,128}`, the kind must be `request|pairing`, and the item must be in `core.pending()`; otherwise the app shows Home with "That request is no longer waiting." Even a valid spoof only opens a screen the user must still approve with biometrics.
11. **FLAG_SECURE is reference-counted per window** (`SecureFlag`), so a transition between two secure screens never clears it in between; applied by the `SecureWindow()` composable in each secure screen.
12. **Biometric gate lives in the ViewModel call path.** `approve(authenticate)` / `answer(authenticate)` call `core.approve` / `core.answerPairing(approve = true)` only after `AuthResult.Success`. `BiometricAuthenticator` returns `Unavailable` when `canAuthenticate(BIOMETRIC_STRONG or DEVICE_CREDENTIAL) != BIOMETRIC_SUCCESS` (no screen lock) — fail closed with "Set a screen lock to approve". No CryptoObject: approval is not key-bound in the MVP (spec §2 accepts that the phone is the trust root).
13. **Scope builder semantics** (maps to `GrantScopeChoice`): Once → `standing = null`. Standing read grant, default "Only these messages" → `selectedMessagesOnly = true` (needs ≥ 1 selected message). "Also allow similar" → `selectedMessagesOnly = false` plus any of: sender addresses, sender domains (both offered from the selected messages' senders), "Subject contains" text; at least one required. Standing send grant: default = every To/Cc address exact; "similar" lets the user swap an address for its domain and add "Subject contains". The public-domain warning shows for any chosen domain in `PublicMailDomains` (read and send).
14. **Lifetime values:** 1 h = 3600 s, 24 h = 86400 s, 7 days = 604800 s, until revoked = no duration and no max uses, N uses = `maxUses = N` (1..1000) with no duration.
15. **`sync()` result:** after each `core.sync(25)` the app re-reads `core.pending()` (local, cheap) and publishes that full list; it does not assume `sync` returns the full list.
16. **Poller:** runs while `MainActivity` is STARTED and the user is signed in; errors back off 1 s, 2 s, 4 s … capped at 30 s; `CoreFailure.NotLoggedIn` signs the UI out; `CoreFailure.Server(403, …)` marks the device "replaced" and stops polling until re-registration.
17. **Device replacement notice:** the worker passes `replaced` to `core.handlePush` unchanged and then posts a local "This phone is no longer your approval device" notification and sets `AppState.device = Replaced`.
18. **Firebase optional:** the google-services plugin is applied only when `app/google-services.json` exists after the copy step; `BuildConfig.HAS_FIREBASE` records it and `FirebaseSupport.available()` also checks `FirebaseApp.getApps()`. Without it: no FCM token (`registerDevice(null)`), foreground long-poll only.
19. **Gmail consent:** `GmailAuthorizer` uses `Identity.getAuthorizationClient(context).authorize(...)`; `hasResolution()` → `NeedsUserInteraction` for the core and a `PendingIntent` for the Connect button (`StartIntentSenderForResult`). `ApiException` status 10 (DEVELOPER_ERROR — no Android OAuth client for this package+SHA-1, today's state) → "Gmail needs setup: register this app's package and signing SHA-1 as an Android OAuth client in Google Cloud (project reins-main)". Disconnect = `clearToken` + `revokeAccess` for the account from a silent authorize.
20. **Rust build via the Variant API**, not `preBuild.dependsOn`: `CargoNdkTask` and `UniffiBindgenTask` are registered with `variant.sources.jniLibs.addGeneratedSourceDirectory` / `variant.sources.kotlin.addGeneratedSourceDirectory`, so AGP wires them before merge/compile automatically for every variant. Outputs go to `app/build/rustJniLibs` and `app/build/generated/uniffi/kotlin` (never under `src/`). `CargoNdkTask` is never up-to-date (cargo is incremental, a no-op run takes ~1 s); downstream tasks stay up-to-date because outputs are content-hashed.
21. **Rust profile:** default `release` (the workspace profile: fat LTO, `strip = "debuginfo"` — symbols stay, which UniFFI library-mode bindgen needs). `-Preins.rustProfile=release-low` for faster local iteration. Never `release-micro` (strips symbols → bindgen fails).
22. **Storage location:** core `dataDir` = `noBackupFilesDir/core`; `android:allowBackup="false"` (Keystore-wrapped keys cannot be restored on another device anyway).
23. **Release build** is minified (R8) and, for the MVP, signed with the debug keystore (only the debug SHA-1 is registered in Firebase/Google Cloud). JNA and generated bindings are kept by `proguard-rules.pro`.
24. **Notifications:** platform `Notification.Builder` (minSdk 31, no compat library); channels `approvals` (high) and `status` (default); `VISIBILITY_PRIVATE` with a content-free public version; posted only if notifications are enabled; `POST_NOTIFICATIONS` is requested from Home on API 33+.
25. **No icon library.** Material icons are not a dependency; actions are text buttons.
26. **Theme:** Material 3 dynamic color (always available at minSdk 31), light/dark from the system; edge-to-edge (`enableEdgeToEdge()`), required by targetSdk 36.

## File Structure

```
android/.gitignore                                   ignore build outputs, local.properties, app/google-services.json
android/settings.gradle.kts                          repositories, :app
android/build.gradle.kts                             plugin aliases (apply false)
android/gradle.properties                            JVM args, AndroidX, config cache
android/gradle/libs.versions.toml                    version catalog (all versions live here)
android/gradle/wrapper/*, android/gradlew*           Gradle 9.8.0 wrapper (generated in Task 1)
android/app/build.gradle.kts                         app module; google-services copy; Rust tasks (Task 12)
android/app/proguard-rules.pro                       JNA + UniFFI keep rules
android/app/src/main/AndroidManifest.xml             components, permissions
android/app/src/main/res/values/strings.xml          app name
android/app/src/main/res/values/themes.xml           window theme for edge-to-edge
android/app/src/main/res/drawable/ic_notification.xml, ic_launcher_foreground.xml
android/app/src/main/res/mipmap-anydpi/ic_launcher.xml
android/app/src/main/java/dev/reins/android/
  ReinsApp.kt                  Application: builds AppContainer, channels, StrictMode (debug)
  AppContainer.kt                 process singletons: core, AppState, stores, platform services
  MainActivity.kt                 FragmentActivity host: Compose root, deep links, poller lifecycle
  core/CoreApi.kt                 the seam: suspend interface mirroring contracts §D
  core/Models.kt                  app-owned mirror records/enums
  core/CoreFailure.kt             app-owned mirror of CoreError
  core/MainSafeCoreApi.kt         every call on Dispatchers.IO; lazy construction
  core/CoreProvider.kt            CoreFactory + default factory (fake until Task 12)
  core/FakeCoreApi.kt             in-memory core for previews and UI tests
  core/UniffiCoreApi.kt           (Task 12) adapter over dev.reins.core.ReinsCore
  core/FfiMappers.kt              (Task 12) generated <-> app model mapping
  core/ForeignAdapters.kt         (Task 12) KeyWrapper/GoogleTokenProvider/Notifier implementations
  state/AppState.kt               session + device status flows
  state/PendingStore.kt           pending items flow
  state/DeviceRegistrar.kt        registerDevice with FCM token, updates AppState
  platform/TaskAwait.kt           Task<T>.await() for Play services tasks
  platform/FirebaseSupport.kt     optional Firebase: availability + token
  platform/KeystoreKeyWrapper.kt  AES-256-GCM non-exportable Keystore key
  platform/GmailAuthorizer.kt     AuthorizationClient: silent token, consent, disconnect
  platform/Authenticator.kt       Authenticator/AuthResult + BiometricAuthenticator
  platform/AppNotifier.kt         channels, item/replaced/gmail notifications, deep-link PendingIntents
  push/PushPayload.kt             FCM data validation
  push/PushHandler.kt             pure push handling (tested on JVM)
  push/PushWorker.kt              expedited CoroutineWorker → PushHandler
  push/RegisterDeviceWorker.kt    token refresh → DeviceRegistrar
  push/ReinsMessagingService.kt FirebaseMessagingService (not exported)
  sync/ForegroundSync.kt          lifecycle-bound long-poll loop + backoff
  ui/theme/Theme.kt               Material 3 dynamic light/dark
  ui/common/Untrusted.kt          bidi/control stripping for untrusted text
  ui/common/Widgets.kt            LtrText, SectionHeader, ErrorCard, formatting helpers
  ui/common/Errors.kt             CoreFailure → user message
  ui/common/SecureWindow.kt       ref-counted FLAG_SECURE
  ui/nav/Route.kt                 sealed routes
  ui/nav/Navigator.kt             back stack
  ui/nav/DeepLink.kt              intent → validated target
  ui/AppViewModel.kt              session-driven navigation, deep-link resolution
  ui/ReinsRoot.kt              route → screen switch
  ui/signin/SignInViewModel.kt, SignInScreen.kt
  ui/home/HomeViewModel.kt, HomeScreen.kt
  ui/gmail/GmailViewModel.kt, GmailSection.kt   shared by Home and Settings
  ui/approval/ApprovalLogic.kt    Lifetime, ScopeDraft, PublicMailDomains, buildChoice (pure)
  ui/approval/ApprovalViewModel.kt, ApprovalScreen.kt
  ui/pairing/PairingViewModel.kt, PairingScreen.kt
  ui/grants/GrantsViewModel.kt, GrantsScreen.kt
  ui/activity/ActivityViewModel.kt, ActivityScreen.kt
  ui/settings/SettingsViewModel.kt, SettingsScreen.kt
android/app/src/test/java/dev/reins/android/        JVM unit tests (per task)
android/app/src/androidTest/java/dev/reins/android/ instrumented tests + ReinsTestRunner + TestEnv
```

---

### Task 1: Gradle project, wrapper, optional Firebase

**Files:**
- Create: `android/.gitignore`, `android/settings.gradle.kts`, `android/build.gradle.kts`, `android/gradle.properties`, `android/gradle/libs.versions.toml`, `android/gradle/wrapper/gradle-wrapper.properties`, `android/gradle/wrapper/gradle-wrapper.jar`, `android/gradlew`, `android/gradlew.bat` (the last four generated), `android/local.properties` (not committed)
- Create: `android/app/build.gradle.kts`, `android/app/proguard-rules.pro`, `android/app/src/main/AndroidManifest.xml`, `android/app/src/main/res/values/strings.xml`, `android/app/src/main/res/values/themes.xml`, `android/app/src/main/res/drawable/ic_notification.xml`, `android/app/src/main/res/drawable/ic_launcher_foreground.xml`, `android/app/src/main/res/mipmap-anydpi/ic_launcher.xml`, `android/app/src/main/java/dev/reins/android/MainActivity.kt`
- Test: `android/app/src/androidTest/java/dev/reins/android/LaunchTest.kt`

**Interfaces:**
- Consumes: nothing.
- Produces: a buildable module `:app` (namespace `dev.reins.android`), `BuildConfig.HAS_FIREBASE: Boolean`, version catalog aliases used by every later task (`libs.*`), `MainActivity : FragmentActivity` (content replaced in Task 5).

- [ ] **Step 1: Create the root build files**

`android/.gitignore`:

```gitignore
/.gradle/
/.kotlin/
/build/
/app/build/
/local.properties
/app/google-services.json
/captures/
.idea/
*.iml
```

`android/settings.gradle.kts`:

```kotlin
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "reins-android"
include(":app")
```

`android/build.gradle.kts`:

```kotlin
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.google.services) apply false
}
```

`android/gradle.properties`:

```properties
org.gradle.jvmargs=-Xmx4g -Dfile.encoding=UTF-8
org.gradle.caching=true
org.gradle.configuration-cache=true
android.useAndroidX=true
android.nonTransitiveRClass=true
kotlin.code.style=official
```

`android/gradle/libs.versions.toml`:

```toml
[versions]
agp = "9.4.1"
kotlin = "2.4.20"
googleServices = "4.5.0"
# compileSdk 36: Compose BOM 2026.08.00+ and lifecycle 2.11+ require compileSdk 37 (see plan Decision 2).
composeBom = "2026.06.01"
activityCompose = "1.13.0"
lifecycle = "2.10.0"
biometric = "1.1.0"
work = "2.12.0"
firebaseMessaging = "25.1.3"
playServicesAuth = "22.0.0"
coroutines = "1.11.0"
jna = "5.19.1"
junit = "4.13.2"
androidxTestRunner = "1.7.0"
androidxTestExtJunit = "1.3.0"
espresso = "3.7.0"

[libraries]
compose-bom = { module = "androidx.compose:compose-bom", version.ref = "composeBom" }
compose-material3 = { module = "androidx.compose.material3:material3" }
compose-ui = { module = "androidx.compose.ui:ui" }
compose-ui-tooling-preview = { module = "androidx.compose.ui:ui-tooling-preview" }
compose-ui-tooling = { module = "androidx.compose.ui:ui-tooling" }
compose-ui-test-junit4 = { module = "androidx.compose.ui:ui-test-junit4" }
compose-ui-test-manifest = { module = "androidx.compose.ui:ui-test-manifest" }
activity-compose = { module = "androidx.activity:activity-compose", version.ref = "activityCompose" }
lifecycle-viewmodel-compose = { module = "androidx.lifecycle:lifecycle-viewmodel-compose", version.ref = "lifecycle" }
lifecycle-runtime-compose = { module = "androidx.lifecycle:lifecycle-runtime-compose", version.ref = "lifecycle" }
biometric = { module = "androidx.biometric:biometric", version.ref = "biometric" }
work-runtime-ktx = { module = "androidx.work:work-runtime-ktx", version.ref = "work" }
firebase-messaging = { module = "com.google.firebase:firebase-messaging", version.ref = "firebaseMessaging" }
play-services-auth = { module = "com.google.android.gms:play-services-auth", version.ref = "playServicesAuth" }
coroutines-android = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-android", version.ref = "coroutines" }
coroutines-test = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-test", version.ref = "coroutines" }
jna = { module = "net.java.dev.jna:jna", version.ref = "jna" }
junit = { module = "junit:junit", version.ref = "junit" }
androidx-test-runner = { module = "androidx.test:runner", version.ref = "androidxTestRunner" }
androidx-test-ext-junit = { module = "androidx.test.ext:junit", version.ref = "androidxTestExtJunit" }
espresso-core = { module = "androidx.test.espresso:espresso-core", version.ref = "espresso" }

[plugins]
android-application = { id = "com.android.application", version.ref = "agp" }
kotlin-compose = { id = "org.jetbrains.kotlin.plugin.compose", version.ref = "kotlin" }
google-services = { id = "com.google.gms.google-services", version.ref = "googleServices" }
```

- [ ] **Step 2: Create the app module**

`android/app/build.gradle.kts`:

```kotlin
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// google-services.json is optional (plan Decision 18). Copy it in from outside the repo when
// available; apply the plugin only when the file is present so the app builds without Firebase.
val googleServicesSource = file(
    providers.gradleProperty("reins.googleServicesJson")
        .getOrElse("${System.getProperty("user.home")}/.config/reins/google-services.json"),
)
val googleServicesTarget = file("google-services.json")
if (!googleServicesTarget.exists() && googleServicesSource.isFile) {
    googleServicesSource.copyTo(googleServicesTarget)
}
val hasFirebase = googleServicesTarget.isFile
if (hasFirebase) {
    apply(plugin = "com.google.gms.google-services")
}

android {
    namespace = "dev.reins.android"
    compileSdk = 36
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "dev.reins.android"
        minSdk = 31
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
        buildConfigField("boolean", "HAS_FIREBASE", hasFirebase.toString())
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // MVP: only the debug keystore's SHA-1 is registered with Firebase / Google Cloud.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

kotlin {
    jvmToolchain(21)
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.material3)
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.biometric)
    implementation(libs.work.runtime.ktx)
    implementation(libs.firebase.messaging)
    implementation(libs.play.services.auth)
    implementation(libs.coroutines.android)
    implementation(libs.jna) { artifact { type = "aar" } }

    testImplementation(libs.junit)
    testImplementation(libs.coroutines.test)

    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.espresso.core)
    androidTestImplementation(libs.coroutines.test)
    debugImplementation(libs.compose.ui.test.manifest)
}
```

`android/app/proguard-rules.pro`:

```text
# JNA (UniFFI runtime): accessed reflectively from native code.
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-dontwarn java.awt.**
# UniFFI-generated bindings (Structure fields and callbacks are looked up by name).
-keep class dev.reins.core.** { *; }
```

`android/app/src/main/AndroidManifest.xml`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">

    <uses-permission android:name="android.permission.INTERNET" />

    <application
        android:allowBackup="false"
        android:fullBackupContent="false"
        android:icon="@mipmap/ic_launcher"
        android:label="@string/app_name"
        android:supportsRtl="true"
        android:theme="@style/Theme.Reins">

        <activity
            android:name=".MainActivity"
            android:exported="true"
            android:launchMode="singleTop"
            android:windowSoftInputMode="adjustResize">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
```

`android/app/src/main/res/values/strings.xml`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<resources>
    <string name="app_name">Reins</string>
</resources>
```

`android/app/src/main/res/values/themes.xml` (note: `android:Theme.Material.DayNight.*` does not exist; AAPT fails on it):

```xml
<?xml version="1.0" encoding="utf-8"?>
<resources>
    <!-- Window-level theme only (follows system light/dark); all UI colors come from Compose Material 3. -->
    <style name="Theme.Reins" parent="android:Theme.DeviceDefault.DayNight">
        <item name="android:windowActionBar">false</item>
        <item name="android:windowNoTitle">true</item>
    </style>
</resources>
```

`android/app/src/main/res/drawable/ic_notification.xml`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp"
    android:height="24dp"
    android:viewportWidth="24"
    android:viewportHeight="24">
    <path
        android:fillColor="#FFFFFFFF"
        android:pathData="M12,2L4,5v6c0,5.55 3.84,10.74 8,12c4.16,-1.26 8,-6.45 8,-12V5L12,2zM10.5,16.5l-4,-4 1.41,-1.41L10.5,13.67l5.59,-5.59L17.5,9.5l-7,7z" />
</vector>
```

`android/app/src/main/res/drawable/ic_launcher_foreground.xml`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <group
        android:translateX="30"
        android:translateY="30"
        android:scaleX="2"
        android:scaleY="2">
        <path
            android:fillColor="#FFFFFFFF"
            android:pathData="M12,2L4,5v6c0,5.55 3.84,10.74 8,12c4.16,-1.26 8,-6.45 8,-12V5L12,2zM10.5,16.5l-4,-4 1.41,-1.41L10.5,13.67l5.59,-5.59L17.5,9.5l-7,7z" />
    </group>
</vector>
```

`android/app/src/main/res/mipmap-anydpi/ic_launcher.xml`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@android:color/system_accent1_600" />
    <foreground android:drawable="@drawable/ic_launcher_foreground" />
    <monochrome android:drawable="@drawable/ic_launcher_foreground" />
</adaptive-icon>
```

`android/app/src/main/java/dev/reins/android/MainActivity.kt`:

```kotlin
package dev.reins.android

import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.fragment.app.FragmentActivity

/** Task 1 shell; Task 5 replaces the content with the real root. FragmentActivity: BiometricPrompt needs it. */
class MainActivity : FragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                Surface(Modifier.fillMaxSize()) {
                    Box(contentAlignment = Alignment.Center) { Text("Reins") }
                }
            }
        }
    }
}
```

- [ ] **Step 3: Bootstrap the Gradle wrapper and SDK location**

There is no system `gradle`; the 9.8.0 distribution is unpacked at `~/.local/share/gradle-dist/gradle-9.8.0` (if it is missing: `mkdir -p ~/.local/share/gradle-dist && curl -fL https://services.gradle.org/distributions/gradle-9.8.0-bin.zip -o ~/.local/share/gradle-dist/gradle-9.8.0-bin.zip && unzip -q ~/.local/share/gradle-dist/gradle-9.8.0-bin.zip -d ~/.local/share/gradle-dist`). The `app/` directory must exist before this runs (Gradle refuses to configure an included project without a directory).

```bash
cd <repo>/android
export ANDROID_HOME=$HOME/Android/Sdk
~/.local/share/gradle-dist/gradle-9.8.0/bin/gradle wrapper --gradle-version 9.8.0 --distribution-type bin
printf 'sdk.dir=$HOME/Android/Sdk\n' > local.properties
./gradlew --version | head -3
```

Expected: `Gradle 9.8.0`; files `gradlew`, `gradlew.bat`, `gradle/wrapper/gradle-wrapper.{jar,properties}` exist.

- [ ] **Step 4: Write the launch test**

`android/app/src/androidTest/java/dev/reins/android/LaunchTest.kt`:

```kotlin
package dev.reins.android

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.platform.app.InstrumentationRegistry
import com.google.firebase.FirebaseApp
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class LaunchTest {
    @get:Rule val rule = createAndroidComposeRule<MainActivity>()

    @Test
    fun launchesWithOrWithoutFirebase() {
        rule.onNodeWithText("Reins").assertIsDisplayed()
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        // Firebase is initialized exactly when the build had google-services.json.
        assertEquals(BuildConfig.HAS_FIREBASE, FirebaseApp.getApps(context).isNotEmpty())
    }
}
```

- [ ] **Step 5: Build and run the launch test WITHOUT google-services.json**

```bash
cd <repo>/android
export ANDROID_HOME=$HOME/Android/Sdk ANDROID_SERIAL=emulator-5554
rm -f app/google-services.json
./gradlew connectedDebugAndroidTest -Preins.googleServicesJson=/nonexistent
ls app/google-services.json 2>&1 | grep -c 'No such file'
```

Expected: `BUILD SUCCESSFUL`, `LaunchTest` 1 test passed (Firebase not initialized and `HAS_FIREBASE == false`), last command prints `1`.

- [ ] **Step 6: Build and run it WITH google-services.json**

```bash
./gradlew connectedDebugAndroidTest
test -f app/google-services.json && git -C .. check-ignore -q android/app/google-services.json && echo ignored
```

Expected: `BUILD SUCCESSFUL` (task `processDebugGoogleServices` ran), test passes (`HAS_FIREBASE == true`, Firebase initialized), prints `ignored`.

- [ ] **Step 7: Commit**

```bash
cd <repo>
git add android/.gitignore android/settings.gradle.kts android/build.gradle.kts android/gradle.properties android/gradle android/gradlew android/gradlew.bat android/app/build.gradle.kts android/app/proguard-rules.pro android/app/src
git status --short android | grep -E 'google-services|local.properties' && echo 'STOP: secret staged' || true
git commit -m "build(android): Gradle 9.8 / AGP 9.4 Compose project with optional Firebase"
```

### Task 2: Core seam — models, failures, `CoreApi`, `FakeCoreApi`

**Files:**
- Create: `android/app/src/main/java/dev/reins/android/core/Models.kt`
- Create: `android/app/src/main/java/dev/reins/android/core/CoreFailure.kt`
- Create: `android/app/src/main/java/dev/reins/android/core/CoreApi.kt`
- Create: `android/app/src/main/java/dev/reins/android/core/FakeCoreApi.kt`
- Test: `android/app/src/test/java/dev/reins/android/core/FakeCoreApiTest.kt`

**Interfaces:**
- Consumes: nothing.
- Produces (package `dev.reins.android.core`):
  - Records: `SessionInfo(serverUrl, email)`, `PendingKind { Request, Pairing }`, `PendingItem(kind, id, title, subtitle, createdAt: Long)`, `ApprovalKind { Search, Read, Send }`, `MessageView(id, from, subject, date: Long, snippet, coveredByGrant)`, `EmailView(to, cc, subject, body)`, `ApprovalView(requestId, connectionLabel, kind, query: String?, messages, email: EmailView?, createdAt)`, `ApprovalChoice(selectedMessageIds: List<String>, standing: StandingGrant?)`, `StandingGrant(durationSecs: Long?, maxUses: Int?, scope: GrantScopeChoice)`, `GrantScopeChoice(selectedMessagesOnly, senderAddresses, senderDomains, subjectPattern: String?, recipientAddresses, recipientDomains)` (all defaulted), `PairingView(id, clientName, clientHost, choices: List<Int>, createdAt)`, `GrantView(id, connectionLabel, action, summary, expiresAt: Long?, maxUses: Int?, uses: Int)`, `ConnectionView(id, label, clientHost, createdAt, lastUsedAt: Long?)`, `ActivityEntry(at, connectionLabel, action, outcome, detail, grantId: String?)`, `sealed interface GmailStatus { Ready; NeedsConsent; Unavailable(message) }`.
  - `sealed class CoreFailure : Exception` with `NotLoggedIn()`, `TwoFactorRequired()`, `UnsupportedTwoFactor()`, `InvalidCredentials()`, `Network(reason)`, `Server(status: Int, reason)`, `GmailNeedsConsent()`, `Gmail(reason)`, `NotFound()`, `Invalid(reason)`, `Storage(reason)`.
  - `interface CoreApi` — 18 `suspend` methods named exactly as contracts §D in camelCase (`sync(waitSecs: Int)`, `activity(limit: Int)`, `answerPairing(pairingId, approve, chosenCode: Int?, label: String?)`).
  - `class FakeCoreApi(now: () -> Long)`: public arrangeable state (`currentSession`, `requireTotp`, `password` = `"correct horse"`, `totpCode` = `"123456"`, `gmail`, `loginDelayMs`, `registerFailure`, `syncFailure`, `approveFailure`, `pendingFlow`, `approvals`, `pairings`, `pairingCodes`, `grantList`, `connectionList`, `activityList`), recorded effects (`calls`, `approved`, `denied`, `pairingAnswers`, `registeredTokens`, `pushes`), `reset()`, `loadSample(): FakeCoreApi`, `addPending(item)`, ids `FakeCoreApi.SEARCH_ID = "req-search-1"`, `SEND_ID = "req-send-1"`, `PAIRING_ID = "pair-1"` (correct code 47, choices 12/47/83).

- [ ] **Step 1: Write the failing test**

`android/app/src/test/java/dev/reins/android/core/FakeCoreApiTest.kt`:

```kotlin
package dev.reins.android.core

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class FakeCoreApiTest {
    private val fake = FakeCoreApi(now = { 1_000_000 })

    @Test
    fun loginAsksForTotpThenAcceptsIt() = runTest {
        fake.requireTotp = true
        try {
            fake.login("https://s", "me@example.com", "correct horse", null)
            fail("expected TwoFactorRequired")
        } catch (_: CoreFailure.TwoFactorRequired) {
        }
        val session = fake.login("https://s", "me@example.com", "correct horse", "123456")
        assertEquals(SessionInfo("https://s", "me@example.com"), fake.session())
        assertEquals("me@example.com", session.email)
    }

    @Test(expected = CoreFailure.InvalidCredentials::class)
    fun wrongPasswordIsInvalidCredentials() = runTest {
        fake.login("https://s", "me@example.com", "nope", null)
    }

    @Test
    fun approveRecordsChoiceAndClearsPending() = runTest {
        fake.loadSample()
        val choice = ApprovalChoice(listOf("m1", "m2"), standing = null)
        fake.approve(FakeCoreApi.SEARCH_ID, choice)
        assertEquals(listOf(FakeCoreApi.SEARCH_ID to choice), fake.approved.toList())
        assertFalse(fake.pending().any { it.id == FakeCoreApi.SEARCH_ID })
    }

    @Test
    fun wrongPairingCodeIsConflictAndCancelsPairing() = runTest {
        fake.loadSample()
        try {
            fake.answerPairing(FakeCoreApi.PAIRING_ID, approve = true, chosenCode = 12, label = "GPT")
            fail("expected 409")
        } catch (e: CoreFailure.Server) {
            assertEquals(409, e.status)
        }
        assertTrue(fake.pending().none { it.id == FakeCoreApi.PAIRING_ID })
    }

    @Test
    fun syncReturnsEarlyWhenAnItemArrives() = runTest {
        fake.loadSample()
        val result = async { fake.sync(25) }
        runCurrent()
        fake.addPending(PendingItem(PendingKind.Request, "new-1", "t", "s", 1))
        assertEquals("new-1", result.await().first().id)
    }

    @Test(expected = CoreFailure.NotLoggedIn::class)
    fun signedOutCallsFailWithNotLoggedIn() = runTest {
        fake.pending()
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd android && ./gradlew testDebugUnitTest --tests 'dev.reins.android.core.FakeCoreApiTest'`
Expected: FAIL — compilation errors `Unresolved reference 'FakeCoreApi'`.

- [ ] **Step 3: Write the models, failures and interface**

`android/app/src/main/java/dev/reins/android/core/Models.kt`:

```kotlin
package dev.reins.android.core

// App-owned mirror of contracts §D records/enums (plan Decision 6). Unsigned Rust integers are
// plain Int/Long here; only core/FfiMappers.kt (Task 12) converts to and from generated types.

data class SessionInfo(val serverUrl: String, val email: String)

enum class PendingKind { Request, Pairing }

data class PendingItem(
    val kind: PendingKind,
    val id: String,
    val title: String,
    val subtitle: String,
    val createdAt: Long,
)

enum class ApprovalKind { Search, Read, Send }

data class MessageView(
    val id: String,
    val from: String,
    val subject: String,
    val date: Long,
    val snippet: String,
    val coveredByGrant: Boolean,
)

data class EmailView(val to: List<String>, val cc: List<String>, val subject: String, val body: String)

data class ApprovalView(
    val requestId: String,
    val connectionLabel: String,
    val kind: ApprovalKind,
    val query: String?,
    val messages: List<MessageView>,
    val email: EmailView?,
    val createdAt: Long,
)

/** For Search/Read: ids ⊆ view messages, covered ones always included. For Send: ignored. */
data class ApprovalChoice(val selectedMessageIds: List<String>, val standing: StandingGrant?)

data class StandingGrant(val durationSecs: Long?, val maxUses: Int?, val scope: GrantScopeChoice)

/** `subjectPattern` is literal "subject contains" text, never a regex (contracts §A invariants). */
data class GrantScopeChoice(
    val selectedMessagesOnly: Boolean = false,
    val senderAddresses: List<String> = emptyList(),
    val senderDomains: List<String> = emptyList(),
    val subjectPattern: String? = null,
    val recipientAddresses: List<String> = emptyList(),
    val recipientDomains: List<String> = emptyList(),
)

data class PairingView(
    val id: String,
    val clientName: String,
    val clientHost: String,
    val choices: List<Int>,
    val createdAt: Long,
)

data class GrantView(
    val id: String,
    val connectionLabel: String,
    val action: String,
    val summary: String,
    val expiresAt: Long?,
    val maxUses: Int?,
    val uses: Int,
)

data class ConnectionView(
    val id: String,
    val label: String,
    val clientHost: String,
    val createdAt: Long,
    val lastUsedAt: Long?,
)

data class ActivityEntry(
    val at: Long,
    val connectionLabel: String,
    val action: String,
    val outcome: String,
    val detail: String,
    val grantId: String?,
)

sealed interface GmailStatus {
    data object Ready : GmailStatus
    data object NeedsConsent : GmailStatus
    data class Unavailable(val message: String) : GmailStatus
}
```

`android/app/src/main/java/dev/reins/android/core/CoreFailure.kt`:

```kotlin
package dev.reins.android.core

/** App-owned mirror of contracts §D `CoreError`. Every CoreApi method throws only these. */
sealed class CoreFailure(message: String) : Exception(message) {
    class NotLoggedIn : CoreFailure("Not signed in")
    class TwoFactorRequired : CoreFailure("Two-step login code required")
    class UnsupportedTwoFactor : CoreFailure("Unsupported two-step login method")
    class InvalidCredentials : CoreFailure("Invalid email or master password")
    class Network(val reason: String) : CoreFailure("Network: $reason")
    class Server(val status: Int, val reason: String) : CoreFailure("Server $status: $reason")
    class GmailNeedsConsent : CoreFailure("Gmail needs consent")
    class Gmail(val reason: String) : CoreFailure("Gmail: $reason")
    class NotFound : CoreFailure("Not found")
    class Invalid(val reason: String) : CoreFailure("Invalid: $reason")
    class Storage(val reason: String) : CoreFailure("Storage: $reason")
}
```

`android/app/src/main/java/dev/reins/android/core/CoreApi.kt`:

```kotlin
package dev.reins.android.core

/**
 * The seam between the Kotlin shell and reins-core (contracts §D, same method names).
 * Implementations: [FakeCoreApi] (previews/tests), UniffiCoreApi (Task 12). The app only ever
 * holds a [MainSafeCoreApi] wrapper, so callers may use it from any dispatcher.
 * Every method throws only [CoreFailure].
 */
interface CoreApi {
    suspend fun session(): SessionInfo?
    suspend fun login(serverUrl: String, email: String, password: String, totp: String?): SessionInfo
    suspend fun logout()
    suspend fun registerDevice(fcmToken: String?)

    suspend fun handlePush(kind: String, id: String)
    suspend fun sync(waitSecs: Int): List<PendingItem>
    suspend fun pending(): List<PendingItem>

    suspend fun approvalView(requestId: String): ApprovalView
    suspend fun approve(requestId: String, choice: ApprovalChoice)
    suspend fun deny(requestId: String)

    suspend fun pairingView(pairingId: String): PairingView
    suspend fun answerPairing(pairingId: String, approve: Boolean, chosenCode: Int?, label: String?)

    suspend fun grants(): List<GrantView>
    suspend fun revokeGrant(grantId: String)
    suspend fun connections(): List<ConnectionView>
    suspend fun revokeConnection(connectionId: String)
    suspend fun activity(limit: Int): List<ActivityEntry>

    suspend fun gmailStatus(): GmailStatus
}
```

- [ ] **Step 4: Write the fake**

The sample deliberately contains a U+202E (RIGHT-TO-LEFT OVERRIDE) in a subject and an activity detail, so screens exercise untrusted-text handling in previews and UI tests.

`android/app/src/main/java/dev/reins/android/core/FakeCoreApi.kt`:

```kotlin
package dev.reins.android.core

import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull

/**
 * In-memory [CoreApi] for Compose previews, JVM tests and instrumented UI tests.
 * All state is public and mutable so tests can arrange it; every call is recorded in [calls].
 */
class FakeCoreApi(private val now: () -> Long = { System.currentTimeMillis() / 1000 }) : CoreApi {

    data class PairingAnswer(val pairingId: String, val approve: Boolean, val chosenCode: Int?, val label: String?)

    // ---- arrangeable state ----
    @Volatile var currentSession: SessionInfo? = null
    @Volatile var requireTotp: Boolean = false
    @Volatile var password: String = "correct horse"
    @Volatile var totpCode: String = "123456"
    @Volatile var gmail: GmailStatus = GmailStatus.NeedsConsent
    @Volatile var loginDelayMs: Long = 0
    @Volatile var registerFailure: CoreFailure? = null
    @Volatile var syncFailure: CoreFailure? = null
    @Volatile var approveFailure: CoreFailure? = null
    val pendingFlow = MutableStateFlow<List<PendingItem>>(emptyList())
    val approvals = LinkedHashMap<String, ApprovalView>()
    val pairings = LinkedHashMap<String, PairingView>()
    val pairingCodes = LinkedHashMap<String, Int>()
    @Volatile var grantList: List<GrantView> = emptyList()
    @Volatile var connectionList: List<ConnectionView> = emptyList()
    @Volatile var activityList: List<ActivityEntry> = emptyList()

    // ---- recorded effects ----
    val calls = CopyOnWriteArrayList<String>()
    val approved = CopyOnWriteArrayList<Pair<String, ApprovalChoice>>()
    val denied = CopyOnWriteArrayList<String>()
    val pairingAnswers = CopyOnWriteArrayList<PairingAnswer>()
    val registeredTokens = CopyOnWriteArrayList<String?>()
    val pushes = CopyOnWriteArrayList<Pair<String, String>>()

    /** Back to a signed-out, empty core. */
    @Synchronized
    fun reset() {
        currentSession = null
        requireTotp = false
        password = "correct horse"
        totpCode = "123456"
        gmail = GmailStatus.NeedsConsent
        loginDelayMs = 0
        registerFailure = null
        syncFailure = null
        approveFailure = null
        pendingFlow.value = emptyList()
        approvals.clear()
        pairings.clear()
        pairingCodes.clear()
        grantList = emptyList()
        connectionList = emptyList()
        activityList = emptyList()
        calls.clear()
        approved.clear()
        denied.clear()
        pairingAnswers.clear()
        registeredTokens.clear()
        pushes.clear()
    }

    /** Signed in with two requests, one pairing, grants, connections and activity. */
    @Synchronized
    fun loadSample(): FakeCoreApi {
        reset()
        val t = now()
        currentSession = SessionInfo("https://reins.example", "me@example.com")
        gmail = GmailStatus.Ready
        approvals[SEARCH_ID] = ApprovalView(
            requestId = SEARCH_ID,
            connectionLabel = "Claude",
            kind = ApprovalKind.Search,
            query = "from:bank.com statement",
            messages = listOf(
                MessageView("m1", "alerts@bank.com", "Your September statement", t - 86_400, "Your statement is ready", coveredByGrant = true),
                MessageView("m2", "support@bank.com", "Card ending 4242", t - 3_600, "We noticed a new sign-in", coveredByGrant = false),
                MessageView("m3", "news@gmail.com", "‮Reversed subject", t - 600, "Newsletter", coveredByGrant = false),
            ),
            email = null,
            createdAt = t - 30,
        )
        approvals[SEND_ID] = ApprovalView(
            requestId = SEND_ID,
            connectionLabel = "ChatGPT",
            kind = ApprovalKind.Send,
            query = null,
            messages = emptyList(),
            email = EmailView(
                to = listOf("alice@gmail.com"),
                cc = listOf("bob@example.com"),
                subject = "Lunch on Friday?",
                body = "Hi Alice,\n\nAre you free for lunch on Friday?\n\nBest",
            ),
            createdAt = t - 10,
        )
        pairings[PAIRING_ID] = PairingView(PAIRING_ID, "ChatGPT", "chatgpt.com", listOf(12, 47, 83), t - 5)
        pairingCodes[PAIRING_ID] = 47
        pendingFlow.value = listOf(
            PendingItem(PendingKind.Pairing, PAIRING_ID, "Connect ChatGPT", "chatgpt.com", t - 5),
            PendingItem(PendingKind.Request, SEND_ID, "ChatGPT wants to send an email", "to alice@gmail.com", t - 10),
            PendingItem(PendingKind.Request, SEARCH_ID, "Claude wants to read mail", "from:bank.com statement", t - 30),
        )
        connectionList = listOf(
            ConnectionView("c1", "Claude", "claude.ai", t - 7 * 86_400, t - 30),
            ConnectionView("c2", "ChatGPT", "chatgpt.com", t - 86_400, null),
        )
        grantList = listOf(
            GrantView("g1", "Claude", "read", "from alerts@bank.com", t + 3_600, null, 2),
            GrantView("g2", "ChatGPT", "send", "to bob@example.com", null, 5, 1),
        )
        activityList = listOf(
            ActivityEntry(t - 100, "Claude", "read", "released", "1 message from alerts@bank.com", "g1"),
            ActivityEntry(t - 200, "ChatGPT", "send", "sent", "to bob@example.com: Re: plans", "g2"),
            ActivityEntry(t - 300, "Claude", "search", "denied", "query from:‮moc.knab", null),
        )
        return this
    }

    @Synchronized
    fun addPending(item: PendingItem) {
        pendingFlow.value = listOf(item) + pendingFlow.value.filterNot { it.id == item.id }
    }

    @Synchronized
    private fun removePending(id: String) {
        pendingFlow.value = pendingFlow.value.filterNot { it.id == id }
    }

    private fun requireSession(): SessionInfo = currentSession ?: throw CoreFailure.NotLoggedIn()

    // ---- CoreApi ----
    override suspend fun session(): SessionInfo? {
        calls += "session"
        return currentSession
    }

    override suspend fun login(serverUrl: String, email: String, password: String, totp: String?): SessionInfo {
        calls += "login"
        if (loginDelayMs > 0) delay(loginDelayMs)
        if (password != this.password) throw CoreFailure.InvalidCredentials()
        if (requireTotp && totp == null) throw CoreFailure.TwoFactorRequired()
        if (requireTotp && totp != totpCode) throw CoreFailure.InvalidCredentials()
        return SessionInfo(serverUrl, email).also { currentSession = it }
    }

    override suspend fun logout() {
        calls += "logout"
        currentSession = null
    }

    override suspend fun registerDevice(fcmToken: String?) {
        calls += "registerDevice"
        requireSession()
        registerFailure?.let { throw it }
        registeredTokens += fcmToken
    }

    override suspend fun handlePush(kind: String, id: String) {
        calls += "handlePush"
        requireSession()
        pushes += kind to id
    }

    override suspend fun sync(waitSecs: Int): List<PendingItem> {
        calls += "sync"
        requireSession()
        syncFailure?.let { throw it }
        withTimeoutOrNull(waitSecs * 1000L) { pendingFlow.drop(1).first() }
        return pendingFlow.value
    }

    override suspend fun pending(): List<PendingItem> {
        calls += "pending"
        requireSession()
        return pendingFlow.value
    }

    override suspend fun approvalView(requestId: String): ApprovalView {
        calls += "approvalView"
        requireSession()
        return synchronized(this) { approvals[requestId] } ?: throw CoreFailure.NotFound()
    }

    override suspend fun approve(requestId: String, choice: ApprovalChoice) {
        calls += "approve"
        requireSession()
        approveFailure?.let { throw it }
        synchronized(this) { approvals.remove(requestId) } ?: throw CoreFailure.NotFound()
        approved += requestId to choice
        removePending(requestId)
    }

    override suspend fun deny(requestId: String) {
        calls += "deny"
        requireSession()
        synchronized(this) { approvals.remove(requestId) } ?: throw CoreFailure.NotFound()
        denied += requestId
        removePending(requestId)
    }

    override suspend fun pairingView(pairingId: String): PairingView {
        calls += "pairingView"
        requireSession()
        return synchronized(this) { pairings[pairingId] } ?: throw CoreFailure.NotFound()
    }

    override suspend fun answerPairing(pairingId: String, approve: Boolean, chosenCode: Int?, label: String?) {
        calls += "answerPairing"
        requireSession()
        val expected = synchronized(this) {
            pairings.remove(pairingId) ?: throw CoreFailure.NotFound()
            pairingCodes.remove(pairingId)
        }
        pairingAnswers += PairingAnswer(pairingId, approve, chosenCode, label)
        removePending(pairingId)
        if (approve && chosenCode != expected) throw CoreFailure.Server(409, "wrong_code")
    }

    override suspend fun grants(): List<GrantView> {
        calls += "grants"
        requireSession()
        return grantList
    }

    override suspend fun revokeGrant(grantId: String) {
        calls += "revokeGrant"
        requireSession()
        grantList = grantList.filterNot { it.id == grantId }
    }

    override suspend fun connections(): List<ConnectionView> {
        calls += "connections"
        requireSession()
        return connectionList
    }

    override suspend fun revokeConnection(connectionId: String) {
        calls += "revokeConnection"
        requireSession()
        val label = connectionList.firstOrNull { it.id == connectionId }?.label ?: throw CoreFailure.NotFound()
        connectionList = connectionList.filterNot { it.id == connectionId }
        grantList = grantList.filterNot { it.connectionLabel == label }
    }

    override suspend fun activity(limit: Int): List<ActivityEntry> {
        calls += "activity"
        requireSession()
        return activityList.take(limit)
    }

    override suspend fun gmailStatus(): GmailStatus {
        calls += "gmailStatus"
        return gmail
    }

    companion object {
        const val SEARCH_ID = "req-search-1"
        const val SEND_ID = "req-send-1"
        const val PAIRING_ID = "pair-1"
    }
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd android && ./gradlew testDebugUnitTest --tests 'dev.reins.android.core.FakeCoreApiTest'`
Expected: PASS, 6 tests.

- [ ] **Step 6: Commit**

```bash
git add android/app/src/main/java/dev/reins/android/core android/app/src/test/java/dev/reins/android/core
git commit -m "feat(android): CoreApi seam mirroring the UniFFI contract, with an in-memory fake"
```

### Task 3: Main-safe core wrapper, app container, state

**Files:**
- Create: `android/app/src/main/java/dev/reins/android/core/MainSafeCoreApi.kt`
- Create: `android/app/src/main/java/dev/reins/android/core/CoreProvider.kt`
- Create: `android/app/src/main/java/dev/reins/android/state/AppState.kt`
- Create: `android/app/src/main/java/dev/reins/android/state/PendingStore.kt`
- Create: `android/app/src/main/java/dev/reins/android/state/DeviceRegistrar.kt`
- Create: `android/app/src/main/java/dev/reins/android/platform/TaskAwait.kt`
- Create: `android/app/src/main/java/dev/reins/android/platform/FirebaseSupport.kt`
- Create: `android/app/src/main/java/dev/reins/android/AppContainer.kt`
- Create: `android/app/src/main/java/dev/reins/android/ReinsApp.kt`
- Modify: `android/app/src/main/AndroidManifest.xml` (add `android:name=".ReinsApp"` to `<application>`)
- Test: `android/app/src/test/java/dev/reins/android/core/MainSafeCoreApiTest.kt`, `android/app/src/test/java/dev/reins/android/state/DeviceRegistrarTest.kt`

**Interfaces:**
- Consumes: Task 2 `CoreApi`, `CoreFailure`, `FakeCoreApi`, `SessionInfo`, `PendingItem`.
- Produces:
  - `class MainSafeCoreApi(dispatcher: CoroutineDispatcher, create: () -> CoreApi) : CoreApi` + `suspend fun warmUp()`.
  - `fun interface CoreFactory { fun create(container: AppContainer): CoreApi }`; `object CoreProvider { var factory: CoreFactory }` (default: `FakeCoreApi()` until Task 12).
  - `sealed interface SessionState { Loading; SignedOut; SignedIn(info); Broken(failure) }`, `sealed interface DeviceStatus { Unknown; Registered; Failed(failure); Replaced }`, `class AppState` with `session: StateFlow<SessionState>`, `device: StateFlow<DeviceStatus>`, `signedIn(info)`, `signedOut()`, `broken(failure)`, `setDevice(status)`.
  - `class PendingStore` with `items: StateFlow<List<PendingItem>>` (newest first), `replace(list)`, `upsert(item)`, `remove(id)`.
  - `fun interface PushTokenSource { suspend fun token(): String? }`; `class DeviceRegistrar(core, tokens, state)` with `suspend fun register(token: String? = null): Boolean`.
  - `suspend fun <T> Task<T>.await(): T` (package `dev.reins.android.platform`); `object FirebaseSupport { fun available(context): Boolean; suspend fun token(context): String? }`.
  - `class AppContainer(context, factory)` with `appScope`, `core: MainSafeCoreApi`, `state: AppState`, `pending: PendingStore`, `pushTokens`, `registrar`, `fun start()`, `suspend fun reload()`.
  - `class ReinsApp : Application` with `container`; extension `val Context.container: AppContainer`.

- [ ] **Step 1: Write the failing tests**

This pins Review Focus "main-thread blocking": the test fails if any `CoreApi` method (checked by reflection against the interface) or the delegate's construction runs on the calling thread, or if a 300 ms synchronous prefix stalls the caller.

`android/app/src/test/java/dev/reins/android/core/MainSafeCoreApiTest.kt`:

```kotlin
package dev.reins.android.core

import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** Review Focus: main-thread blocking. Every call must execute on the core dispatcher. */
class MainSafeCoreApiTest {
    private val mainExecutor = Executors.newSingleThreadExecutor { Thread(it, "fake-main") }
    private val ioExecutor = Executors.newFixedThreadPool(2) { Thread(it, "core-io") }
    private val main = mainExecutor.asCoroutineDispatcher()
    private val io = ioExecutor.asCoroutineDispatcher()

    @After
    fun tearDown() {
        mainExecutor.shutdownNow()
        ioExecutor.shutdownNow()
    }

    /** Records (method, thread) for every call; blocks the thread in login like a KDF would. */
    private class RecordingCore : CoreApi {
        val seen = CopyOnWriteArrayList<Pair<String, String>>()
        private fun rec(name: String) {
            seen += name to Thread.currentThread().name
        }
        private val session = SessionInfo("https://s", "e@x.com")

        override suspend fun session(): SessionInfo? = session.also { rec("session") }
        override suspend fun login(serverUrl: String, email: String, password: String, totp: String?): SessionInfo {
            rec("login")
            Thread.sleep(300) // synchronous CPU-bound prefix, as Argon2 would be
            return session
        }
        override suspend fun logout() = rec("logout")
        override suspend fun registerDevice(fcmToken: String?) = rec("registerDevice")
        override suspend fun handlePush(kind: String, id: String) = rec("handlePush")
        override suspend fun sync(waitSecs: Int): List<PendingItem> = emptyList<PendingItem>().also { rec("sync") }
        override suspend fun pending(): List<PendingItem> = emptyList<PendingItem>().also { rec("pending") }
        override suspend fun approvalView(requestId: String): ApprovalView {
            rec("approvalView")
            return ApprovalView(requestId, "c", ApprovalKind.Read, null, emptyList(), null, 0)
        }
        override suspend fun approve(requestId: String, choice: ApprovalChoice) = rec("approve")
        override suspend fun deny(requestId: String) = rec("deny")
        override suspend fun pairingView(pairingId: String): PairingView {
            rec("pairingView")
            return PairingView(pairingId, "c", "h", listOf(1, 2, 3), 0)
        }
        override suspend fun answerPairing(pairingId: String, approve: Boolean, chosenCode: Int?, label: String?) =
            rec("answerPairing")
        override suspend fun grants(): List<GrantView> = emptyList<GrantView>().also { rec("grants") }
        override suspend fun revokeGrant(grantId: String) = rec("revokeGrant")
        override suspend fun connections(): List<ConnectionView> = emptyList<ConnectionView>().also { rec("connections") }
        override suspend fun revokeConnection(connectionId: String) = rec("revokeConnection")
        override suspend fun activity(limit: Int): List<ActivityEntry> = emptyList<ActivityEntry>().also { rec("activity") }
        override suspend fun gmailStatus(): GmailStatus = GmailStatus.Ready.also { rec("gmailStatus") }
    }

    @Test
    fun everyMethodAndConstructionRunOffTheCallingThread(): Unit = runBlocking {
        val recording = RecordingCore()
        var constructedOn: String? = null
        val api = MainSafeCoreApi(io) {
            constructedOn = Thread.currentThread().name
            recording
        }
        withContext(main) {
            api.session()
            api.login("https://s", "e@x.com", "pw", null)
            api.logout()
            api.registerDevice("tok")
            api.handlePush("req", "r1")
            api.sync(0)
            api.pending()
            api.approvalView("r1")
            api.approve("r1", ApprovalChoice(emptyList(), null))
            api.deny("r1")
            api.pairingView("p1")
            api.answerPairing("p1", true, 47, "label")
            api.grants()
            api.revokeGrant("g1")
            api.connections()
            api.revokeConnection("c1")
            api.activity(10)
            api.gmailStatus()
        }
        // Thread names carry " @coroutine#N" when coroutine debug mode is on, hence startsWith.
        assertTrue("constructed on $constructedOn", constructedOn!!.startsWith("core-io"))
        val interfaceMethods = CoreApi::class.java.declaredMethods.map { it.name }.toSet()
        assertEquals("every CoreApi method is exercised", interfaceMethods, recording.seen.map { it.first }.toSet())
        assertTrue(recording.seen.toString(), recording.seen.all { it.second.startsWith("core-io") })
    }

    @Test
    fun blockingCorePrefixDoesNotStallTheCaller(): Unit = runBlocking {
        val api = MainSafeCoreApi(io) { RecordingCore() }
        api.warmUp()
        withContext(main) {
            val login = async { api.login("https://s", "e@x.com", "pw", null) }
            val started = System.nanoTime()
            // A second job on the same single "main" thread must run while login's 300 ms block is in progress.
            val ranAfterMs = async { (System.nanoTime() - started) / 1_000_000 }.await()
            assertTrue("main thread stalled for $ranAfterMs ms", ranAfterMs < 100)
            login.await()
        }
    }

    @Test
    fun failedConstructionIsRetriedOnNextCall(): Unit = runBlocking {
        val attempts = AtomicInteger()
        val api = MainSafeCoreApi(io) {
            if (attempts.incrementAndGet() == 1) throw CoreFailure.Storage("disk busy")
            RecordingCore()
        }
        try {
            api.session()
            fail("expected Storage failure")
        } catch (_: CoreFailure.Storage) {
        }
        assertEquals("e@x.com", api.session()?.email)
        assertEquals(2, attempts.get())
    }
}
```

`android/app/src/test/java/dev/reins/android/state/DeviceRegistrarTest.kt`:

```kotlin
package dev.reins.android.state

import dev.reins.android.core.CoreFailure
import dev.reins.android.core.FakeCoreApi
import dev.reins.android.core.PendingItem
import dev.reins.android.core.PendingKind
import dev.reins.android.core.SessionInfo
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DeviceRegistrarTest {
    private val fake = FakeCoreApi().apply { currentSession = SessionInfo("https://s", "e@x.com") }
    private val state = AppState()

    @Test
    fun registersWithTokenFromSource() = runTest {
        val ok = DeviceRegistrar(fake, { "fcm-1" }, state).register()
        assertTrue(ok)
        assertEquals(listOf<String?>("fcm-1"), fake.registeredTokens.toList())
        assertEquals(DeviceStatus.Registered, state.device.value)
    }

    @Test
    fun explicitTokenWinsAndNullTokenIsAllowed() = runTest {
        DeviceRegistrar(fake, { null }, state).register()
        DeviceRegistrar(fake, { "ignored" }, state).register("fresh")
        assertEquals(listOf(null, "fresh"), fake.registeredTokens.toList())
    }

    @Test
    fun failureIsRecorded() = runTest {
        fake.registerFailure = CoreFailure.Network("offline")
        val ok = DeviceRegistrar(fake, { null }, state).register()
        assertFalse(ok)
        val status = state.device.value as DeviceStatus.Failed
        assertTrue(status.failure is CoreFailure.Network)
    }

    @Test
    fun pendingStoreKeepsNewestFirstAndDedupes() {
        val store = PendingStore()
        store.replace(listOf(item("a", 1), item("b", 3)))
        store.upsert(item("c", 2))
        store.upsert(item("a", 5))
        assertEquals(listOf("a", "b", "c"), store.items.value.map { it.id })
        store.remove("b")
        assertEquals(listOf("a", "c"), store.items.value.map { it.id })
    }

    private fun item(id: String, at: Long) = PendingItem(PendingKind.Request, id, "t", "s", at)
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd android && ./gradlew testDebugUnitTest --tests '*MainSafeCoreApiTest' --tests '*DeviceRegistrarTest'`
Expected: FAIL — `Unresolved reference 'MainSafeCoreApi'`, `'AppState'`, `'DeviceRegistrar'`, `'PendingStore'`.

- [ ] **Step 3: Write the wrapper and provider**

`android/app/src/main/java/dev/reins/android/core/MainSafeCoreApi.kt`:

```kotlin
package dev.reins.android.core

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.withContext

/**
 * Runs every core call — and the construction of the delegate — on [dispatcher] (Dispatchers.IO
 * in the app). UniFFI polls Rust futures on the calling thread, so without this the synchronous
 * part of an `async fn` (e.g. the KDF in `login`) would run on the main thread (plan Decision 7).
 * A failed construction is retried on the next call (Kotlin `lazy` does not cache exceptions).
 */
class MainSafeCoreApi(
    private val dispatcher: CoroutineDispatcher,
    create: () -> CoreApi,
) : CoreApi {
    private val delegate = lazy(LazyThreadSafetyMode.SYNCHRONIZED, create)

    private suspend fun <T> io(block: suspend CoreApi.() -> T): T =
        withContext(dispatcher) { delegate.value.block() }

    /** Constructs the delegate ahead of first use. */
    suspend fun warmUp() = io { }

    override suspend fun session() = io { session() }
    override suspend fun login(serverUrl: String, email: String, password: String, totp: String?) =
        io { login(serverUrl, email, password, totp) }
    override suspend fun logout() = io { logout() }
    override suspend fun registerDevice(fcmToken: String?) = io { registerDevice(fcmToken) }
    override suspend fun handlePush(kind: String, id: String) = io { handlePush(kind, id) }
    override suspend fun sync(waitSecs: Int) = io { sync(waitSecs) }
    override suspend fun pending() = io { pending() }
    override suspend fun approvalView(requestId: String) = io { approvalView(requestId) }
    override suspend fun approve(requestId: String, choice: ApprovalChoice) = io { approve(requestId, choice) }
    override suspend fun deny(requestId: String) = io { deny(requestId) }
    override suspend fun pairingView(pairingId: String) = io { pairingView(pairingId) }
    override suspend fun answerPairing(pairingId: String, approve: Boolean, chosenCode: Int?, label: String?) =
        io { answerPairing(pairingId, approve, chosenCode, label) }
    override suspend fun grants() = io { grants() }
    override suspend fun revokeGrant(grantId: String) = io { revokeGrant(grantId) }
    override suspend fun connections() = io { connections() }
    override suspend fun revokeConnection(connectionId: String) = io { revokeConnection(connectionId) }
    override suspend fun activity(limit: Int) = io { activity(limit) }
    override suspend fun gmailStatus() = io { gmailStatus() }
}
```

`android/app/src/main/java/dev/reins/android/core/CoreProvider.kt`:

```kotlin
package dev.reins.android.core

import dev.reins.android.AppContainer

/** Builds the real core. Called once, lazily, on Dispatchers.IO (inside [MainSafeCoreApi]). */
fun interface CoreFactory {
    fun create(container: AppContainer): CoreApi
}

/**
 * Process-wide factory, read once in ReinsApp.onCreate. Instrumented tests replace it in
 * ReinsTestRunner before the Application is created (plan Decision 9).
 */
object CoreProvider {
    /** Until Task 12 wires reins-core, the app runs on a signed-out in-memory core. */
    @Volatile
    var factory: CoreFactory = CoreFactory { FakeCoreApi() }
}
```

- [ ] **Step 4: Write state holders and the registrar**

`android/app/src/main/java/dev/reins/android/state/AppState.kt`:

```kotlin
package dev.reins.android.state

import dev.reins.android.core.CoreFailure
import dev.reins.android.core.SessionInfo
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

sealed interface SessionState {
    data object Loading : SessionState
    data object SignedOut : SessionState
    data class SignedIn(val info: SessionInfo) : SessionState
    /** The core could not be opened (e.g. storage failure); nothing works until restart. */
    data class Broken(val failure: CoreFailure) : SessionState
}

sealed interface DeviceStatus {
    data object Unknown : DeviceStatus
    data object Registered : DeviceStatus
    data class Failed(val failure: CoreFailure) : DeviceStatus
    /** Another phone registered as the approval device (push "replaced" or HTTP 403). */
    data object Replaced : DeviceStatus
}

/** Process-wide UI-relevant state. Mutated only through these methods. */
class AppState {
    private val _session = MutableStateFlow<SessionState>(SessionState.Loading)
    val session: StateFlow<SessionState> = _session.asStateFlow()

    private val _device = MutableStateFlow<DeviceStatus>(DeviceStatus.Unknown)
    val device: StateFlow<DeviceStatus> = _device.asStateFlow()

    fun signedIn(info: SessionInfo) {
        _session.value = SessionState.SignedIn(info)
    }

    fun signedOut() {
        _session.value = SessionState.SignedOut
        _device.value = DeviceStatus.Unknown
    }

    fun broken(failure: CoreFailure) {
        _session.value = SessionState.Broken(failure)
    }

    fun setDevice(status: DeviceStatus) {
        _device.value = status
    }
}
```

`android/app/src/main/java/dev/reins/android/state/PendingStore.kt`:

```kotlin
package dev.reins.android.state

import dev.reins.android.core.PendingItem
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/** Items awaiting the user, newest first. Fed by sync/pending() and the core's Notifier. */
class PendingStore {
    private val _items = MutableStateFlow<List<PendingItem>>(emptyList())
    val items: StateFlow<List<PendingItem>> = _items.asStateFlow()

    fun replace(items: List<PendingItem>) {
        _items.value = items.sortedByDescending { it.createdAt }
    }

    fun upsert(item: PendingItem) {
        _items.update { list -> (list.filterNot { it.id == item.id } + item).sortedByDescending { it.createdAt } }
    }

    fun remove(id: String) {
        _items.update { list -> list.filterNot { it.id == id } }
    }
}
```

`android/app/src/main/java/dev/reins/android/state/DeviceRegistrar.kt`:

```kotlin
package dev.reins.android.state

import dev.reins.android.core.CoreApi
import dev.reins.android.core.CoreFailure

/** Source of the FCM token; returns null when Firebase is not configured. */
fun interface PushTokenSource {
    suspend fun token(): String?
}

/** Registers this phone as the approval device (contracts A1) and records the outcome. */
class DeviceRegistrar(
    private val core: CoreApi,
    private val tokens: PushTokenSource,
    private val state: AppState,
) {
    /** Uses [token] if given (onNewToken), else asks [tokens]. Returns true on success. */
    suspend fun register(token: String? = null): Boolean {
        val fcmToken = token ?: tokens.token()
        return try {
            core.registerDevice(fcmToken)
            state.setDevice(DeviceStatus.Registered)
            true
        } catch (e: CoreFailure) {
            state.setDevice(DeviceStatus.Failed(e))
            false
        }
    }
}
```

- [ ] **Step 5: Write the Play services / Firebase helpers**

`android/app/src/main/java/dev/reins/android/platform/TaskAwait.kt`:

```kotlin
package dev.reins.android.platform

import com.google.android.gms.tasks.Task
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine

/** Suspends until a Play services [Task] completes (replaces kotlinx-coroutines-play-services). */
suspend fun <T> Task<T>.await(): T = suspendCancellableCoroutine { cont ->
    addOnCompleteListener({ it.run() }) { task ->
        when {
            task.isCanceled -> cont.cancel()
            task.exception != null -> cont.resumeWithException(task.exception!!)
            else -> cont.resume(task.result)
        }
    }
}
```

`android/app/src/main/java/dev/reins/android/platform/FirebaseSupport.kt`:

```kotlin
package dev.reins.android.platform

import android.content.Context
import android.util.Log
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import dev.reins.android.BuildConfig
import kotlinx.coroutines.CancellationException

/** Firebase is optional (plan Decision 18): every entry point checks [available] first. */
object FirebaseSupport {
    fun available(context: Context): Boolean =
        BuildConfig.HAS_FIREBASE && FirebaseApp.getApps(context).isNotEmpty()

    /** Current FCM registration token, or null without Firebase or on failure. */
    @Suppress("DEPRECATION") // getToken() is deprecated in firebase-messaging 25.x but is still the documented way to read the token.
    suspend fun token(context: Context): String? {
        if (!available(context)) return null
        return try {
            FirebaseMessaging.getInstance().token.await()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Log.w("Reins", "FCM token unavailable", e)
            null
        }
    }
}
```

- [ ] **Step 6: Write the container and Application, register it in the manifest**

`android/app/src/main/java/dev/reins/android/AppContainer.kt`:

```kotlin
package dev.reins.android

import android.content.Context
import dev.reins.android.core.CoreFactory
import dev.reins.android.core.CoreFailure
import dev.reins.android.core.MainSafeCoreApi
import dev.reins.android.platform.FirebaseSupport
import dev.reins.android.state.AppState
import dev.reins.android.state.DeviceRegistrar
import dev.reins.android.state.PendingStore
import dev.reins.android.state.PushTokenSource
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/** Process singletons. Built in ReinsApp.onCreate; no DI framework (spec §6). */
class AppContainer(val context: Context, factory: CoreFactory) {
    val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val core = MainSafeCoreApi(Dispatchers.IO) { factory.create(this) }
    val state = AppState()
    val pending = PendingStore()
    val pushTokens = PushTokenSource { FirebaseSupport.token(context) }
    val registrar = DeviceRegistrar(core, pushTokens, state)

    /** Opens the core off the main thread and loads session + pending items. */
    fun start() {
        appScope.launch { reload() }
    }

    /** Re-reads session and pending items from the core (also used by tests). */
    suspend fun reload() {
        val session = try {
            core.session()
        } catch (e: CoreFailure) {
            state.broken(e)
            return
        }
        if (session == null) {
            pending.replace(emptyList())
            state.signedOut()
            return
        }
        try {
            pending.replace(core.pending())
        } catch (_: CoreFailure) {
            // Stale list is fine; the next sync refreshes it.
        }
        state.signedIn(session)
    }
}
```

`android/app/src/main/java/dev/reins/android/ReinsApp.kt`:

```kotlin
package dev.reins.android

import android.app.Application
import android.content.Context
import android.os.StrictMode
import dev.reins.android.core.CoreProvider

class ReinsApp : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        if (BuildConfig.DEBUG) {
            StrictMode.setThreadPolicy(
                StrictMode.ThreadPolicy.Builder()
                    .detectDiskReads()
                    .detectDiskWrites()
                    .detectNetwork()
                    .penaltyLog()
                    .build(),
            )
        }
        container = AppContainer(this, CoreProvider.factory)
        container.start()
    }
}

val Context.container: AppContainer
    get() = (applicationContext as ReinsApp).container
```

In `android/app/src/main/AndroidManifest.xml`, the `<application` start tag becomes:

```xml
    <application
        android:name=".ReinsApp"
        android:allowBackup="false"
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cd android && ./gradlew testDebugUnitTest assembleDebug`
Expected: PASS — `MainSafeCoreApiTest` 3, `DeviceRegistrarTest` 4, `FakeCoreApiTest` 6; `BUILD SUCCESSFUL`.

- [ ] **Step 8: Prove the main-thread test bites**

Temporarily change the body of `io` in `MainSafeCoreApi.kt` to `delegate.value.block()` (no `withContext`), run `./gradlew testDebugUnitTest --tests '*MainSafeCoreApiTest'`, expect 2 FAILED (`everyMethodAndConstructionRunOffTheCallingThread`, `blockingCorePrefixDoesNotStallTheCaller`), then restore `withContext(dispatcher) { delegate.value.block() }` and re-run to PASS.

- [ ] **Step 9: Commit**

```bash
git add android/app/src/main android/app/src/test
git commit -m "feat(android): main-safe core wrapper, app container and state"
```
