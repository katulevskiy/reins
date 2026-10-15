# Reins Signal: implementation plan (approved)

Read `HANDOFF.md` first; this file records the decisions taken on top of it and how the work is split.

## Decisions

- **Palettes:** Signal values go into the existing palettes (`ios/Shared/Design/Palette.swift`, `android/.../design/Palette.kt`, `crates/reins-desktop-app/src/theme.rs`). Phase 1 repoints the old names to the new values and keeps them as aliases, so every screen recolours with no layout change. Phase 2 renames callers screen by screen; the aliases are deleted last. Never add a parallel colour system.
- **One field type per platform:** `Field { push, read, delete, grant, pair, accounts, warn }` with the solid field colour (same in both themes), the kind ink (lighter in dark) and the 13% kind tile. It replaces today's overlapping kind/tone mappings (`ActionKind.tint`, `Tone`, `HeroTone`, desktop `Tone`). The logic that picks a kind stays; only its output type changes.
- **Reads and searches** are both `read` (blue). The old teal `read` goes away.
- **Overnight** (merged in #85; not drawn by the design): while it runs it uses the violet `grant` field, its chip is `violet-fill`, and its glyph is a `moon` drawn in the Signal style (below).
- **Glyphs:** the 69 in `assets/icons` are the one set for all three platforms. Glyphs the apps need that the set lacks are drawn in the same style (24 grid, 1.6 stroke, round caps and joins, `currentColor`, no fills except where the set already fills) into `assets/icons/` with an `index.json` entry, so every platform imports one folder. Map to the nearest existing glyph only where the meaning is the same.
- **Hold to confirm:** 1200 ms everywhere, release drains in 180 ms. The fill is driven by elapsed time so Reduce Motion / "Remove animations" never shortens it.
- **Grain:** soft-light on iOS and Android (minSdk 31). Desktop GPUI has no blend modes or image tiling: pre-bake one soft-light tile per field colour at startup and paint them as tiles. Off with Reduce Transparency (and the Appearance switch). Widgets, notifications and Live Activities use a pre-baked ~6% grain, Dynamic Island stays plain.
- **Autopilot's reason line** ("like 11 pushes you approved"): built on the phone from Autopilot's per-class counts, and for pushes refined with the repo and branch kind (default vs feature) counted from the decided entries' stored approvals (`audit_snapshots`, which keep the approval view including `GitPushView` refs). The current push names its branch from the request.
- **Desktop boards:** follow each board; keep both "wait for your phone" settings (guard and approval timeouts) as two rows; tray uses the board's pause lengths.
- **Logic stays:** data, networking, crypto, policy, approval and grant logic do not change. If the IA moves a view, move the view and its state; do not rewrite it.

## Phase 1: foundation (one PR, `signal/foundation`, no screen changes)

Each platform lands its part on `signal/foundation-<platform>` (branched from `signal/foundation`); the coordinator merges them into `signal/foundation` and opens one PR.

- Fonts (already added: `ios/Shared/Fonts`, `android/app/src/main/assets/fonts`; desktop embeds from `ios/Shared/Fonts`): register them, add the type scale from `tokens/signal.tokens.json` as named styles, the italic target helper for asks (3 lines before shrinking, never below 32 pt), map weight 700 to 600.
- Tokens into the existing palette with aliases (above); light and dark.
- `Field` type, grain, pond and kite illustrations (light/dark).
- Glyph set imported from `assets/icons` with a small script, plus a `Glyph` type; call sites keep working (screens switch to glyphs in phase 2, except where phase 1 must swap a removed icon).
- HANDOFF §8 components, built and previewed/tested but not yet used by screens: buttons (inverse, outline, soft, danger wash, on-field white, on-field wash; pressed 55%, disabled 40%, focus ring), hold button, Allow-for chooser (with the locked "Asked every time"), menu that grows from its trigger, mode chip with breathing dot, waiting card, hairline row, kind tile, pill, switch, timer pill, empty state (pond + mood line), field header (runs under the status bar, bottom radius 30, insets from the system), tab bar (phones).
- Motion helpers: the curves and durations from the tokens, a Reduce Motion reader (150 ms cross-fades instead), Reduce Transparency reader.
- A component gallery screen in the debug/demo build (iOS demo, Android debug, desktop `--demo`) showing every component in light and dark, used for the phase-1 screenshots.

## Phase 2+: screens, one PR per platform chunk

- **iOS** (`signal/ios-*`): 1 shell and IA (tabs Activity · Trust · Connections · Settings; Trust = Autopilot + Grants; Connections = Settings' connections + Integrations; deep links; UI-test tab IDs `tab:activity|trust|connections|settings`), 2 Activity (stack + river, menus, flick, warn field, fold), 3 Approval + every kind + pair/join, 4 Trust pages, 5 Connections/Vault/Settings (+Appearance)/Arrive/Tour 8→5, 6 Outside (widgets, Live Activities, notification content extension, Mode control) and alias removal.
- **Android** (`signal/android-*`): same chunks; docked tab bar; screenshots from the Robolectric `ScreenshotsTest` (light/dark); QS tiles and Glance widgets are new.
- **Desktop** (`signal/desktop-*`): 1 sidebar/Home/Waiting/Activity, 2 Connections/Keys/Rules/Settings(+Appearance)/pause/work session/health, 3 first run, tray, notification text, and the server's web pages (`src/api/reins/pages.rs`).
- Every screen: light and dark screenshot compared against its board; before/after in the PR; UI tests updated for renamed or moved screens; approximations listed in the PR.
