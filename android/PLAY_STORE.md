# Reins on Google Play

The checklist for publishing the `play` flavor, with the answer to give in each Play Console form. Items marked
**[legal review]** are drafts that someone responsible for the app's legal commitments must check before they are
entered; **[owner action]** is something only the account owner can do; **[unsure]** is an answer this document could
not settle from the code and Google's documentation alone.

## Two builds

| | `full` | `play` |
|---|---|---|
| Distributed as | APK on reins2fa.com (`scripts/release-android.sh`) | App bundle uploaded to Google Play (the release workflow) |
| Application id | `com.reins2fa.app` | `com.reins2fa.app` (the same) |
| Text messages (READ_SMS, SEND_SMS) | yes | no: no permission, the core is not offered the integration, the Integrations screen does not list it |
| Updates | in-app updater (REQUEST_INSTALL_PACKAGES, `latest.json`) | Google Play; no updater, no "App updates" channel, no background check |
| Signed by | dedicated production key (`android/release-signing.sha256`) | the same key as the **upload key**; Google Play re-signs what it delivers with the app signing key it holds (Play App Signing) |

Both have the same application id. Only one of them can be installed on a phone, and because the signing keys differ,
Android refuses to update one with the other: switching from the APK to Play (or back) means uninstalling first, which
deletes the phone's data (it is in no-backup storage and bound to the phone's Keystore). The user signs in again and
registers the phone again; grants, history and Autopilot's training stay on the old phone's data and are lost. A
separate id (`com.reins2fa.app.play`) would let both coexist, but would need a second Firebase app, a second Android
OAuth client and its own redirect scheme for no real benefit: one person needs one approval phone.

Historical APKs used a different application id. The package rename prevents an in-place update.
Existing `com.reins2fa.app` test/direct builds signed with the old debug certificate also require an explicit
migration/reinstall before using the new production key. Verify recovery first; uninstalling erases local phone data.

What the flavors change is kept in one place each:

- `app/build.gradle.kts`: `productFlavors` (dimension `distribution`) set `BuildConfig.HAS_SMS`,
  `BuildConfig.SELF_UPDATE` and `UPDATE_URL`.
- `app/src/full/AndroidManifest.xml`: READ_SMS, SEND_SMS, REQUEST_INSTALL_PACKAGES, the telephony feature and the
  installer's result receiver. `src/main` has none of them, so the `play` manifest cannot ask for them.
- `PhoneBridge.SERVICES` / `PhoneBridge.offers()`: the on-phone integrations the core is offered
  (`DeviceBridge.services()`), and what the Integrations screen lists.
- `AppContainer.updates` is null when `SELF_UPDATE` is false; the Settings section, the prompt, the background check and
  the notification channel all follow it.

## Building and uploading

Every release builds the bundle next to the APK (`.github/workflows/release.yml`, job `android`):

1. `./gradlew :app:assembleFullRelease :app:bundlePlayRelease` with the same native libraries, versionCode (minutes since
   1970), Telegram credentials and `google-services.json`. `bundlePlayRelease` is signed with the production key through
   the `REINS_UPLOAD_*` settings (`app/build.gradle.kts`; never the debug key).
2. `scripts/check-android-apk.py reins-<version>-play.aab --bundletool ... --cert-sha256 <pinned>` checks the bundle:
   exactly one signer, the certificate in `android/release-signing.sha256`, `jarsigner -verify` covers every entry,
   `bundletool validate`, then on the universal APK bundletool makes from it: package, version, label, not debuggable,
   both ABIs, 16 KB-aligned native code, targetSdk 36 or later, none of the permissions Play restricts (SMS, call log,
   REQUEST_INSTALL_PACKAGES, QUERY_ALL_PACKAGES, exact alarms, background location, full-screen intents, AD_ID, ...),
   no foreground-service type but `dataSync`, no cleartext traffic.
3. The bundle is a release asset, `reins-<version>-play.aab`, listed in `SHA256SUMS`.
4. Job `play`, after the GitHub release is published: `scripts/play-upload.py` uploads it to the track in the repository
   variable `PLAY_TRACK` (default `internal`; `alpha` is closed testing, `beta` open testing, `production`), with the
   status in `PLAY_RELEASE_STATUS` (default `completed`) and the release notes in `play/release-notes/<language>.txt`
   (`{version}` is replaced; 500 characters at most). Without a service account it is skipped with a notice.

The uploader (standard library and `openssl` only) signs a JWT with the service account's key, exchanges it for a
one-hour token, then does one edit: insert, upload the bundle, set the track's release, commit; on any error it deletes
the edit. Neither the key nor the token is printed; the workflow also masks the key's lines. `--dry-run` checks the
bundle, the notes and the key without network access; `scripts/test-play-upload.py` runs it against a fake Google
Play. By hand:

```sh
PLAY_SERVICE_ACCOUNT_JSON="$(cat key.json)" scripts/play-upload.py --bundle reins-0.3.0-play.aab --version 0.3.0 \
    --notes-dir android/play/release-notes [--track internal|alpha|beta|production] [--status completed|draft] \
    [--status inProgress --user-fraction 0.1] [--changes-not-sent-for-review] [--dry-run]
```

Exit code 3 means "Only releases with status draft may be created on draft app": see the first upload below.

The service account secret: Infisical project `reins-release`, environment `prod`, folder `/signing/play`, secret
`PLAY_SERVICE_ACCOUNT_JSON` = the whole JSON key file (one line: `jq -c . key.json`). The release identity already
reads the whole project; `INFISICAL_PLAY_PATH` overrides the folder, a repository secret of the same name is the
fallback.

Locally:

```sh
cd android
./gradlew bundlePlayRelease -Preins.versionCode=N -Preins.versionName=0.1.0 -Preins.build=0.1.0-N \
    -Preins.prebuiltNativeDir=/path/to/native   # optional: scripts/build-android-native.sh outputs
# → app/build/outputs/bundle/playRelease/app-play-release.aab, signed when REINS_UPLOAD_* (or reins.upload*) are set
./gradlew testFullDebugUnitTest testPlayDebugUnitTest
```

- **versionCode** must grow with every upload. Minutes since 1970 (about 29.9 million now) works for both builds.
- **Native code** is linked for 16 KB pages (Play requires it for apps targeting Android 15+); the bundle check
  verifies every 64-bit library, ONNX Runtime's included.
- **Target API**: `targetSdk = 36` (Google Play requires 36 for new apps and updates from 2026-08-31) and
  `compileSdk = 37`. Raise `PLAY_MIN_TARGET_SDK` in `scripts/check-android-apk.py` with Google's next requirement.

## First upload

Google Play accepts API uploads only for an app that already has a bundle, and only draft releases while the app has
never been published on any track. So:

1. **[owner action]** Play Console > Create app: name **Reins 2FA**, default language English (United States), App,
   Free, accept the declarations.
2. Take `reins-<version>-play.aab` from the newest GitHub release (or build it as above).
3. Test and release > Testing > Internal testing > Testers: create an email list (yourself first). Then Create new
   release. Play asks how to sign: keep **Use a Google-generated key** (Play App Signing). The bundle's certificate
   (`reinsrelease`, SHA-256 `61:ED:FC:4C:...:F1:C8`) becomes the registered upload key; from then on Play refuses bundles
   signed by anything else (the CI check enforces the same certificate). Upload the bundle, paste
   `play/release-notes/en-US.txt` (with the version), Save, Review release, **Start rollout to Internal testing**.
   If the Console wants setup tasks first, the Dashboard lists them ("Start testing now"): app access, ads, content
   rating, target audience, data safety, privacy policy, as answered below.
4. Test and release > App integrity > App signing: copy the **app signing key certificate** SHA-1 and SHA-256 (the
   key Google holds) and register them (see "Signing certificates" below).
5. **[owner action]** Service account: in a Google Cloud project, enable the *Google Play Android Developer API*;
   IAM > Service accounts > Create (no project roles); Keys > Add key > JSON. Play Console > Users and permissions >
   Invite new users: the service account's email; App permissions: Reins 2FA with **Release apps to testing tracks**
   (add **Release to production, exclude devices, and use Play App Signing** only when production uploads should be
   automatic). Invitation of a service account needs no acceptance.
6. Infisical: create the folder `/signing/play` in `reins-release` / `prod` and set `PLAY_SERVICE_ACCOUNT_JSON`
   (dashboard, or `infisical secrets set --projectId=e21d41de-d3f4-41bf-accb-b4c2df9f164d --env=prod --path=/signing/play
   PLAY_SERVICE_ACCOUNT_JSON="$(jq -c . key.json)"`). Delete the downloaded key file afterwards.
7. The next release uploads by itself. If its `play` job says the app is a draft (exit code 3), set the repository
   variable `PLAY_RELEASE_STATUS` to `draft` and roll each release out by hand in the Console until the first rollout
   went through, then delete the variable. A failed `play` job never affects the GitHub release; rerun the job.
8. Later: `PLAY_TRACK=alpha` for closed testing, then production (below). Staged production rollouts are by hand
   (`--status inProgress --user-fraction`) or in the Console.

**New personal developer accounts** (created after 2023-11-13) must run a closed test with at least 12 opted-in testers
for 14 days in a row before they can apply for production access. Organization accounts are exempt.

## Signing certificates

Play App Signing means the phones see the **app signing key**, not the upload key. After the first upload, add its
SHA-1 and SHA-256 (App integrity page) next to the production key's, everywhere the app's identity is checked:

- **Firebase** (Project settings > Your apps > com.reins2fa.app > Add fingerprint): SHA-1 and SHA-256. Push (FCM) works
  without it, but keep the project's app registration complete; `google-services.json` does not change.
- **Google Cloud OAuth** (APIs and services > Credentials): an Android OAuth client holds one SHA-1, so create another
  **Android** client: package `com.reins2fa.app`, the Play app signing SHA-1. Without it Gmail, Google Calendar and Google
  Contacts show "needs setup" in the Play build. While the consent screen is in testing, add the testers' Google
  accounts as test users; restricted Gmail scopes need Google's verification (and its yearly CASA assessment) before
  production.
- **The server**: `REINS_ANDROID_CERT_SHA256` (comma-separated, `AB:CD:...`) lists every certificate allowed to open
  `https://app.reins2fa.com/pair` links (`/.well-known/assetlinks.json`): the production key's and the Play app signing
  key's. Restart the server; Android re-verifies App Links on install.
- **Android developer verification** (Android Developer Console): register `com.reins2fa.app` with both certificates,
  or the `full` APK stops installing where verification is enforced (2026-09 in Brazil, Indonesia, Singapore and
  Thailand, later worldwide). **[owner action]**

## Policy audit

Merged `play` release manifest (dependencies included), checked on a built bundle, against Google Play's policies as of
2026-10:

| Permission / feature | Where from | Play status | `play` |
|---|---|---|---|
| INTERNET, ACCESS_NETWORK_STATE, WAKE_LOCK | app, WorkManager, Firebase | OK (normal) | kept |
| POST_NOTIFICATIONS | app | OK (runtime) | kept |
| RECEIVE_BOOT_COMPLETED | app (grant reminders), WorkManager | OK | kept |
| USE_BIOMETRIC, USE_FINGERPRINT | androidx.biometric | OK | kept |
| com.google.android.c2dm.permission.RECEIVE | Firebase Messaging | OK | kept |
| `com.reins2fa.app.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION` | androidx.core (signature) | OK | kept |
| FOREGROUND_SERVICE, FOREGROUND_SERVICE_DATA_SYNC (WorkManager's `SystemForegroundService`, type `dataSync`) | app (Autopilot model download) | **Needs declaration** (Foreground service permissions form, with a video) | kept |
| READ_CALENDAR, WRITE_CALENDAR | app (Phone calendar) | OK; personal and sensitive data: prominent disclosure + Data safety | kept |
| READ_CONTACTS | app (Phone contacts) | OK; personal and sensitive data: prominent disclosure + Data safety | kept |
| READ_SMS, SEND_SMS | app (Text messages) | **Not allowed**: SMS and Call Log policy permits them only to the default SMS handler (or a listed exception, none of which fits) | removed |
| `android.hardware.telephony` (required=false) | app | OK (filtering only) | removed with SMS |
| REQUEST_INSTALL_PACKAGES + in-app APK updater | app | **Not allowed**: an app from Play must not update itself outside Play (Device and Network Abuse); the permission needs a core-use declaration that self-updating does not satisfy | removed |
| QUERY_ALL_PACKAGES | not requested | (would need a declaration) | absent |
| SCHEDULE_EXACT_ALARM / USE_EXACT_ALARM | not requested: grant reminders use inexact `setAndAllowWhileIdle` | (would need a declaration) | absent |
| ACCESS_BACKGROUND_LOCATION, any location | not requested | n/a | absent |
| Accessibility service, VPN service, MANAGE_EXTERNAL_STORAGE, USE_FULL_SCREEN_INTENT, READ_MEDIA_* | not used | n/a | absent |
| `com.google.android.gms.permission.AD_ID` | not requested, no ads or analytics SDK | Declare "no advertising ID" | absent |
| Cleartext (http) traffic | `usesCleartextTraffic="false"` in `src/main` (the default for targetSdk 28+, now explicit and checked) | OK | none |
| Exported `McpRedirectActivity`, `SsoRedirectActivity` (`com.reins2fa.app://mcp-oauth`, `://sso-callback`) | app | OK; they only hand a well-formed redirect to `MainActivity` | kept |
| Autopilot model download (~400 MB ONNX from reins2fa.com/models) | app | OK: data, not executable code; pinned by SHA-256 in the core | kept |
| ONNX Runtime telemetry (`ai.onnxruntime.TelemetryInitializer`) | onnxruntime-android | removed from the manifest (`tools:node="remove"`) | removed |
| Google code scanner (ML Kit, through Play services) | play-services-code-scanner | Collects device information, identifiers and diagnostics for Google's usage analytics (ML Kit data disclosure): declared in Data safety | kept |
| Gmail restricted scopes (`gmail.readonly`, `gmail.send`) | Google sign-in | Not a Play rule, but Google OAuth verification of restricted scopes, including a yearly security assessment (CASA) because the content reaches a server | owner action |

Other policy points:

- **User Data policy / prominent disclosure.** Calendar, contacts, email, Telegram, vault items and repository content
  leave the phone (to the user's Reins server and on to the user's AI) after the user approves each request or
  grant. The Integrations screen explains each service before Android's permission prompt; Play asks for an in-app
  disclosure *before* the runtime prompt that says what is collected, how it is used and that it is sent off the device.
  Check that the service screens' texts meet that, or add a disclosure dialog. **[legal review]**
- **Privacy policy** (required: the app handles personal and sensitive data): a public, non-PDF URL, linked in the
  listing and reachable from the app. The app links `https://reins2fa.com/privacy` and `https://reins2fa.com/terms`
  (`ui/common/Links.kt`), which answered 404 on 2026-10-08; the drafts are `docs/legal/`. Publish both before
  submitting. **[legal review] [owner action]**
- **Account deletion**: in the app (Settings) and on the web at `https://reins2fa.com/delete-account` (also 404 on
  2026-10-08: publish it; it must name the app or developer, list what is deleted and what is kept and for how long).
- **Android 15 `dataSync` limit**: a `dataSync` foreground service may run 6 hours a day. The model download is far
  shorter; a user-initiated data transfer job would avoid the declaration but is not needed.
- **Not a generative AI app** in Play's sense: Autopilot classifies requests on the phone and shows its verdict; it does
  not generate content for users. No AI-generated content declaration applies. **[unsure]** if Play's form asks.

## Play Console answers

Policy and programs > App content, in the order the Console lists them.

### Privacy policy

`https://reins2fa.com/privacy` (must be live first, see above).

### App access

**All or some functionality in my app is restricted.** Add one set of instructions:

- Name: Demo account
- Username: `playreview@reins2fa.com` · Password: in 1Password, "Reins Google Play review demo account" (a WorkOS
  user with email + password sign-in, no MFA, used only by Play's reviewers; Apple's have their own,
  `appreview@reins2fa.com`). **Never sign it in on another phone**: a second phone opens the Unlock screen instead.
- Any other information:

  > Reins lets a user approve, on their phone, what their AI assistants (Claude, ChatGPT, coding agents) may do with
  > their accounts. Sign-in is passwordless through our sign-in page (WorkOS).
  > 1. Open the app and tap Continue. The sign-in page opens in the browser: enter the email above, choose to sign in
  >    with a password, and enter the password above. The app opens again.
  > 2. The app shows a recovery code. Write it down (any value is fine for the review), tick the box, and type the last
  >    group of the code to confirm. If the app instead asks to unlock the account, write to support@reins2fa.com and
  >    we reset the demo account within a day.
  > 3. Allow notifications. The setup offers to connect a computer or an AI; "Skip setup" skips it.
  > 4. Requests from AI tools arrive in Activity; Grants lists standing permissions; Settings > Autopilot shows the
  >    on-phone model (the download is optional, about 400 MB). Integrations (top of Activity) connects Gmail, Google
  >    Calendar and Contacts, GitHub, Telegram, the phone's calendar and contacts, and the password vault; none is
  >    needed to review the app.
  > A request needs an AI connected to the account. A video of the whole flow (an AI asks, the phone approves) is at
  > `[VIDEO URL]`. No payment or subscription is required.

### Ads

**No, my app does not contain ads.**

### Content rating

Category: **All other app types** (a utility). Email for IARC: `[CONTACT EMAIL]`.

| Question | Answer |
|---|---|
| Violence, blood, sexuality, nudity, profanity or crude humor, drugs/alcohol/tobacco, gambling or simulated gambling | No to each |
| Fear / horror content | No |
| Does the app natively allow users to interact or exchange content with other users (chat, messaging, email)? | **Yes**: with the user's approval the app sends email (Gmail) and Telegram messages to other people **[unsure]** |
| Does the app allow users to share their physical location with other users? | No |
| Does the app allow users to purchase digital goods? | No |
| Is the app a web browser or search engine (unrestricted internet access)? | No (Custom Tabs are used for sign-in only) |
| Does the app contain user-generated content that is shared with other users? | No |

Expected: ESRB Everyone, PEGI 3, USK 0 and equivalents, with the interactive element "Users Interact".

### Target audience and content

- Target age groups: **13–15, 16–17 and 18 and over**, matching the published terms (https://reins2fa.com/terms: 13,
  or the minimum age in the user's country if higher). Never select an age group under 13: that puts the app under
  the Families policy.
- Could the app unintentionally appeal to children? **No.**
- Store listing presence: not designed for children.

### News apps

**No.**

### Data safety

"Collected" in Play's sense means sent off the device by the app, to the developer or anyone else; data sent to a
self-hosted server still counts, so the answers assume the worst case (the user uses app.reins2fa.com). Transfers the
user initiates (approving that a result goes to their AI) are not "sharing"; Google (Firebase Cloud Messaging, ML Kit)
acts as a service provider. **[legal review]** for the whole section.

- Does the app collect or share any of the required user data types? **Yes.**
- Is all of the user data collected by the app encrypted in transit? **Yes** (the core only talks to `https://` servers;
  plain `http://` only to localhost; Google, Telegram and the git hosts are HTTPS/MTProto; cleartext is off).
- Which ways can users create an account? **Username and other authentication** (passwordless email, passkeys,
  through WorkOS), **OAuth** (sign in with Google, through WorkOS), **Username and password** (self-hosted servers'
  email and master password).
- Delete account URL: **https://reins2fa.com/delete-account**. Users can delete their account **in the app** (Settings)
  and via that page; deleting removes the account, vault, approval-phone registration, AI connections and tokens
  (`docs/legal/privacy-policy.md`, "How long we keep data").
- Can users request that some data be deleted without deleting the account? **Yes** (removing an integration's
  account, an AI connection or a grant deletes it; signing out deletes the phone's copy).
- Independent security review: **No.** Committed to the Families policy: **No** (not for children).

| Play data type | Collected | Shared | Ephemeral | Required / optional | Purposes | What |
|---|---|---|---|---|---|---|
| Personal info > Name | yes | no | yes | optional | App functionality | contact names, email senders, Telegram chat names in results the user approves; the optional account name |
| Personal info > Email address | yes | no | no | required | App functionality, Account management | the account email (sign-in); addresses inside approved emails and contacts |
| Personal info > User IDs | yes | no | no | required | App functionality, Account management | the server account id, the device registration |
| Personal info > Phone number | yes | no | yes | optional | App functionality | the Telegram sign-in number (sent to Telegram); numbers in approved contacts |
| Personal info > Other info | yes | no | yes | optional | App functionality | password-vault fields released one at a time after approval |
| Messages > Emails | yes | no | yes | optional | App functionality | Gmail messages and attachments approved for the AI; emails the AI sends after approval |
| Messages > SMS or MMS | **no** (the `full` APK: yes) | | | | | |
| Messages > Other in-app messages | yes | no | yes | optional | App functionality | Telegram messages approved for the AI or sent by it |
| Photos and videos, Audio | no | | | | | (images inside emails or files count as Files and docs) |
| Files and docs | yes | no | **no** (large files are kept on the server up to an hour) | optional | App functionality | email attachments, repository files, uploads the phone checks |
| Calendar > Calendar events | yes | no | yes | optional | App functionality | phone calendar and Google Calendar events approved for the AI, events it creates |
| Contacts | yes | no | yes | optional | App functionality | phone contacts and Google Contacts approved for the AI |
| App activity > Other actions | yes | no | no | required | App functionality | approve/deny decisions and grants sent to the server so it can answer the AI |
| App info and performance > Diagnostics | yes | no | no | optional | Analytics | ML Kit (the QR code scanner, in Play services) reports performance and error data to Google **[unsure]** |
| Device or other IDs | yes | no | no | required | App functionality, Analytics | the FCM registration token and Firebase installation ID (push); ML Kit's device identifiers (scanner diagnostics) |
| Location, Health and fitness, Financial info, Web browsing, App info and performance > Crash logs, Installed apps, Search history | no | | | | | no crash reporting or analytics SDK of our own; ONNX Runtime's telemetry is removed |

Stays on the phone (not collected): the Google, Telegram, git host and vault credentials (in the encrypted store, bound
to the Keystore, excluded from backups); the activity history; Autopilot's model, its decisions and what it learns
(the model runs on the phone; only the download of the model package reaches reins2fa.com, without user data).

### Government apps, financial features, health

- Government app: **No.**
- Financial features: **My app doesn't provide any financial features.**
- Health apps: **My app does not have any health features.**

### Advertising ID

Does your app use advertising ID? **No.** (No AD_ID permission; the bundle check fails if a dependency adds it.)

### Foreground service permissions

One type: **Data sync** (`FOREGROUND_SERVICE_DATA_SYNC`, WorkManager's `SystemForegroundService`).

- Task: **Network transfer: upload or download** (other: "Download of a user-requested file").
- Description:

  > Reins can decide routine approval requests with Autopilot, an optional AI model that runs entirely on the phone.
  > The model (about 400 MB) is downloaded only when the user taps Download in Settings > Autopilot (on Wi-Fi unless
  > the user allows mobile data). The download runs as a foreground service with a notification that shows its
  > progress, so it finishes when the user leaves the app; the service stops as soon as the download completes, fails
  > or is cancelled. Nothing else in the app uses a foreground service.

- Impact if the system deferred or interrupted it: "Autopilot, which the user just asked for, stays unavailable, and an
  interrupted download of several hundred MB starts over, wasting the user's data and time."
- User-initiated: **yes**. Video: **[VIDEO URL]**: a short screen recording on a phone (Settings > Autopilot >
  Download, the progress notification in the shade, the download finishing). Upload it unlisted (YouTube, Drive with
  link access). **[owner action]**

### Permissions declarations

None of the declaration forms apply beyond the foreground service: no SMS/Call Log, All files access, Query all
packages, exact alarms, background location, accessibility, VPN, full-screen intents, photo/video or Health Connect
permissions (the bundle check enforces it). Calendar and contacts are ordinary runtime permissions: covered by the
prominent disclosure and Data safety. POST_NOTIFICATIONS needs nothing.

### Store settings

- App category: **Productivity**. Tags: Security, Productivity tools (pick from Play's list).
- Contact details: email `[SUPPORT EMAIL]` (required, shown publicly), website `https://reins2fa.com`.
- External marketing: on.

## Store listing

In `play/` (no fastlane needed; paste or upload them in Grow users > Store presence > Main store listing):

| File | Play field | Limit |
|---|---|---|
| `play/listing/en-US/title.txt` | App name: **Reins 2FA** | 30 characters (9) |
| `play/listing/en-US/short-description.txt` | Short description | 80 characters |
| `play/listing/en-US/full-description.txt` | Full description | 4000 characters |
| `play/release-notes/en-US.txt` | Release notes (the uploader fills them in) | 500 characters |
| `play/graphics/icon.png` | App icon | 512 × 512, 32-bit PNG, ≤ 1 MB |
| `play/graphics/feature-graphic.png` | Feature graphic | 1024 × 500, 24-bit PNG |
| `play/graphics/phone-screenshots/*.png` | Phone screenshots (8) | 1215 × 2160 (9:16), 24-bit PNG |

`play/graphics/render.sh` renders them: the icon from `icon.svg` (the launcher icon, full-bleed; Play rounds the
corners), the feature graphic from `feature-graphic.svg` (headless Chromium, with the app's Geist faces), and the
screenshots from the `play` build itself (`PlayStoreScreenshots`, Robolectric, dark theme, frozen clock; no text
messages or updater). `scripts/test-play-upload.py` (CI) checks every limit and that the listing never mentions text
messages, the APK or self-updating. Keep the text to what the app does (README "Features"); tablet screenshots are
optional.

The listing has no translations yet; add `play/listing/<language>/` and `play/release-notes/<language>.txt` together.
