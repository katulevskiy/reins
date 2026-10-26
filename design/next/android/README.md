# Reins · Android · Next (design boards, 2026-10-26)

A copy of the "Android · Next" page from the Reins design canvas, as published right now. 14 boards.

- `previews/` a PNG of every board, the quickest way to look.
- `project/` the board files (`*.dc.html`), `canvas.json` (positions, titles, pages) and `support.js` (the board runtime).
- `_blob/` every image the boards use (wallpapers, grain, illustrations, logos, screenshots), named by asset id.
- `source/` the scripts that generate the boards (Python, plus a few Node render and measure scripts).

To open a board in a browser, serve this folder from its root so `/_blob/...` resolves, for example
`python3 -m http.server 8000` here, then open `http://localhost:8000/project/<Board>.dc.html`.
Fonts (Sora, Geologica, JetBrains Mono, Instrument Sans and Serif, Geist) load from Google Fonts, so you need to be online.

**Android · Next:** Android Next · System (the kit), Android Next · Arrive, Android Next · Activity, Android Next · Activity with a wallpaper, Android Next · Approve a push, Android Next · Every kind of ask, Android Next · Pair and join, Android Next · Permissions, Android Next · Trust, modes and grants, Android Next · Autopilot, Android Next · Connections, Android Next · Vault, Android Next · Settings, Android Next · Outside the app
