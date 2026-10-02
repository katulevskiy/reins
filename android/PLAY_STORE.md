# Reins on Google Play

The checklist for publishing the `play` flavor. Nothing here has been submitted. Items marked **[legal review]** are
drafts that someone responsible for the app's legal commitments must check before they are entered in the Play Console.

## Two builds

| | `full` | `play` |
|---|---|---|
| Distributed as | APK on rewarden.arc-chat.com (`scripts/release-android.sh`) | App bundle uploaded to Google Play |
| Application id | `dev.rewarden.android` | `dev.rewarden.android` (the same) |
| Text messages (READ_SMS, SEND_SMS) | yes | no: no permission, the core is not offered the integration, the Integrations screen does not list it |
| Updates | in-app updater (REQUEST_INSTALL_PACKAGES, `latest.json`) | Google Play; no updater, no "App updates" channel, no background check |
| Signed by | the app's key (the script re-signs; SHA-1 `0dda03e6…`) | the upload key; Play re-signs with the app signing key it holds (Play App Signing) |

Both have the same application id. Only one of them can be installed on a phone, and because the signing keys differ,
Android refuses to update one with the other: switching from the APK to Play (or back) means uninstalling first, which
deletes the phone's data (it is in no-backup storage and bound to the phone's Keystore). The user signs in again and
registers the phone again; grants, history and Autopilot's training stay on the old phone's data and are lost. A
separate id (`dev.rewarden.android.play`) would let both coexist, but would need a second Firebase app, a second Android
OAuth client and its own redirect scheme for no real benefit: one person needs one approval phone.

What the flavors change is kept in one place each:

- `app/build.gradle.kts`: `productFlavors` (dimension `distribution`) set `BuildConfig.HAS_SMS`,
  `BuildConfig.SELF_UPDATE` and `UPDATE_URL`.
- `app/src/full/AndroidManifest.xml`: READ_SMS, SEND_SMS, REQUEST_INSTALL_PACKAGES, the telephony feature and the
  installer's result receiver. `src/main` has none of them, so the `play` manifest cannot ask for them.
- `PhoneBridge.SERVICES` / `PhoneBridge.offers()`: the on-phone integrations the core is offered
  (`DeviceBridge.services()`), and what the Integrations screen lists.
- `AppContainer.updates` is null when `SELF_UPDATE` is false; the Settings section, the prompt, the background check and
  the notification channel all follow it.

## Building

```sh
cd android
./gradlew assembleFullRelease          # the direct APK (scripts/release-android.sh does this and publishes it)
./gradlew bundlePlayRelease \
    -Prewarden.versionCode=N -Prewarden.versionName=0.1.0 -Prewarden.build=0.1.0-N \
    -Prewarden.defaultServer=https://rewarden.arc-chat.com
# → app/build/outputs/bundle/playRelease/app-play-release.aab
./gradlew testFullDebugUnitTest testPlayDebugUnitTest
```

- **Upload key.** Without it the bundle is signed with the debug key, which is fine for checking it locally and which
  Play rejects. Create an upload key (`keytool -genkeypair -v -keystore upload.jks -keyalg RSA -keysize 4096 -validity
  10000 -alias upload`), keep it outside the repository, and name it in `~/.gradle/gradle.properties`:
  `rewarden.uploadKeystore`, `rewarden.uploadKeystorePassword`, `rewarden.uploadKeyAlias`, `rewarden.uploadKeyPassword`.
  Enrol in Play App Signing (the default for new apps) and let Google generate the app signing key.
- **versionCode** must grow with every upload. The release script's scheme (minutes since 1970, about 29.8 million now)
  works for Play too, and keeps the two builds' numbers comparable.
- **google-services.json** must be present when building (see `app/build.gradle.kts`), or the bundle has no push.
- **Google sign-in and Firebase.** Add the SHA-1 *and* SHA-256 of the Play app signing key (Play Console > Test and
  release > App integrity) and of the upload key to the Firebase project and as Android OAuth clients in Google Cloud.
  Without that, Gmail, Google Calendar and Google Contacts fail with "needs setup" in the Play build.
- **Native code** is built with 16 KB page alignment (Play requires it for apps targeting Android 15+).
- **Target API**: `targetSdk = 36` meets Play's requirement for new apps and updates in 2026.

## Policy audit

Merged `play` release manifest (dependencies included) against Google Play's policies as of 2026-10:

| Permission / feature | Where from | Play status | `play` |
|---|---|---|---|
| INTERNET, ACCESS_NETWORK_STATE, WAKE_LOCK | app, WorkManager, Firebase | OK (normal) | kept |
| POST_NOTIFICATIONS | app | OK (runtime) | kept |
| RECEIVE_BOOT_COMPLETED | app (grant reminders), WorkManager | OK | kept |
| USE_BIOMETRIC, USE_FINGERPRINT | androidx.biometric | OK | kept |
| com.google.android.c2dm.permission.RECEIVE | Firebase Messaging | OK | kept |
| `dev.rewarden.android.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION` | androidx.core (signature) | OK | kept |
| FOREGROUND_SERVICE, FOREGROUND_SERVICE_DATA_SYNC (WorkManager's `SystemForegroundService`, type `dataSync`) | app (Autopilot model download) | **Needs declaration** (Foreground service permissions form, with a video) | kept |
| READ_CALENDAR, WRITE_CALENDAR | app (Phone calendar) | OK; personal and sensitive data: prominent disclosure + Data safety | kept |
| READ_CONTACTS | app (Phone contacts) | OK; personal and sensitive data: prominent disclosure + Data safety | kept |
| READ_SMS, SEND_SMS | app (Text messages) | **Not allowed**: SMS and Call Log policy permits them only to the default SMS handler (or a listed exception, none of which fits) | removed |
| `android.hardware.telephony` (required=false) | app | OK (filtering only) | removed with SMS |
| REQUEST_INSTALL_PACKAGES + in-app APK updater | app | **Not allowed**: an app from Play must not update itself outside Play (Device and Network Abuse); the permission needs a core-use declaration that self-updating does not satisfy | removed |
| QUERY_ALL_PACKAGES | not requested | (would need a declaration) | absent |
| SCHEDULE_EXACT_ALARM / USE_EXACT_ALARM | not requested: grant reminders use inexact `setAndAllowWhileIdle` | (would need a declaration) | absent |
| ACCESS_BACKGROUND_LOCATION, any location | not requested | n/a | absent |
| Accessibility service, VPN service, MANAGE_EXTERNAL_STORAGE, USE_FULL_SCREEN_INTENT | not used | n/a | absent |
| `com.google.android.gms.permission.AD_ID` | not requested, no ads or analytics SDK | Declare "no advertising ID" | absent |
| Exported `McpRedirectActivity` (`dev.rewarden.android://mcp-oauth`) | app | OK; it only hands a well-formed redirect to `MainActivity` | kept |
| Autopilot model download (~400 MB ONNX from rewarden.arc-chat.com/models) | app | OK: data, not executable code; pinned by SHA-256 in the core | kept |
| ONNX Runtime telemetry (`ai.onnxruntime.TelemetryInitializer`) | onnxruntime-android | removed from the manifest (`tools:node="remove"`) | removed |
| Gmail restricted scopes (`gmail.readonly`, `gmail.send`) | Google sign-in | Not a Play rule, but Google OAuth verification of restricted scopes, including a yearly security assessment (CASA) because the content reaches a server | owner action |

Other policy points:

- **User Data policy / prominent disclosure.** Calendar, contacts, email, Telegram, vault items and repository content
  leave the phone (to the user's Reins server and on to the user's AI) after the user approves each request or
  grant. The Integrations screen explains each service before Android's permission prompt; Play asks for an in-app
  disclosure *before* the runtime prompt that says what is collected, how it is used and that it is sent off the device.
  Check that the service screens' texts meet that, or add a disclosure dialog. **[legal review]**
- **Privacy policy** (required: the app handles personal and sensitive data): a public URL, linked in the listing and
  reachable from the app. **[legal review]**
- **Account deletion.** Accounts are created on the Reins (Vaultwarden) server, not in the app, so the in-app
  deletion requirement may not apply; Play still asks for a web link where users can request deletion of their account
  and data. Provide one for rewarden.arc-chat.com. **[legal review]**
- **App access for review.** Everything is behind sign-in: give the reviewers a demo server account (email, password,
  no two-step) and instructions to pair an AI, in Play Console > App content > App access.
- **New developer accounts** (personal, created after 2023-11-13) must run a closed test with at least 12 testers for
  14 days before applying for production.
- **Android developer verification** (enforced from 2026-09 in Brazil, Indonesia, Singapore and Thailand, later
  worldwide) also covers the `full` APK: register `dev.rewarden.android` and both signing certificates (the APK's key
  and the Play app signing key) in the Android Developer Console, or the APK will stop installing there. Note that the
  APK is currently signed with a debug keystore. **[owner action]**
- **Android 15 `dataSync` limit**: a `dataSync` foreground service may run 6 hours a day. The model download is far
  shorter; a user-initiated data transfer job would avoid the declaration but is not needed.

## Data safety form

Derived from the code (`crates/rewarden-core`, `android/app`). "Collected" in Play's sense means sent off the device by
the app, to the developer or anyone else; data sent to a self-hosted server still counts, so the answers assume the
worst case (the user uses rewarden.arc-chat.com). **[legal review]** for the whole section.

**Does the app collect or share user data?** Yes.

**Is all data encrypted in transit?** Yes: the core only talks to `https://` servers (plain `http://` is accepted for
localhost only); Google, Telegram and the git hosts are HTTPS/MTProto.

**Can users request deletion?** Yes: signing out deletes the phone's copy; uninstalling deletes everything on the phone;
the server account and its data are deleted on request (link above). **[legal review]**

**Shared with third parties?** No. Everything leaves the phone at the user's request to the user's own server and from
there to the AI the user paired; Play counts user-initiated transfers as not shared. Google (Firebase Cloud Messaging)
is a service provider. **[legal review]**

| Play data type | Collected | What and why | Optional | Ephemeral |
|---|---|---|---|---|
| Personal info > Email address | yes | the Reins account email (sign-in; App functionality, Account management); email addresses inside relayed email and contacts | required (account) | no |
| Personal info > Name | yes | contact names, email senders, Telegram chat names in relayed results (App functionality) | optional (per request) | yes |
| Personal info > Phone number | yes | Telegram sign-in goes to Telegram; phone numbers in relayed contacts (App functionality) | optional | yes |
| Personal info > User IDs | yes | the server account id and the device registration (App functionality, Account management) | required | no |
| Personal info > Other info | yes | password-vault items (usernames, single secrets) released one field at a time after approval | optional | yes |
| Messages > Emails | yes | Gmail messages and attachments the user approves for the AI; emails the AI sends after approval | optional | yes |
| Messages > Other in-app messages | yes | Telegram messages approved for the AI or sent by it | optional | yes |
| Messages > SMS or MMS | **no in `play`** (the `full` APK: yes) | | | |
| Files and docs | yes | files the user approves (email attachments, repository files, uploads checked on the phone) | optional | yes |
| Calendar > Calendar events | yes | phone calendar and Google Calendar events approved for the AI, events it creates | optional | yes |
| Contacts | yes | phone contacts and Google Contacts approved for the AI | optional | yes |
| App activity > Other actions | yes | approve/deny decisions and grants sent to the server so it can answer the AI (App functionality) | required | no |
| Device or other IDs | yes | the FCM registration token (sent to the Reins server to deliver approval requests) and the Firebase installation ID (Google, for push) | required for push | no |
| Location, Health, Financial info, Photos and videos, Audio, Web browsing, App info and performance (crash logs, diagnostics) | no | no analytics or crash reporting; ONNX Runtime's telemetry is removed from the manifest | | |

Stays on the phone (not collected): the Google, Telegram, git host and vault credentials (in the encrypted store, bound
to the Keystore, excluded from backups); the activity history; Autopilot's model, its decisions and what it learns
(the model runs on the phone; only the download of the model package reaches rewarden.arc-chat.com, without user data).

## Store listing and other forms

- **App category**: Productivity (or Tools). Tags: AI, security, privacy.
- **Content rating** (IARC questionnaire): a utility; no violence, sexual content, gambling, drugs or profanity of its
  own. It does let the user's AI send emails and Telegram messages to other people after the user approves each one:
  answer the "users can communicate" questions accordingly. Expected rating: Everyone / PEGI 3 with a "Users interact"
  notice. **[legal review]**
- **Target audience**: 18+ (not designed for children; avoids the Families policy). **[legal review]**
- **Ads**: none. **Advertising ID**: not used. **Government app / financial features / health / news**: no.
- **Foreground service declaration**: `dataSync`: "the user starts the download of Autopilot's on-device model (about
  400 MB) and sees its progress in a notification"; a short screen recording of starting the download.
- **Assets**:
  - app icon 512 × 512 PNG (32-bit, ≤ 1 MB);
  - feature graphic 1024 × 500 PNG/JPEG;
  - 2–8 phone screenshots (9:16, 1080 × 1920 or larger; `./gradlew testPlayDebugUnitTest --tests '*Screenshots*'
    -Drewarden.screenshots=/tmp/shots` renders the screens; do not show text messages or the updater);
  - optional 7" and 10" tablet screenshots;
  - short description (≤ 80 characters), full description (≤ 4000);
  - privacy policy URL, support email, website (https://rewarden.arc-chat.com).
- **Description**: do not mention text messages or self-updating for the Play listing.
