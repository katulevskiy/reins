# Auth, downloads and release readiness

Review date: 2026-10-29. Source baseline: `88b193e` (Reins, release v0.29.0). The previous review was 2026-10-02 at
`97b9cd7`. Source implementation and production configuration are separate checks. Each status below was checked
against the repository, the release workflow logs (run
[40627531516](https://github.com/katulevskiy/reins/actions/runs/40627531516), v0.29.0) or the live URLs on
2026-10-29; anything that could not be checked that way says "unverified as of 2026-10-29".

## Implemented in source

- Both mobile apps use WorkOS AuthKit with PKCE. The live hosted authorize endpoint was also verified to redirect
  to WorkOS AuthKit at review time. WorkOS-backed Reins disables local password login, signup and
  password-token refresh automatically. WorkOS's configured email, social and passkey methods are accepted by
  default; passkey-only sign-in remains an explicit deployment option. Every Reins sign-in requires a WorkOS session
  identifier for revocation.
- Vault keys are random and encrypted on the phone, independent of the email address. Both apps require recording
  the recovery code and confirming its final group before continuing; the acknowledgment persists through relaunch
  and email changes without storing the recovery code in preferences.
- Another phone gets vault keys through an encrypted join approved by the existing phone, or through the recovery
  code. Identity login alone cannot take over approvals. Passkey synchronization belongs to Apple/Google or the
  user's chosen credential provider, rather than the Reins relay.
- WorkOS lifecycle events update verified email/name, revoke sessions, and delete accounts. Existing approval
  phones learn verified email changes through the normal poll, migrate their encrypted vault alias and grants
  atomically, and keep the same recovery code. Session revocation is
  transactional. Failed account cleanup is queued durably for retry while the account and tokens are disabled; it does not block
  later session revocations.
  A last organization owner still needs administrator intervention to finish deletion.
- The website (`../reins-site`) detects the phone platform and honors configured App Store/Google Play URLs.
  Desktop links start native browser downloads and open the QR companion dialog. Download state describes the
  browser handoff honestly: a web page cannot inspect completion of a native download or claim the app is installed.
- CI has one release gate, component-aware heavy jobs, shared native Android generation, parallel Rust test suites,
  and caches for release builds. Historical implementation recipes were removed; active specs, tests, licenses and
  production assets remain.
- Android production signing uses a newly generated non-debug RSA-4096 certificate pinned in
  `android/release-signing.sha256`. The historical debug key is preserved separately for migration/recovery.
  The release workflow fetches the production signing bundle from Infisical (`/signing/android`) via GitHub OIDC;
  this works in CI (v0.29.0: the Infisical step succeeded and the FullRelease APK and PlayRelease AAB were signed in
  the release run). Device testing remains.
- Google Play: v0.29.0 (versionCode 29885975) is released on the internal and closed testing (`alpha`) tracks.
  Open testing (`beta`) and production are refused with "Precondition check failed" until production access is
  granted; the upload job skips them with a warning and succeeds.
- TestFlight: v0.29.0 (29885975) was uploaded and internal testers have it. It was added to the external group but
  not submitted for beta app review: Apple takes only a few review submissions a day, so external review was skipped
  for 0.28.1 and 0.29.0 (0.28.0 was the last build submitted).
- The privacy policy, terms and account-deletion pages (`https://reins2fa.com/privacy`, `/terms`, `/delete-account`)
  answer 200.
- Releases require Android production signing and a successfully built APK. Publication verifies the installed
  name `Reins`, package `com.reins2fa.app`, release version and non-debuggable status. CI checks all tracked paths
  and file contents for obsolete branding.

## Required before production rollout

| Item | Missing or unverified |
| --- | --- |
| WorkOS login methods | Enable email codes, email/password, passkeys and configured social providers in the environment matching the live `SSO_CLIENT_ID`. Verify ordinary account signup and sign-in on Android/iOS. WorkOS passkeys require the hosted UI, and progressive enrollment is optional and skippable. |
| Legacy vault migration | New WorkOS accounts use random keys and mandatory recovery recording. Existing password-encrypted accounts retain their password for key unlock. Require an authenticated rewrap/migration using the original decryption credential before claiming every existing account is passwordless. |
| Existing sessions | If choosing a passkey-only deployment, enforcement applies to new sign-ins. Revoke existing non-passkey WorkOS/Reins sessions during that rollout so old sessions cannot bypass the policy. |
| WorkOS recovery | The Reins recovery code recovers encrypted vault keys, not a lost WorkOS identity. Confirm the identity recovery journey in WorkOS and explain this distinction to users. |
| Infisical access | Done: release signing is fetched from Infisical in CI (v0.29.0 release run). |
| GitHub release identity | Done: the repository variable `INFISICAL_RELEASE_IDENTITY_ID` is set and the release workflow uses it. Whether its access is limited to the signing folders is unverified as of 2026-10-29. |
| Historical Android package/certificate | Earlier builds used a different package id; current Reins uses `com.reins2fa.app`, a separate installation. Current-package test/direct builds signed with the old debug certificate also cannot update to the new production key. Verify recovery before any reinstall and provide an authenticated migration path; never silently discard encrypted data. |
| Android push/Google client | The repository secret `GOOGLE_SERVICES_JSON` is set and release builds use it. Still open **[owner action]**: register the Google-held Play app signing certificate with Firebase and create the Android OAuth client for its SHA-1 ([PLAY_STORE.md](../android/PLAY_STORE.md#signing-certificates)); until then Google integrations in the Play build are unverified. Gmail's restricted scopes need Google's verification and CASA assessment before production. |
| Play production signing | Release bundles are signed in CI with the upload key pinned in `android/release-signing.sha256` and uploaded to Play. Registering the app signing key elsewhere is in the row above. |
| Google Play production | Production access is not granted yet: `beta` and production fail "Precondition check failed". **[owner action]** Complete the closed-test requirement for new personal accounts (12 opted-in testers for 14 days) and apply for production access. Play Console forms still open: Data safety, content rating, the review demo account (app access), prominent disclosure, Age Signals use, and the foreground-service declaration video. The contact and support email placeholders in [PLAY_STORE.md](../android/PLAY_STORE.md) are not filled in. Console state is unverified as of 2026-10-29. |
| App Store | No App Store release path: builds reach TestFlight only. External TestFlight review depends on Apple's daily submission limit. The iOS privacy manifest is in PR #270 (open, not merged). Release gating on the iOS build and unit tests is in draft PR #272. Add the listing URL to the website store configuration once a listing exists. |
| Crash reporting | No crash reporting SDK in the apps, and the R8 `mapping.txt` is not uploaded to Play (CI keeps it only as a test-report artifact), so Play's crash stacks stay obfuscated. |
| Deployment | The release workflow deploys the server (the v0.29.0 deploy job succeeded). Website deployment is a separate repository and is unverified as of 2026-10-29. |
| Native verification | No recorded device pass for passkey, push and onboarding on Android or iOS. CI builds and unit-tests both apps, which does not establish those results. |

WorkOS constraints are documented in [passkeys](https://workos.com/docs/authkit/passkeys) and the
[authentication API](https://workos.com/docs/reference/authkit/authentication).
Signing setup is in [CONTRIBUTING.md](../CONTRIBUTING.md#maintainers-signing-the-apk-in-releases).

CI removes duplicated work, but its elapsed-time improvement must be measured on GitHub runners after the first
successful run; no speedup percentage is claimed from configuration changes alone.
