# Bug triage

Beta bugs are GitHub issues in this repository. Testers use the
[bug report form](../.github/ISSUE_TEMPLATE/bug_report.yml). A teammate files that same form when a report
arrives by email or a personal message. [`.github/labels.yml`](../.github/labels.yml) declares the labels.
[Sync labels](../.github/workflows/sync-labels.yml) creates or updates them when a maintainer runs it.

## How a report gets in

The form asks for the platform, the app version or build, the device and OS version, what happened, what
should have happened, the steps, and a severity. Screenshots or a recording, logs, and whether the build is a
beta (TestFlight, Google Play closed testing, the Android Full build, or another pre-release) are optional.

The form asks GitHub to add `bug` and `needs-triage`. `bug` is already on the repository. `needs-triage` sticks
once that label exists. Until then GitHub drops a label it does not know.

A report that arrives by email or a personal message is filed by a teammate, through the same form. Transcribe
what the tester saw. Start **What happened** with `Filed for a tester.` Leave out the conversation, their
address, secrets, tokens, a master password, a recovery code, and message contents.

Security reports stay private, as [SECURITY.md](../SECURITY.md) describes. The issue chooser links to that
private report. Blank issues are off, so the chooser offers the bug report form and that security link.

## Labels

Create each label below, or run **Sync labels** (Actions → Sync labels → Run workflow) after the workflow is
on the default branch. The workflow is manual only. A merge does not run it, and it does not delete a label
that the file does not list.

| Label | Apply it when |
| --- | --- |
| `needs-triage` | The form adds this. Remove it once severity is set and the platform and area labels that fit are on the issue. |
| `beta` | TestFlight, Google Play closed testing, the Android Full build, or another pre-release. A public release does not get this. |
| `platform:ios` | iPhone or iPad app. |
| `platform:android` | Android app. Play closed testing and the Full build (installed from a file) are both this label. |
| `platform:desktop` | Desktop app on Linux, macOS, or Windows. The device field says which operating system. |
| `platform:server` | Hosted or self-hosted server and relay. |
| `area:pairing` | Pairing a phone with a computer or another device. |
| `area:crypto` | Keys, encryption, signing, or the vault cipher. |
| `area:relay` | The relay between phones, desktops, and AI clients. |
| `area:install-update` | Install, update, TestFlight, Play, or desktop packages. |
| `area:notifications` | Push or local notifications. |
| `sev:blocker` | The blocker line under [Severity](#severity). Blocks a release. |
| `sev:major` | The major line under [Severity](#severity). |
| `sev:minor` | The minor line under [Severity](#severity). |
| `sev:cosmetic` | The cosmetic line under [Severity](#severity). |

Use one platform label and one severity label. Use every area label that fits. A vault screen or an AI
connection may fit none of the area labels. Leave area unset then.

**Not sure** on the platform question means leave the platform label off until someone can tell.

## Severity

The form's menu and the `sev:*` label descriptions use these lines.

| Severity | Definition |
| --- | --- |
| Blocker | Cannot install, sign in, pair, or approve, or this must not ship. Blocks a release. |
| Major | A main flow is broken. A workaround may exist. Does not by itself block a release. |
| Minor | Something is wrong and there is a workaround. Does not block a release. |
| Cosmetic | Look, layout, or wording only. Behavior is right. Does not block a release. |

An open `sev:blocker` blocks a release. Ship after that issue is closed, or after the label is removed and the
issue says why it no longer blocks.

## After triage

Leave `bug` on the issue. Leave `beta` when the report is from a pre-release build. Remove `needs-triage` when
severity is set and the platform and area labels that fit are on the issue.
