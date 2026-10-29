# iOS: App Store submission

Every release already goes to TestFlight (`.github/workflows/release.yml`, job `ios-testflight`;
[CONTRIBUTING.md](../CONTRIBUTING.md#maintainers-testflight)). Sending one of those builds to App Store review is a
separate, manual step: the **App Store submission** workflow (`.github/workflows/ios-app-store.yml`). It never runs on
a push, a schedule or another workflow.

What the workflow does, through the App Store Connect API (`scripts/testflight.py submit`):

0. Refuses, before any request (with `dry_run` too), unless the secret `ITS_ENCRYPTION_EXPORT_COMPLIANCE_CODE` is set:
   see "Export compliance" below.
1. Finds the build (`version`, `build_number`) of `com.reins2fa.app` and requires it to be processed (`VALID`), to
   declare non-exempt encryption (`usesNonExemptEncryption` true, from its Info.plist), and the app to have an
   encryption declaration with that code in the `APPROVED` state.
2. Stops if the app already has a review submission waiting for review, in review, with unresolved issues, or being
   cancelled.
3. Finds the iOS App Store version of `version`, or creates it. A version past "Prepare for Submission" (or a
   rejected one) is refused. Sets its release type: **`MANUAL` by default** (you release it by hand after approval),
   or `AFTER_APPROVAL` when chosen.
4. Attaches the build to the version and submits the version through a review submission (a draft submission of the
   app is reused). It never sets the build's export compliance answer: the build's Info.plist gives it.

With `dry_run` it does steps 0 to 3 as checks only and changes nothing. Apple refuses the final submit while
anything in the checklist below is missing, and the run's error names what Apple reported.

## Running it

1. Pick a release whose TestFlight build has been processed: App Store Connect > Reins 2FA > TestFlight shows the
   version (the release's, e.g. `1.2.3`) and the build number (minutes since 1970, e.g. `29860000`). The TestFlight
   job's log names both ("Reins 1.2.3 (29860000) is on TestFlight").
2. Actions > **App Store submission** > Run workflow, branch `main` (other refs are skipped). Enter `version` and
   `build_number`, keep `release_type` = `MANUAL`. Run once with `dry_run` checked, then again without it.
3. After approval, the version waits in "Pending Developer Release". Release it in App Store Connect (Distribution >
   the version > Release This Version) when you are ready.

## Export compliance

Reins uses standard, non-exempt encryption (end-to-end encryption between the phone, the server and the desktop) and
is distributed in France, so Apple needs an approved encryption declaration, including the French encryption
declaration, before any build is distributed:

- `ios/Reins/Info.plist` sets `ITSAppUsesNonExemptEncryption` to `true` (always).
- `ITSEncryptionExportComplianceCode` is **not** in the file. It is the build setting `REINS_EXPORT_COMPLIANCE_CODE`
  (empty in `ios/Config/Reins.xcconfig`), which the app target's "Export compliance" build phase
  (`ios/scripts/export-compliance.sh`, added by `ios/scripts/gen-project.py`) writes into the built Info.plist only
  when it is set. A Release **archive** without it fails (`REINS_EXPORT_COMPLIANCE_REQUIRED`); Debug, simulator, test
  and other builds just leave the key out. Locally, set it in `ios/Config/Local.xcconfig` (not committed).
- Only the app's Info.plist carries the keys; the widget and notification extensions do not (App Store Connect reads
  the app's).
- The release workflow's TestFlight job passes the code from the `testflight` environment secret
  `ITS_ENCRYPTION_EXPORT_COMPLIANCE_CODE` and fails before archiving while it is not set (a `verify_only` run archives
  without it and uploads nothing). `scripts/testflight.py` never sets a build's `usesNonExemptEncryption`.
- The App Store submission workflow passes the same secret to `scripts/testflight.py submit` as
  `EXPORT_COMPLIANCE_CODE`; without it the script refuses (exit 2) before any request, dry runs included.

Builds uploaded before this change declared `ITSAppUsesNonExemptEncryption` = `false` (and the TestFlight job set
`usesNonExemptEncryption` = false through the API when App Store Connect differed). Do not submit or distribute them:
submit a build archived with the code.

## Secrets

One new secret, `ITS_ENCRYPTION_EXPORT_COMPLIANCE_CODE` in the `testflight` environment (above): the code App Store
Connect shows for the approved encryption declaration. It is not a credential (it ends up in every build's Info.plist),
but it is kept out of the repository so a build cannot claim a declaration that is not approved. The job also uses the `testflight` environment's App Store Connect API key "Reins CI", the same secrets as
the TestFlight upload: `AC_API_KEY_P8`, `AC_API_KEY_ID`, `AC_API_ISSUER_ID` and `APPLE_TEAM_ID` (optional, defaults
to `5DRCQV8HY2`). The key's role is App Manager, which can create versions and submit them for review; if Apple answers
403, check the key's role in App Store Connect > Users and Access > Integrations. No signing certificate or profile is
used: nothing is built or signed.

Optional: add a required reviewer to the `testflight` environment (Settings > Environments > testflight > Required
reviewers) so each run waits for your approval. This also gates the TestFlight job.

## Checklist: App Store Connect, before the first submission

Most of this is one-time. Apple keeps it per app or per version as noted.

- [ ] **App record.** "Reins 2FA" for bundle id `com.reins2fa.app` exists (TestFlight uses it). Check the name,
  subtitle, primary language (en-US), primary and secondary category, SKU, and the support URL, marketing URL and
  privacy policy URL (App Information).
- [ ] **Version.** The workflow creates the version from `version` if it does not exist. Fill in, per version: the
  description, keywords, promotional text, support URL, copyright, and "What's New" (not asked for the first
  version). Only one version can be in "Prepare for Submission": if you created one by hand with another number,
  rename it to the release's version or delete it first.
- [ ] **Privacy nutrition label** (App Privacy). Declare what the app and the Reins server collect and whether it is
  linked to the user or used for tracking: account data (email via WorkOS sign-in), device push tokens (APNs),
  diagnostics if any, and what the calendar, contacts and Gmail integrations read on the phone or the server. The
  privacy policy URL is required. The privacy manifests for required-reason APIs (`PrivacyInfo.xcprivacy`) are a
  separate, pending pull request (#270); the label must match the manifests once merged, and builds uploaded before
  it lack them (Apple may warn or reject: submit a build from a release that includes #270).
- [ ] **Export compliance** (before any build is distributed; see "Export compliance" above). The app uses standard
  non-exempt encryption and is distributed in France:
  1. In App Store Connect (App Information > App Encryption Documentation), add the encryption documentation, answer
     that the app uses standard encryption algorithms and is available in France, and upload the French encryption
     declaration. Wait until Apple **approves** it.
  2. Copy the export compliance code App Store Connect shows for the approved declaration into the `testflight`
     environment secret `ITS_ENCRYPTION_EXPORT_COMPLIANCE_CODE` (Settings > Environments > testflight). Never commit
     it.
  3. Until then, the TestFlight job fails at "App settings" and this workflow refuses to submit (dry runs too). After
     it, release (or rerun the TestFlight job with `testflight_tag`) so a build with the code is uploaded, check
     TestFlight no longer shows "Missing Compliance" for it, and submit that build.
- [ ] **Age rating.** Answer the age rating questionnaire (App Information > Age Rating). Note any unrestricted web
  access (sign-in browser sessions) and user-generated content answers.
- [ ] **Screenshots per device size.** Required: iPhone 6.9" (1320 × 2868 or 1290 × 2796) or 6.5"; iPad 13"
  (2064 × 2752 or 2048 × 2732) because the app runs on iPad. Apple scales smaller sizes from these. The `-demo` launch
  argument shows sample data (`ios/README.md`), e.g. `ios/scripts/sim.sh "iPhone 18 Pro Max" --shot /tmp/reins.png --
  -demo`. Optional: an app preview video.
- [ ] **App Review information.** Contact name, phone and email, and **the demo account**. Apple's reviewers use
  `appreview@reins2fa.com`, a WorkOS user with email and password sign-in and no MFA (see "App access" in
  `android/PLAY_STORE.md`). Put its email and password in the sign-in fields; never commit the password. Confirm the
  account exists and where its password is kept (the Play twin's is in 1Password). Sign-in steps: Continue, enter the
  email on the WorkOS page, choose password sign-in, confirm the recovery code (any value), allow notifications, Skip
  setup. Never sign it in on a second phone (that opens Unlock). The reviewer also needs a way to see a request arrive;
  say how (for example, a request you send during review hours, or the demo video).
- [ ] **Review notes** explaining the entitlements and keys Apple asks about:
  - Background modes (`UIBackgroundModes`): `fetch`, `processing` and `remote-notification`: what each refreshes or
    processes in the background (approval requests arriving by push, the on-device model download, ...).
  - Time-sensitive notifications (`com.apple.developer.usernotifications.time-sensitive`): approval requests that
    expire within minutes and must break through Focus.
  - Live Activities with `NSSupportsLiveActivitiesFrequentUpdates`: a waiting request (Deny / Review in the Dynamic
    Island), a running bypass, the model download, and why they update often.
  - Local network use (`NSLocalNetworkUsageDescription`) and the camera (QR pairing with a computer).
  - Face ID: approving a request needs the user.
- [ ] **Pricing and availability.** Price (free), the countries and regions, and pre-orders (off).
- [ ] **Release choice.** Keep **Manually release this version** (the workflow's default `MANUAL`) for a quiet
  launch: after approval nothing goes live until you release it. Phased release is a separate per-version option
  (Version > Phased Release for Automatic Updates), off by default.
- [ ] **Build.** Submit only a build from a published release, already on TestFlight and tested there. A rejected
  version can take the same build again (or a newer one of that version) by running the workflow once the issue is
  fixed in App Store Connect.

## Not covered

- The workflow does not edit the listing, screenshots, privacy answers, age rating or review information; set them in
  App Store Connect.
- It does not release an approved version, and it does not cancel a submission.
- It has not run against App Store Connect yet: the API calls follow Apple's documented review submission flow and
  are tested only against a fake (`scripts/test-testflight.py`). Run it with `dry_run` first.
