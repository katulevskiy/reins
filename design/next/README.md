# Reins Next: design package (2026-10-26)

The Next redesign for iPhone, Android and desktop, exported from the Reins design canvas. It follows Signal
(`design/signal/`) in structure and behaviour and replaces its look: Sora (Geologica for Cyrillic), JetBrains Mono
for machine values, dithered wallpapers behind frosted panes, colour fields for the ask and the running modes.

The boards are the spec. Start with each platform's System board, then its screens.

```
iphone/    25 boards: Next-*.png      (System, Type, Cyrillic, Flows, ModePick, Running, Arrival, Tour, Activity,
                                        Approve, Kinds, Connect, Trust, Permissions, Autopilot, Connections, Vault,
                                        Settings, Outside, More, Wallpaper, Proto, Stress, SE, AX)
android/   14 boards: AndNext-*.png   (System, Arrival, Activity, Wallpaper, Approve, Kinds, Connect, Permissions,
                                        Trust, Autopilot, Connections, Vault, Settings, Outside)
desktop/   12 boards: DeskNext-*.png  (System, Setup, Home, Ask, Pushes, Connections, Keys, Secrets, Autopilot,
                                        Rules, Settings, MenuBar)
```

In each platform folder:

- `previews/`: a PNG of every board. This is what to compare screenshots against.
- `project/`: the board markup (`*.dc.html`) with the exact CSS values (colours, sizes, radii, spacing), plus
  `canvas.json` and `support.js`. To open a board, serve the platform folder (`python3 -m http.server 8000`) and open
  `http://localhost:8000/project/<Board>.dc.html` (fonts come from Google Fonts, so you need to be online).
- `_blob/`: every image the boards use (wallpapers, grain, logos), named by asset id; the markup says which is which.

See `PLAN.md` for how the work is split and the rules for implementing it.
