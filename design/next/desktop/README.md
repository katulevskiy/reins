# Reins · Desktop · Next (design boards, 2026-10-26)

A copy of the "Desktop · Next" page from the Reins design canvas, as published right now. 12 boards.

- `previews/` a PNG of every board, the quickest way to look.
- `project/` the board files (`*.dc.html`), `canvas.json` (positions, titles, pages) and `support.js` (the board runtime).
- `_blob/` every image the boards use (wallpapers, grain, illustrations, logos, screenshots), named by asset id.
- `source/` the scripts that generate the boards (Python, plus a few Node render and measure scripts).

To open a board in a browser, serve this folder from its root so `/_blob/...` resolves, for example
`python3 -m http.server 8000` here, then open `http://localhost:8000/project/<Board>.dc.html`.
Fonts (Sora, Geologica, JetBrains Mono, Instrument Sans and Serif, Geist) load from Google Fonts, so you need to be online.

**Desktop · Next:** Desktop Next · System (the kit), Desktop Next · First run, Desktop Next · Home and Activity, Desktop Next · The ask, Desktop Next · Pushes and diffs, Desktop Next · Connections, Desktop Next · Keys and Vault, Desktop Next · Secrets scan, Desktop Next · Autopilot, Try it, Desktop Next · Rules and modes, Desktop Next · Settings, Desktop Next · Menu bar and notifications
