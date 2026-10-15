# Reins Signal redesign: implementation handoff

This package is the complete redesign of the Reins apps (iPhone, Android, desktop) in the Reins Signal style. It is meant to be dropped into the `katulevskiy/reins` repo at `design/signal/` and implemented by a coding agent.

## 1. What is the source of truth

Read in this order. When two disagree, the earlier one wins.

1. `screens/png/*.png`: how every screen looks. Phone boards show light and dark side by side (alternating).
2. `screens/notes/*.md`: the text of every board, including the caption under each phone. The captions are the behaviour spec (what a tap, swipe, hold or menu does, which haptic fires, what the empty state says).
3. `screens/source/*.dc.html`: the board markup. Open these for exact values (padding, radius, font size, line height, colours, glyph paths). They are HTML/CSS mockups, not production code. Ignore `{{holes}}`, `<sc-if>`, `<sc-for>`, `<x-dc>` and the `DCLogic` script; they are canvas templating.
4. `tokens/signal.tokens.json`: every token (colour light/dark, type scale, spacing, radius, motion, texture, illustrations).
5. `SIGNAL-RULES.md`: the base rules of the style. The redesign extends them. Where it says Geist is the sans, the redesign uses **Instrument Sans** instead (Geist stays as the fallback). Everything else in it still holds.

`screens/source/canvas.json` lists every board with its title and platform page.

## 2. The one rule

A solid field of colour appears **only** when something needs a decision. Everything else is paper, ink and hairlines.

| Field | Means | Hex (same in both themes) |
| --- | --- | --- |
| push | writes or sends | #C2410C |
| read | reads or searches | #2563EB |
| delete | can't be undone, pay, bypass | #B91C1C |
| grant | power already handed over | #5B43E8 |
| pair | connects something new | #0E7490 |
| accounts | changes an account | #BE185D |
| warn | Reins is broken and needs you | #A16207 |

Fields stay, paper flips: only `paper`, `ink` and neighbours change between light and dark. Every field carries the grain texture. Nothing else does.

## 3. Type

- **Instrument Serif** (400, regular + italic): asks, titles, big numbers, the mood line. Never bold. The target of an ask is italic: "Push 8 commits to *octo/app*".
- **Instrument Sans** (400/500/600): everything you read. Geist is the drop-in alternative.
- **Geist Mono** (400/500): labels in capitals tracked 0.16em, and machine values (branches, SHAs, timers, amounts).
- No weight 700 anywhere. Full scale in `tokens/signal.tokens.json` and `screens/png/Type.png`.
- Dynamic Type / Android font scale: asks reflow to three lines before they shrink, never below 32 pt.

## 4. Information architecture (new)

- **Phone tabs:** Activity, Trust (Autopilot + Grants merged), Connections, Settings. The tab bar is a floating `inverse` capsule with a `paper` pill on the selected tab (iOS). Android docks it, same pill.
- **Desktop sidebar:** Home, Activity, Connections, Keys, Rules, Settings.
- Phone and desktop mirror each other: Activity = Activity, Trust = Rules, Connections = Connections, Settings = Settings.
- `screens/png/Map.png` shows every screen, what is NEW, MOVED or MERGED, and which are sheets.

## 5. Assets in this package

| Folder | What | How to use |
| --- | --- | --- |
| `fonts/` | TTFs for Instrument Serif, Instrument Sans, Geist, Geist Mono (OFL, see `OFL.txt`) | iOS: add to target + `UIAppFonts`. Android: `res/font` or the existing `assets/fonts`. Desktop: register in `theme.rs` like Geist is today. |
| `assets/icons/` | 96 glyphs, 24 grid, 1.6 stroke, round caps, `currentColor`. `preview.html` shows them all, `index.json` lists names | iOS: asset catalog SVG, Preserve Vector Data, render as template. Android: convert to VectorDrawable (Android Studio Vector Asset, or `npx svg2vectordrawable`). Desktop: put in `crates/reins-desktop-app/assets/icons`. Replace the old icon set with these so all three platforms share one set. |
| `assets/logos/` | Brand marks for providers (Claude, OpenAI, Cursor, Gemini, GitHub, GitLab, Codeberg, Bitbucket, Gmail, Google Calendar, Contacts, Telegram, MCP, SMS, Vault, Payments) and the Reins mark | Shown inside a 28 px white circle avatar at 62%. Keep the existing provider assets if they already match. |
| `assets/illustrations/` | `pond` (empty / all quiet) and `kite` (arrival), each as `-light` and `-dark` SVG | iOS: one image set with Any / Dark appearances. Android: `drawable` + `drawable-night`. Desktop: switch on theme. |
| `assets/textures/grain.png` | Tileable 360 px grey noise | Tile at 180 pt over every colour field, soft-light blend. iOS: `Image("grain").resizable(resizingMode: .tile).blendMode(.softLight)`. Android: `ImageShader(..., TileMode.Repeated)` drawn with `BlendMode.Softlight`. Desktop: if GPUI has no soft-light, draw at about 6% opacity normal blend. Off with Reduce Transparency. Never animated. |
| `tokens/` | `signal.tokens.json`, `signal.css`, `ios/SignalTokens.swift`, `android/SignalTokens.kt`, `desktop/signal_tokens.rs` | Generated colour constants. Replace the values in the existing palettes rather than adding a parallel system. |

## 6. Where things live in the repo today

- **iOS (SwiftUI):** `ios/Shared/Design/Palette.swift`, `ios/Shared/Design/Typography.swift`, components in `ios/Reins/Design/` (Cards, Components, Countdown, HoldButton, GrantTime, Avatars), screens in `ios/Reins/Screens/*`, widgets in `ios/ReinsWidgets`, notifications in `ios/ReinsNotifications`.
- **Android (Compose):** `android/app/src/main/java/dev/reins/android/design/` (Palette.kt, Type.kt, Fonts.kt, Theme.kt, Icons.kt, Cards.kt, Components.kt, HoldButton.kt, Tags.kt), screens in `.../android/ui/*`, fonts in `android/app/src/main/assets/fonts`, sounds in `res/raw`.
- **Desktop (Rust, GPUI via the zui fork):** `crates/reins-desktop-app/src/theme.rs`, views in `src/ui`, tray in `tray.rs`, first run in `welcome.rs`, icons in `assets/icons`.

Verify these paths before editing; they were correct when this package was made.

## 7. Motion, haptics, sound

From `screens/notes/Motion.md` (full detail there):

| Moment | Motion | Haptic | Sound (existing fx_*) |
| --- | --- | --- | --- |
| Request arrives | Field slides in 320 ms, ease (0.2, 0.8, 0.2, 1); ask follows 60 ms later | attention: two ticks 90 ms apart | chime_request |
| Approve | Field folds into its kind tile and lands at top of the list, 360 ms | confirm: one click | send |
| Deny | Same fold | click, low tick, falling buzz | close |
| Flick a waiting card | Follows thumb, tilts up to 6°, past 35% flies off in 220 ms; next card springs (0.32, 0.86) | confirm or deny | |
| Hold (irreversible) | 1200 ms linear fill; release drains in 180 ms | ticks 260 ms apart closing to 70 ms, heavy thud at the end; none on release | |
| Grant created | | quick rise, low tick | chime_done |
| Bypass on | | surge: ramp, crack, thud | surge |
| Tabs | Pill slides 240 ms spring with slight overshoot | select | |
| Sheet | Rises 280 ms over 32% scrim, field stays visible above | | |
| Menu | Grows from trigger, scale 0.92 to 1, 180 ms; closes 120 ms | select per row crossed | |
| Numbers | Roll digit by digit, 240 ms | | |
| Timer | Ring drains each second; under 15 s becomes a white pill with red digits | | |
| Slider detents | | tick per step | detent |

Reduce Motion: folds and slides become 150 ms cross-fades. The hold duration is never shortened. Everything obeys the Sounds and haptics switch.

## 8. Components (see `Components.png`)

Buttons (inverse primary, outline Deny, soft, danger wash, on-field white and wash), the hold button, the "Allow for" chooser (segmented, locks to "Asked every time" for irreversible kinds), menus that grow from their trigger (replace system pickers everywhere), the mode chip in every header (Manual, Auto, Lockdown, Bypass with a breathing dot), waiting cards with Autopilot's pick as the first button, hairline rows (never cards), kind tiles (field colour at 13% under its own ink; lighter inks in dark), pills that always carry a word, switches in `violet-fill`. Pressed dims to 55%, disabled 40%, focus is a 2 px violet ring at 3 px offset (white ring on fields).

## 9. Suggested plan

1. **Foundation, all platforms:** fonts, tokens into the existing palettes, the glyph set, grain, illustrations. Build the shared components from section 8. No screen changes yet.
2. **iOS screens** board by board in the order of the Map: Arrival, Tour, Activity (stack + river + fold), Approve, every kind of ask, Pair and join, Trust, Autopilot, Connections, Vault, Settings, Outside the app (notifications, Live Activities, widgets, controls).
3. **Android** (`And-Core`, `And-Pay`, `And-Outside`; anything not drawn for Android follows the iPhone board with Material conventions: docked tab bar, back gesture, heads-up notifications, Quick Settings tile).
4. **Desktop** (`Desk-*`).

Rules for the work:
- Restyle and restructure the UI only. Keep data, networking, crypto, policy and approval logic as they are. If the new IA needs a screen merged (Autopilot + Grants into Trust), move views; do not rewrite their logic.
- After each screen, take a simulator / emulator / app screenshot in light and dark and compare against the PNG. Fix differences before moving on.
- Respect safe areas: fields run behind the status bar and notch and only their bottom corners round (30). Never hardcode the status bar height.
- Accessibility: keep contrast at or above what the tokens list, support Dynamic Type and font scale, label every icon button, honour Reduce Motion and Reduce Transparency.
- Work on a branch per platform and open a PR for each, with before/after screenshots.

## 10. Done means

- Every board in `screens/png` has a matching screen in the app, in light and dark.
- Colour fields appear only on decision surfaces; grain only on fields.
- One glyph set and one type system across all three platforms.
- Haptics and sounds fire as in section 7, and turn off with the switch.
- Existing tests pass; UI tests updated for renamed screens.
