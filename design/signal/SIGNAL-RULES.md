Signal is the Reins app's style for iPhone and Android. It has one rule: **a solid field of colour appears only when something needs your decision.** Everything else is paper, ink and hairlines. When the screen turns orange, blue or red, you know before reading a word that an AI is asking you something, and what kind of thing it is.

Signal grows out of the Reins design system (the same Geist faces, the same violet, the same glyphs). It adds Instrument Serif, the face from reins.dev, as the voice for asks, titles and numbers, so the app and the site sound alike.

## Principles

- **Colour means it needs you.** A waiting request, an open approval, an active grant, a bypass that's on, a broken setting: each gets a field. A request that's been decided drops to paper with a small coloured tile.
- **Fields stay, paper flips.** Light and dark share exactly the same field colours. Only `paper`, `ink` and their neighbours change. A red delete looks the same at noon and at midnight.
- **Three voices.** Instrument Serif (`ask`, `title`, `stat`) for what you decide on. Geist for everything you read. Geist Mono in capitals (`label`) for labels and machine facts.
- **Say exactly what will happen.** The ask is one sentence, imperative, with the target in italic: "Push 8 commits to *octo/app*", "Delete the repo *octo/old-api*". The facts below it are plain rows, not a summary.
- **Two buttons decide.** Deny and the action. Anything that can't be undone is held, not tapped.

## Colour

**Fields** (the same in both themes, white text at 4.9:1 or more):

| Field | Means | Examples |
| --- | --- | --- |
| `field-push` | writes or sends | git push, send email, comment |
| `field-read` | reads or searches | read emails, see calendar |
| `field-delete` | can't be undone; bypass | delete repo, force-push, pay, bypass on |
| `field-grant` | power already handed over | a grant's page |
| `field-pair` | connects something new | pair a phone, connect an AI |
| `field-accounts` | changes an account | passwords, recovery code |
| `field-warn` | Reins is broken and needs you | notifications off, another phone approves |

- On a field, text is `on-field`, the label line and details are `on-field-2` (14px and up only), and translucent buttons are `on-field-wash`. Review is an `on-field` pill with ink text.
- **Paper:** screens are `paper`, sheets and inputs `paper-raised`, wells and soft buttons `paper-sunken`. Rows divide with `rule` hairlines, never cards.
- **Ink:** `ink` for titles and values, `ink-2` for keys and body, `ink-meta` for meta, `ink-faint` only for chevrons and empty rings.
- **Violet** is Reins and Autopilot: `violet` for text and glyphs on paper (links, Auto tags, "Allowed by Autopilot"), `violet-fill` for switches, the Auto chip and badges. Violet is also the grant field, because a grant is Reins acting for you.
- Status on paper: `success` for additions and Ready, `danger` for deletions and destructive rows, `warning` for flags. Never let colour carry a status alone: the label line names the kind ("GITHUB · DELETE"), and outcomes carry a word.
- Never put a field colour on a non-decision surface (no orange headers, no blue tabs). Never use gradients.

## Type

- `ask` 45/46 serif for the request on a field; `ask-small` 25 on waiting cards. Put the object or target in italic. Use sentence case and no full stop.
- `title` 44 serif for screen titles (Activity, Grants, Autopilot, Settings), left-aligned under the status bar. Sheet titles use `sheet-title` 32 with one italic word: "Remember *for*", "Bypass *every AI?*".
- `stat` 40 serif for numbers that are the point (142 requests, 94%, 5h left).
- Geist: `button` 16.5 semibold, `row` 15.5 medium, `body` 15/22, `meta` 13.
- Mono: `label` 11.5 capitals tracked 0.16em (field label lines, section labels); `value` 13.5 for branches, accounts, addresses, SHAs, the timer.
- Bold 700 is not used. Serif is never bold.

## Layout

- The screen gutter is `space-24`. Waiting cards inset `space-16`. Section labels sit `space-26` above their list.
- **A field runs into the top of the screen**, behind the status bar and the notch, Dynamic Island or punch-hole camera. Only its bottom corners round (`radius-field`). Its contents start below the safe-area inset; never hardcode the status-bar height.
- A field holds: the top bar (close or back, the agent, the timer), the `label` line, the `ask`, and one verdict line (Autopilot's opinion, or why it always asks). Nothing else.
- Under the field: facts as hairline rows (key left in `ink-2`, value right). Long lists fold into a row with a chevron. Drill-ins keep a thin band of the field colour at the top, so you never lose what you're deciding.
- The decision bar sits at the bottom: an "Allow for" chooser (Once · 1 hour · 1 day · Custom), then Deny (outline) and the action (`inverse`, with the Face ID or fingerprint glyph).
- Root screens: serif title, header actions (round `paper-raised` buttons, the mode chip), section labels, hairline lists, and the floating tab bar (`inverse` capsule, the selected tab a `paper` pill). On Android the tab bar docks, Material-style.
- Sheets rise on `paper-raised` with `radius-sheet` over a `scrim`; the field stays visible behind.

## States

- **Waiting:** a waiting card or field with a running timer ring. Under 15 s the timer becomes a white pill with red digits.
- **Autopilot's pick** is the first button on a waiting card ("Allow · 97%", "Deny · 91%"); when unsure it shows a ring and "Unsure"; always-ask kinds show a lock and "Always asks".
- **Decided:** paper, a kind tile, the ask in serif on paper, the verdict in violet, and "This was wrong" as a soft button.
- **Irreversible:** the field is `field-delete`, the chooser locks to "Asked every time", and the action becomes a hold button that fills over `hold-duration` with haptic ticks that speed up. Letting go cancels.
- **Pressed:** content dims to 55%. **Disabled:** 40%. **Focus:** a 2px `violet` ring at a 3px offset on paper, a 2px `on-field` ring on fields.

## Motion

- A field slides down from the top over `field-in` (320 ms, ease out) when an approval opens; the ask fades in 60 ms later.
- When the kind changes (moving between waiting requests), the field colour cross-fades over 200 ms; the layout doesn't move.
- Hold buttons fill linearly over `hold-duration`. Sheets rise over 280 ms. Hover and colour changes take `fade`.
- Everything respects reduced motion: fields appear without sliding, holds still take their full time.

## Iconography

- Reins glyphs (asset group `Glyphs`): 24 grid, round caps and joins, 1.6 stroke, one weight. Sizes 14 (chevrons), 16–19 (buttons, verdicts), 20–22 (rows, tiles). On a field they're `on-field`; in rows `ink-2`; chevrons `ink-faint`.
- Signal adds seven glyphs on the same grid: `pulse` (Activity tab), `infinity` (no end), `undo` (This was wrong), `fingerprint` (Android approve), `face` (Face ID approve), `arrow-left` (Android back), `minus`.
- Service and agent marks (GitHub, Gmail, Claude, ChatGPT) appear only to name that service or agent: small, unmodified, in an avatar or on a white tile.
- No emoji, and no ✦ ⚠ ✕ characters.

## The mark

The Reins mark (`Logos/reins-mark.svg`) is a lavender shield and white check on `#170b33`. It sits on paper or dark paper, never on a field.
