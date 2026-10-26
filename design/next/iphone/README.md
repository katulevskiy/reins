# Reins · iPhone · Next (design boards, 2026-10-26)

A copy of the "iPhone · Next" page from the Reins design canvas, as published right now. 25 boards.

- `previews/` a PNG of every board, the quickest way to look.
- `project/` the board files (`*.dc.html`), `canvas.json` (positions, titles, pages) and `support.js` (the board runtime).
- `_blob/` every image the boards use (wallpapers, grain, illustrations, logos, screenshots), named by asset id.
- `source/` the scripts that generate the boards (Python, plus a few Node render and measure scripts).

To open a board in a browser, serve this folder from its root so `/_blob/...` resolves, for example
`python3 -m http.server 8000` here, then open `http://localhost:8000/project/<Board>.dc.html`.
Fonts (Sora, Geologica, JetBrains Mono, Instrument Sans and Serif, Geist) load from Google Fonts, so you need to be online.

**iPhone · Next:** Next · User flows, scored, Next · System (the kit), Next · Mode picker, final, Next · Running mode and the ask, immersive, Next · Type (Sora), Next · Cyrillic for Sora, four options, Next · Arrive, Next · Tour, Next · Activity, Next · Approve a push, Next · Every kind of ask, Next · Pair and join, Next · Trust and grants, Next · Permissions, Next · Autopilot, Next · Connections, Next · Vault, Next · Settings, Next · Outside the app, Next · More screens, Next · Activity with a wallpaper, Next · Prototype, core loop, Next · Stress test, Next · iPhone SE, chose B, Next · Largest text
