# Reins Next: implementation plan

## Branches

- `next/v1` is the integration branch (this package is its first commit). Every platform PR targets `next/v1`,
  never `main`. The coordinator keeps `next/v1` merged up with `main`.
- One branch per platform chunk: `next/ios-*`, `next/android-*`, `next/desktop-*`.
- When all three platforms are done, `next/v1` merges to `main` with a `feat:` title (minor version bump) and the
  release workflow ships every platform at once.

## Rules

- **The boards are the spec.** Compare every screen, light and dark, against its board in `previews/`; read exact
  values from `project/*.dc.html`. Where a board and the platform's conventions disagree, follow the board unless it
  breaks accessibility or a platform rule (then note it in the PR).
- **Replace Signal, don't add a second system.** Put the Next values into the existing palette, type and component
  code (`ios/Shared/Design`, `android/.../design`, `crates/reins-desktop-app/src/theme` and `ui/kit`). Delete
  Signal-only pieces once nothing uses them.
- **Fonts:** Sora 400/500/600 for all text, Geologica 500 for Cyrillic under Sora (Latin stays Sora), JetBrains Mono
  400/500 for machine values (branches, hashes, addresses, codes, diffs). OFL files from github.com/google/fonts;
  ship the licence beside them. Type scale from the Type board (title 30, ask 33, card ask 22, row 16, meta 12.5,
  section 13 caps-free with tracking, code 11.5, codes 24 mono).
- **Wallpapers and panes:** the dithered wallpapers from the boards (Halo default in dark, Torii in light; others
  selectable as on the Wallpaper/Settings boards) sit behind frosted panes. Reduce Transparency (iOS), the
  "remove transparency"/high-contrast equivalent on Android, and the macOS/desktop setting make panes solid; the
  art stays.
- **Logic stays.** Data, networking, crypto, policy, approval and grant logic do not change for the look. If a board
  draws behaviour the app doesn't have yet (a new grant length, a new filter, swipe actions, wallpaper choice),
  build it in the UI layer when the existing core/API already supports it. If it needs a new core or server API,
  don't invent one in the platform PR: list it in the PR under "Needs core" and tell the coordinator, who
  schedules it once for all platforms.
- **Icons:** SF Symbols on iOS, Material Symbols on Android, the existing desktop glyph set; brand marks as logos. No
  emoji or unicode glyphs as icons.
- **Text:** as on the boards: short. Help goes behind a (?) button.
- **Accessibility:** Dynamic Type / font scale up to the Largest text board (iPhone AX board), Reduce Motion
  (150 ms cross-fades), Reduce Transparency, VoiceOver/TalkBack labels, 44/48 pt targets.
- **Every PR:** before/after screenshots (light and dark) beside the matching board; UI tests updated for anything
  renamed or moved; a list of approximations and anything not built.
