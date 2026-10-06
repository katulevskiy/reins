# Auth, downloads and release readiness

Review date: 2026-10-02. Source baseline: `97b9cd7` (Reins) and the merged download-page/Infisical changes in
`katulevskiy/reins-site`. Source implementation and production configuration are separate checks.

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
  The release workflow can fetch the prepared production signing bundle from Infisical via GitHub OIDC.
  Optimized, non-debuggable FullRelease APK and signed PlayRelease AAB were built locally and their signatures
  verified against that production certificate. The APK passed 16 KB ZIP alignment verification. These local review
  artifacts have not been uploaded or published; device testing and store enrollment remain.
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
| Infisical access | The configured project returned HTTP 404 in both dev and prod. Signing secrets could not be imported or inspected. Sign in again, restore project access, import the prepared private signing bundle and configure the release machine identity. |
| GitHub release identity | Set `INFISICAL_RELEASE_IDENTITY_ID` and bind the identity to this repository's release workflow on `main`, with access limited to `/signing/android`. There were no GitHub repository secrets or variables at review time. |
| Historical Android package/certificate | Earlier builds used a different package id; current Reins uses `com.reins2fa.app`, a separate installation. Current-package test/direct builds signed with the old debug certificate also cannot update to the new production key. Verify recovery before any reinstall and provide an authenticated migration path; never silently discard encrypted data. |
| Android push/Google client | The local `google-services.json` has no client for `com.reins2fa.app`. Supply the correct Firebase/Google configuration and register the new production certificate and, for Play, the actual Google-held app signing certificate. Tests without Firebase do not verify push or Google integration. |
| Play production signing | A signed `playRelease` AAB was built and verified locally with the dedicated production upload key. Enroll in Play App Signing, register the distributed certificate with Firebase/Google, upload the bundle and complete native/store verification. Google-held app signing keys may differ from the upload/direct-APK key. |
| App Store and Play Store | No real App Store listing or configured Google Play listing is available. Add actual listing URLs in the website store configuration. Android currently falls back to the signed APK; iOS must show availability until its listing exists. |
| Deployment | Merge and deploy the Reins and website changes. Code changes do not update the running server or website automatically in this working session. |
| Native verification | iOS needs an Xcode build and device/simulator onboarding check; Android needs a physical-device passkey/push check. Linux-only source validation cannot establish those results. |

WorkOS constraints are documented in [passkeys](https://workos.com/docs/authkit/passkeys) and the
[authentication API](https://workos.com/docs/reference/authkit/authentication).
Signing setup is in [CONTRIBUTING.md](../CONTRIBUTING.md#maintainers-signing-the-apk-in-releases).

CI removes duplicated work, but its elapsed-time improvement must be measured on GitHub runners after the first
successful run; no speedup percentage is claimed from configuration changes alone.
