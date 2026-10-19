# How a request is written and marked (approved 2026-10-15)

Applies to every place a request appears, on all three platforms: the waiting field, waiting cards, Activity rows,
entry/detail pages, notifications, the tray, widgets and Live Activities. One formatter per platform builds it; screens
never assemble the sentence themselves. Every kind of request has a test.

## One sentence, four voices

| Part | Style | Example |
| --- | --- | --- |
| Kind (label line) | Geist Mono caps, tracked 0.16em, above the sentence | `GITHUB · PUSH · TODAY 02:07:55` |
| Who + verb + count | Instrument Serif, roman | git pushed 3 commits to |
| Target: the one thing acted on | Instrument Serif *italic*, the only italic; short and human (owner/repo, person, folder, file, project · env) | *katulevskiy/reins* |
| Machine details: command, branch, host, path, SHA, address, card | Geist Mono, on its own line under the sentence, never inside the serif | `signal/desktop-release · new branch · github.com` |

- Waiting requests are present tense ("Push 3 commits to *octo/app*"); decided ones past tense ("git pushed 3 commits
  to *katulevskiy/reins*"). On a waiting field the asker is in the top bar, so the sentence starts with the verb.
- Commands, URLs, branches and paths never go into the serif. The core supplies a command presentation: fixed words
  such as "Run Gradle", the actionable command in mono, setup with environment values concealed, and visible
  notices about runtime context or limitations. All remaining commands and operators stay together; unsupported
  syntax says "Review a shell command" and requires inspecting the full invocation. The complete original stays
  available behind an explicit inspection control (it may contain secrets). Presentation never defines what an
  approval or standing permission covers. A permitted desktop command says "Approved running …" in history:
  the approval is not an executor's report that it actually ran. Older labels retain their legacy wording.
- Branches: the same source and destination shows once (`feature/x · new branch`); different ones `feature/x → main`.
- Counts and amounts are roman serif ("3 commits", "$42.10"); the target stays the only italic.
- Nothing the user approves is ever truncated: long values wrap or open in full.

## The icon: integration + action

Every request's icon is the integration's mark with the action as a small sub-icon at its bottom-right corner, as the
phones showed before Signal:

- Main mark: the provider/agent logo (GitHub, GitLab, Gmail, Calendar, Telegram, MCP server, Vault, Payments) on its
  white plate, or a Signal glyph for local kinds (terminal for commands, file for files, key for API keys/SSH, lock
  for secrets, help for questions).
- Sub-icon: a round badge at the bottom-right corner, filled with the request's kind colour at full strength (the
  field colour: push orange, read blue, delete red, grant violet, pair teal, accounts pink, warn amber), carrying the
  action glyph in white (send/branch for push, eye for read, search for search, trash for delete, plus for create,
  pencil for edit, play for run, key for secrets). Ringed in the surface colour so it separates from the mark.
- Same component in rows, detail headers, waiting cards and notifications (as large as the surface allows). It
  replaces the single tinted kind tile.

## Pushes: commit inspection

A push entry lists its commits (short SHA in mono, subject, +/−, file count); a commit opens its files with +/−; a
file opens its coloured diff inline. The desktop keeps commits and per-file stats for every approved push and diff text
up to about 1 MB per push for 30 days, in a file only the user can read. Phones already keep the approval's push view
in Activity.
