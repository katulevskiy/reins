# Integration connection completion

Connecting an account now ends with an explicit **Account connected** surface on Android and iOS. It shows the identity returned by the core, a **Done** action and, for services that support multiple accounts, **Add another account**. The add form stays hidden until the completion is acknowledged. Existing account rows and their checked status remain available below it.

This is also the finishing surface for the forthcoming typed account-permission choices. This change does not change permissions, grants, Auto mode or account admission and makes no claim that connecting an account changes those settings.

## When completion is shown

- Google: after `addAccount` / `addServiceAccount` returns its verified account, never just because the browser or chooser closed. Android checks chooser and consent result codes. Cancelled consent is consumed without registration.
- Token and vault: after the core accepts the secret and returns the account. The secret form is removed on success; rejected secrets keep an error and permit retry.
- Device services: after the operating system grants access and core registration succeeds. An empty Android permission result is not a grant. iOS prevents a second permission request while the first is pending.
- Telegram: after the returned `Done` account or the verified password result. Sending a code or asking for the second-factor password is not completion.
- MCP: the server detail shows **Server connected** only for a returned `status == "ok"`. A configured server with unfinished sign-in or a connection error retains its actual status. Failed sign-in never produces a success receipt. A verified result is retained even when a subsequent server-list refresh fails, so refresh failure does not require adding the server again.

The completion receipt belongs to the current Reins session and the returned integration account/server. It survives screen re-entry and Android activity recreation until acknowledged; removing that account/server or changing the Reins session clears it. It is UI state, contains no access token and is not written to disk. On a process restart, the core's saved account/server and its checked status remain the source of truth; the app does not replay OAuth or infer a fresh success from a changed account count. Results from an old session cannot repopulate completion in another session. Finishing an MCP add after navigating away does not force navigation back to that server.

## Validation

Android tests exercise verified identities, retry after rejection, Google consent cancellation, stale consent/results, Telegram/device completion, screen re-entry and activity recreation, acknowledgement, removal, session isolation and MCP error/usable outcomes. Existing MCP and Git-host flows remain covered. Native-graphics Robolectric renders below use the same unchanged Rust/UniFFI source as the branch base, with light/dark themes and 1.6× text. They are real Compose renders over a fake integration core, not physical-device captures.

iOS integration-model and UI tests cover the corresponding completion, cancellation, failure, removal and session behavior. The app, unit-test and UI-test targets are SDK-typechecked on Linux. An iOS simulator and live provider sign-in are unavailable on this host, so these tests and SwiftUI rendering require execution on macOS before claiming device coverage.


Visual evidence is attached to the pull request rather than stored in the repository.
