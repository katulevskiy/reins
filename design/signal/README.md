# Reins Signal: design package

The full Reins redesign for iPhone, Android and desktop: every screen, the design system, and all assets.

Start with **HANDOFF.md** (how to implement it), then look at **screens/png/Main.png** and **screens/png/Map.png**.

## Contents

- `HANDOFF.md`: implementation brief for a coding agent.
- `SIGNAL-RULES.md`: the base rules of the Signal style.
- `screens/png/`: a rendered image of every board (plus four states of the interactive prototype).
- `screens/notes/`: all text on every board, including the behaviour captions.
- `screens/source/`: board markup with exact CSS values, and `canvas.json`.
- `tokens/`: tokens as JSON, CSS, Swift, Kotlin and Rust.
- `fonts/`: Instrument Serif, Instrument Sans, Geist, Geist Mono (OFL).
- `assets/icons/`: 69 glyphs (open `preview.html`).
- `assets/logos/`: provider marks and the Reins mark.
- `assets/illustrations/`: pond and kite drawings, light and dark.
- `assets/textures/grain.png`: the field grain.

## Screens

### System

- **Cover: Signal en plein air**: `screens/png/Main.png`, notes `screens/notes/Main.md`, source `screens/source/Main.dc.html`
- **Colour**: `screens/png/Color.png`, notes `screens/notes/Color.md`, source `screens/source/Color.dc.html`
- **Type**: `screens/png/Type.png`, notes `screens/notes/Type.md`, source `screens/source/Type.dc.html`
- **Motion, haptics, sound**: `screens/png/Motion.png`, notes `screens/notes/Motion.md`, source `screens/source/Motion.dc.html`
- **Components**: `screens/png/Components.png`, notes `screens/notes/Components.md`, source `screens/source/Components.dc.html`
- **Map of every screen**: `screens/png/Map.png`, notes `screens/notes/Map.md`, source `screens/source/Map.dc.html`

### iPhone

- **iPhone · Arrive**: `screens/png/iOS-Arrival.png`, notes `screens/notes/iOS-Arrival.md`, source `screens/source/iOS-Arrival.dc.html`
- **iPhone · Tour**: `screens/png/iOS-Tour.png`, notes `screens/notes/iOS-Tour.md`, source `screens/source/iOS-Tour.dc.html`
- **iPhone · Activity**: `screens/png/iOS-Activity.png`, notes `screens/notes/iOS-Activity.md`, source `screens/source/iOS-Activity.dc.html`
- **iPhone · Approve a push**: `screens/png/iOS-Approve.png`, notes `screens/notes/iOS-Approve.md`, source `screens/source/iOS-Approve.dc.html`
- **iPhone · Every kind of ask**: `screens/png/iOS-Kinds.png`, notes `screens/notes/iOS-Kinds.md`, source `screens/source/iOS-Kinds.dc.html`
- **iPhone · Pair and join**: `screens/png/iOS-Connect.png`, notes `screens/notes/iOS-Connect.md`, source `screens/source/iOS-Connect.dc.html`
- **iPhone · Trust and grants**: `screens/png/iOS-Trust.png`, notes `screens/notes/iOS-Trust.md`, source `screens/source/iOS-Trust.dc.html`
- **iPhone · Autopilot**: `screens/png/iOS-Autopilot.png`, notes `screens/notes/iOS-Autopilot.md`, source `screens/source/iOS-Autopilot.dc.html`
- **iPhone · Connections**: `screens/png/iOS-Connections.png`, notes `screens/notes/iOS-Connections.md`, source `screens/source/iOS-Connections.dc.html`
- **iPhone · Vault**: `screens/png/iOS-Vault.png`, notes `screens/notes/iOS-Vault.md`, source `screens/source/iOS-Vault.dc.html`
- **iPhone · Settings**: `screens/png/iOS-Settings.png`, notes `screens/notes/iOS-Settings.md`, source `screens/source/iOS-Settings.dc.html`
- **iPhone · Outside the app**: `screens/png/iOS-Outside.png`, notes `screens/notes/iOS-Outside.md`, source `screens/source/iOS-Outside.dc.html`

### Android

- **Android · Core screens**: `screens/png/And-Core.png`, notes `screens/notes/And-Core.md`, source `screens/source/And-Core.dc.html`
- **Android · Payments**: `screens/png/And-Pay.png`, notes `screens/notes/And-Pay.md`, source `screens/source/And-Pay.dc.html`
- **Android · Outside the app**: `screens/png/And-Outside.png`, notes `screens/notes/And-Outside.md`, source `screens/source/And-Outside.dc.html`

### Desktop

- **Desktop · Home**: `screens/png/Desk-Home.png`, notes `screens/notes/Desk-Home.md`, source `screens/source/Desk-Home.dc.html`
- **Desktop · Home, waiting**: `screens/png/Desk-Waiting.png`, notes `screens/notes/Desk-Waiting.md`, source `screens/source/Desk-Waiting.dc.html`
- **Desktop · Activity**: `screens/png/Desk-Activity.png`, notes `screens/notes/Desk-Activity.md`, source `screens/source/Desk-Activity.dc.html`
- **Desktop · Connections**: `screens/png/Desk-Connections.png`, notes `screens/notes/Desk-Connections.md`, source `screens/source/Desk-Connections.dc.html`
- **Desktop · Keys**: `screens/png/Desk-Keys.png`, notes `screens/notes/Desk-Keys.md`, source `screens/source/Desk-Keys.dc.html`
- **Desktop · Rules**: `screens/png/Desk-Rules.png`, notes `screens/notes/Desk-Rules.md`, source `screens/source/Desk-Rules.dc.html`
- **Desktop · Settings**: `screens/png/Desk-Settings.png`, notes `screens/notes/Desk-Settings.md`, source `screens/source/Desk-Settings.dc.html`
- **Desktop · First run**: `screens/png/Desk-Setup.png`, notes `screens/notes/Desk-Setup.md`, source `screens/source/Desk-Setup.dc.html`
- **Desktop · Menus, sessions, web**: `screens/png/Desk-Extras.png`, notes `screens/notes/Desk-Extras.md`, source `screens/source/Desk-Extras.dc.html`

### Prototype

- **Prototype · Ask to answer**: `screens/png/Proto-Approve.png`, notes `screens/notes/Proto-Approve.md`, source `screens/source/Proto-Approve.dc.html`
- Prototype states: `screens/png/Proto-Approve-2-review-push.png`, `-3-after-deny-push`, `-4-review-delete`, `-5-all-quiet`
