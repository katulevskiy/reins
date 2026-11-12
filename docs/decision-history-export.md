# Exporting your decision history

The phone app can save your decision history (the Activity list: what agents asked for and how each request was
decided) as a CSV or JSON file. Organizations have a separate, signed audit log; see [the org audit log](audit.md).

## How

In **Settings**, tap **Export decision history** (iPhone: under Data; Android: under This phone). Confirm with Face ID,
fingerprint or the screen lock, pick **Spreadsheet (CSV)** or **JSON**, then choose where to save the file. The file is named `reins-decisions-<date>.csv` or `.json`,
where the date is in UTC.

## What is in it

- **Only this phone.** The history lives on your phones, encrypted, and the server can't read it, so the export is
  built on the phone and never sent anywhere. A phone keeps the most recent 5,000 entries. A phone added later has only
  the entries that synced to it.
- **Decisions, not content.** Each entry says when it happened, the service and account, what kind of action it was,
  the operation in a few words, how it was decided (approved, denied or failed) and by whom (you, Autopilot, a rule, a
  Circle, …). It also has the agent, its stated purpose and reason, your note, and Autopilot's suggestion and
  confidence. An export never holds message or email bodies, recipients, subject lines, search queries, the full
  request, the approval as it was shown, or any secret.
- **The file isn't encrypted.** Keep it somewhere safe.

## Formats

**CSV** (RFC 4180, UTF-8, newest first). The columns are `time_utc, decision, decided_by, service, account, action,
summary, connection, agent, purpose, why, note, count, execution, autopilot_suggestion, autopilot_confidence`. A text
cell that starts with `=`, `+`, `-` or `@` gets a leading `'`, so a spreadsheet won't run it as a formula.

**JSON** has the full detail:

```json
{
  "format": "reins.decision-history",
  "version": 1,
  "exported_at": "2026-11-12T09:30:00Z",
  "scope": "this-device",
  "entries": [
    {
      "id": 4182,
      "at": "2026-11-12T09:12:44Z",
      "service": "gmail",
      "account": "me@example.com",
      "action": "send",
      "title": "Send an email",
      "outcome": "sent",
      "decision": "approved",
      "decided_by": "you",
      "connection_id": "c_…",
      "connection": "Claude Code",
      "count": 1,
      "request": { "agent": "claude-code", "purpose": "Reply to the invoice thread" }
    }
  ]
}
```

Times are RFC 3339 in UTC. Absent values are left out, never `null`. Other optional fields are `label`, `autopilot`
(`mode`, `suggested`, `confidence`, `risk`, `reason`), `note`, `grant_summary`, `hold` and `execution`. Readers should
ignore fields they don't know. `version` changes only when a field changes its meaning or is removed. The format is
defined in `crates/reins-core/src/activity_export.rs`.
