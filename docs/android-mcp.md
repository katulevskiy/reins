# Android MCP

Android MCP is Google's name for AppFunctions: an app acts as an on-device MCP server, and each function is one tool. The OS indexes the functions. A caller that Android has certified, and that holds `EXECUTE_APP_FUNCTIONS`, can discover them and run them. Gemini is that caller. The tools run on the phone.

The platform API is [AppFunctions](https://developer.android.com/ai/appfunctions). The Jetpack library is [`androidx.appfunctions`](https://developer.android.com/jetpack/androidx/releases/appfunctions) (Reins uses `1.0.0-beta01`). Samples and a testing agent live in [android/appfunctions](https://github.com/android/appfunctions). The agent skill is [device-ai/appfunctions](https://github.com/android/skills/tree/main/device-ai/appfunctions).

AppFunctions run on Android 16 (API 36) and higher. Jetpack's `AppFunctionManager.getInstance` also returns an instance on API 34 and 35 when the device ships the App Functions extension library, and it returns null for a profile user. Reins's service is API 36 only, so the Gemini switch exists on Android 16 and higher, on a full user. Gemini on Android reaches the functions on phones and tablets. Country and language follow the Play listing.

As of the [overview](https://developer.android.com/ai/appfunctions) (updated 22 Oct 2026), Gemini's use of AppFunctions is still described as a private preview that began in May 2026, and only a limited set of apps and system agents get the full path. On 28 Oct 2026 Google opened the [Connected Apps Platform](https://blog.google/innovation-and-ai/technology/developers-tools/connected-apps-one-place/) preview: a cloud MCP server, or Android MCP through AppFunctions, registered in the [Google Connected Console](https://developers.google.com/connected-apps). Access opens in phases.

## Why Reins exposes it

Reins is the phone-side approval gate. Claude Code, Grok, Codex and the others ask over MCP, and the person approves on the phone. Android MCP lets that person manage the gate by talking to Gemini, and some of it with the phone still locked:

- what is waiting
- deny one request, or every waiting request
- turn lockdown on
- leave Bypass, Overnight or Auto and go back to manual
- what the agents did
- which standing permissions are open, and revoke one

Approving stays on the Reins screen. Gemini opens that screen. It does not approve.

## Functions

Each function is one tool. The KDoc is what the agent reads. All of them ship disabled. The Settings switch labeled Gemini turns them on, and turning it off disables them again. The switch is off by default. Signed out, every call answers that the person needs to sign in to Reins first. Answers are for the profile the app is running in.

| Function | What Gemini gets back |
|---|---|
| `getStatus` | Mode (manual, auto, bypass, overnight, lockdown), when a timed mode ends, how many requests wait, how many AIs are connected. |
| `listWaitingRequests` | Newest first. Default 10, max 25. Id, which AI, which integration, a one-line summary, when it started waiting, when the AI stops waiting, and whether it could be approved quickly. |
| `openRequest` | A hand-off into Reins, on that request. Same destination as the notification. Unknown id: not found. |
| `denyRequest` | Denies that request. An optional note, cut at 500 characters. |
| `denyAllWaiting` | Denies every waiting request. Returns how many. |
| `startLockdown` | Turns lockdown on. Returns the new status. |
| `switchToManual` | Stops Bypass, Overnight or Auto and sets manual. Refuses while lockdown is on. |
| `searchActivity` | What the AIs did. Default 15, max 50. When, which AI, which integration, a one-line summary, the outcome, and who decided (you, autopilot, bypass, lockdown, or a standing permission). |
| `listStandingPermissions` | Id, which AI, which integration, one line on what it allows, when it expires. |
| `revokeStandingPermission` | Revokes that grant. |
| `listConnectedAis` | Id, name, last seen. |

There is no approve function, no disconnect, and nothing that grants, pairs, starts Bypass, Overnight or Auto, reveals a secret, or adds an account.

## Security model

The rule is the same as the Quick Settings tiles. An assistant may only take power away.

- Approving happens in Reins. `openRequest` returns a `PendingIntent` that opens that request. Gemini is told to use it and to stop claiming something was approved.
- `startLockdown` turns lockdown on. Nothing turns it off. `switchToManual` tightens Bypass, Overnight or Auto to manual, and refuses while lockdown is on, with "Lockdown is on; end it in Reins."
- Reads and denies (`listWaitingRequests`, `openRequest`, `denyRequest`, `denyAllWaiting`, `searchActivity`, `listStandingPermissions`, `revokeStandingPermission`, `listConnectedAis`) require the phone to be unlocked. Locked, they answer "Unlock the phone first." `getStatus`, `startLockdown` and `switchToManual` work while locked, as an emergency brake.
- Gemini can process on Google's servers, so the payload stays small. Summaries are one line, cut at 200 characters, with no message bodies. For the vault and for payments the summary is the operation title only: no item names, amounts, merchants or card data.
- Every function is disabled until the person turns the Gemini switch on.

## Test with adb

`adb shell cmd app_function` talks to the OS index. It does not need Gemini, and it does not need the app to hold `EXECUTE_APP_FUNCTIONS`. The shell user already has that permission. The device needs Android 16 or higher. Turn the Gemini switch on first, or the functions stay disabled and a call comes back disabled.

```sh
adb shell cmd app_function list-app-functions | grep -A10 com.reins2fa.app
```

The function id is the one in that list (`dev.reins.android.platform.assistant.ReinsAppFunctions#getStatus` and the rest). Run one:

```sh
adb shell "cmd app_function execute-app-function \
  --package com.reins2fa.app \
  --function '<id>' \
  --parameters '{...}'"
```

The parameter JSON matches the function's arguments. Official shape and a testing-agent app are in [Verify AppFunction integration](https://developer.android.com/ai/appfunctions/add-appfunctions).

## Release checklist

Gemini on a normal phone reaches Reins only after the Play app is an Android MCP connected app. Apply for the [Connected Apps preview](https://connected.google.com/console/help/?apply). Access is phased ([announcement](https://blog.google/innovation-and-ai/technology/developers-tools/connected-apps-one-place/), [platform page](https://developers.google.com/connected-apps)). The separate [AppFunctions early-access form](https://forms.gle/GN5ybjQFhzHRCguM7) registers interest in the agent pipeline. Submitting it does not grant access. Google emails if the app is selected.

The console steps are [Create and manage Connected Apps](https://developer.android.com/ai/appfunctions/create-manage-connected-apps) (updated 27 Oct 2026). In order:

1. An account owner accepts the Connected Apps terms. The person doing the work needs **Create and manage Connected Apps**, and release permission on the Play app.
2. The Play app already exists in the same account, with a default store listing (icon and languages). One Play app links to one Android MCP connected app. The link cannot be changed later.
3. Create a connected app. Pick the category once (it cannot be changed). Choose connection type **Android MCP** (it cannot be changed). Link `com.reins2fa.app`.
4. Save **App details** before uploading any bundle. Required: name (what people type after `@`), a 512×512 JPEG or PNG icon at most 1 MB, a description under 30 words, at least three capabilities, at least three example prompts, a policy URL and a terms URL. Website is optional.
5. Prompts: at least two start with `@Reins`, and at least one mentions Reins in the sentence with no `@`. No numbers, no quotes around the prompt.
6. Upload the `.aab` to an internal test track. Confirm the release page detected the functions (name, version code, tool count, languages, countries) and that the checks are clean. **Submit connected app and continue.** Leave the Play release as a draft while iterating, or that version code is stuck.
7. Add testers by `@gmail.com` address, including your own. They can take about 10 minutes to show up. Each tester is on an Android phone or tablet, signed in with that account, on the exact submitted version. In Gemini: `@Reins …`, or **Settings & help > Connected apps**.
8. Publish by shipping that bundle to Play **production**. Android MCP has no separate publish button. Google reviews it with the Play release, which can make review slower. After rollout it is available to Gemini on Android in the listing's countries and languages. Until then only the testers see it.

Capabilities to enter, one per line:

- Show what your agents are waiting on
- Deny a waiting request, or all of them
- Lock Reins down, and return Bypass or Overnight to manual
- Show what your agents did, and revoke a standing permission

Example prompts:

- `@Reins what are my agents waiting on?`
- `@Reins deny the GitHub push`
- `What did my agents do this morning in Reins?`

Policy URL `https://reins2fa.com/privacy`. Terms URL `https://reins2fa.com/terms`. The Play listing already uses these. A description that fits: "Shows what your AI agents are waiting on, denies requests, and locks Reins down. Approving stays in the app."

On the phone, the person still turns on Settings > Gemini, and connects the app in Gemini. With the switch off, the index shows the functions and calls return disabled.

## Next

### Reins as a caller

The other direction is a Phone apps integration. A remote AI that already talks to Reins over MCP (Claude Code and the rest) could search the phone's other apps and call their AppFunctions, with Reins asking the person on each call.

Two tools:

- `android_apps_search` finds apps and functions on the phone.
- `android_app_call` runs one function. Arguments are a JSON object. The approval preview shows the app, the function and the arguments. Anything destructive is once-only, the same rule as a one-shot action. Results are truncated before they go back to the agent.

This is blocked on the permission, on every current definition of it.

A caller that wants other packages' functions needs `EXECUTE_APP_FUNCTIONS`. The [public reference](https://developer.android.com/reference/android/Manifest.permission#EXECUTE_APP_FUNCTIONS) (API 36) lists the protection level as `normal`, and then says allowlist checks run at execution time, with further requirements from the allowlist metadata. Declaring the permission is not enough.

[AOSP `main`](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/core/res/AndroidManifest.xml), `frameworks/base/core/res/AndroidManifest.xml`, still defines it as `internal|role`, behind `enable_app_function_manager`, `@SystemApi` and `@hide`. The comment grants it to preinstalled system apps in the Assistant role. `EXECUTE_APP_FUNCTIONS_TRUSTED` is the same protection level, for system packages in the System UI Intelligence role, and target apps cannot opt out of that caller.

[AOSP `android-latest-release`](https://cs.android.com/android/platform/superproject/+/android-latest-release:frameworks/base/core/res/AndroidManifest.xml) splits `EXECUTE_APP_FUNCTIONS` on `app_function_access_api_enabled`:

- Flag on: protection level `normal`. The comment still requires the caller to be on a device allowlist, and execution is subject to the user's approval.
- Flag off: protection level `internal|privileged`, granted to privileged system apps.

On that same tree, behind `enable_app_function_permission_v2`:

- `DISCOVER_APP_FUNCTIONS` is `internal|role`, for the System UI Intelligence and System Vendor Intelligence roles. It can search. It cannot execute.
- `EXECUTE_APP_FUNCTIONS_SYSTEM` is `internal|recents|role`, for the recents component and the System UI role. It can search and execute.

"Certified by Android" is the name of the broadest function access level, `ACCESS_LEVEL_ANDROID_TRUSTED` (API 37.2). The [platform docs](https://developer.android.com/reference/android/app/appfunctions/AppFunctionMetadata) define it as a caller certified by Android that holds `EXECUTE_APP_FUNCTIONS`. There is no program a Play app joins to become that caller. An app can always run its own functions. Cross-app search and execute stay with the certified caller.

An [open issue filed 26 Oct 2026](https://issuetracker.google.com/issues/578107704) reports the same wall on current devices: only preinstalled first-party assistants can discover and invoke. The permission has no `development` flag, so `adb shell pm grant` does not hand it to a normal package. `adb shell cmd app_function` works because the platform grants the permission to the shell user, which is how Reins tests its own functions.

Until a Play app can be the certified caller, `android_apps_search` and `android_app_call` have nothing to call. Reins stays a provider.

### iOS

The iOS counterpart is App Intents. An app exposes actions to the system, and Siri runs them. Shortcuts and Spotlight see the same intents. Apple does not document a way for one App Store app to search and invoke another app's intents, so a Phone apps caller has the same shape of block there.

Parity for Reins is the same set of functions, with the same rule: show what waits, deny, lock down, revoke, and hand approval back to the Reins screen.
