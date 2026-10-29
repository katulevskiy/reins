# iOS: App Store submission

Every release already goes to TestFlight (`.github/workflows/release.yml`, job `ios-testflight`;
[CONTRIBUTING.md](../CONTRIBUTING.md#maintainers-testflight)). Sending one of those builds to App Store review is a
separate, manual step: the **App Store submission** workflow (`.github/workflows/ios-app-store.yml`). It never runs on
a push, a schedule or another workflow.

What the workflow does, through the App Store Connect API (`scripts/testflight.py submit`):

1. Finds the build (`version`, `build_number`) of `com.reins2fa.app` and requires it to be processed (`VALID`).
2. Stops if the app already has a review submission waiting for review, in review, with unresolved issues, or being
   cancelled.
3. Finds the iOS App Store version of `version`, or creates it. A version past "Prepare for Submission" (or a
   rejected one) is refused. Sets its release type: **`MANUAL` by default** (you release it by hand after approval),
   or `AFTER_APPROVAL` when chosen.
4. Sets the build's export compliance from `ITSAppUsesNonExemptEncryption` in `ios/Reins/Info.plist` (as TestFlight
   does), attaches the build to the version, and submits the version through a review submission (a draft submission
   of the app is reused).

With `dry_run` it does steps 1 to 3 as checks only and changes nothing. Apple refuses the final submit while
anything in the checklist below is missing, and the run's error names what Apple reported.

## Running it

1. Pick a release whose TestFlight build has been processed: App Store Connect > Reins 2FA > TestFlight shows the
   version (the release's, e.g. `1.2.3`) and the build number (minutes since 1970, e.g. `29860000`). The TestFlight
   job's log names both ("Reins 1.2.3 (29860000) is on TestFlight").
2. Actions > **App Store submission** > Run workflow, branch `main` (other refs are skipped). Enter `version` and
   `build_number`, keep `release_type` = `MANUAL`. Run once with `dry_run` checked, then again without it.
3. After approval, the version waits in "Pending Developer Release". Release it in App Store Connect (Distribution >
   the version > Release This Version) when you are ready.

## Secrets

No new secrets. The job uses the `testflight` environment's App Store Connect API key "Reins CI", the same secrets as
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
- [ ] **Export compliance.** `ios/Reins/Info.plist` sets `ITSAppUsesNonExemptEncryption` to `false`, and the
  workflow answers "no non-exempt encryption" for the build from it. **Confirm this is right** before the first
  submission: the app uses end-to-end encryption between the phone, the server and the desktop. Apple's exemption
  covers standard algorithms used only for authentication or protecting user data, and France distribution has its own
  rules. If the answer is "yes", set the key to `true`, upload the documentation in App Store Connect (App
  Information > App Encryption Documentation), and the workflow then leaves the build's answer to you.
- [ ] **Age rating.** Answer the age rating questionnaire (App Information > Age Rating). Note any unrestricted web
  access (sign-in browser sessions) and user-generated content answers.
- [ ] **Screenshots per device size.** Required: iPhone 6.9" (1320 × 2868 or 1290 × 2796) or 6.5"; iPad 13"
  (2064 × 2752 or 2048 × 2732) because the app runs on iPad. Apple scales smaller sizes from these. The `-demo` launch
  argument shows sample data (`ios/README.md`), e.g. `ios/scripts/sim.sh "iPhone 18 Pro Max" --shot /tmp/reins.png --
  -demo`. Optional: an app preview video.
- [ ] **App Review information.** Contact name, phone and email, and **a demo account**. Sign-in is WorkOS with a
  passkey on the hosted server, which a reviewer cannot use with your credentials: decide how review signs in (a
  review account Apple can create a passkey for, or a review server reachable under "Use another server" with
  password sign-in) and describe it step by step. The reviewer also needs a paired computer or a way to see a request
  arrive; say how (for example, a request you send during review hours).
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
