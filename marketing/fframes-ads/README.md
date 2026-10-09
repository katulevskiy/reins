# Reins ads

Short ads for Reins, written in Rust and SVG with [fframes](https://github.com/dmtrKovalenko/fframes) (MIT) and
rendered on the GPU (Skia on Vulkan). Every shape, word, timing and sound is code in this folder: no screen
recordings, no stock footage, no AI imagery. The phone is a coded recreation of the app's approval sheet, Activity and
Keys screens, in the Reins Dark palette and Geist (linked from the Android app's assets).

## Films

| Command name | Output file | Size | Length | What it is |
| --- | --- | --- | --- | --- |
| `hero-wide` | `reins-hero-16x9.mp4` | 1920×1080 | 30 s | Hero ad. An agent force-pushes, deletes and emails everyone; the tape rewinds, Reins appears, and the same push now waits for the phone: one tap stops it, one tap approves the next. Keys stay on the phone. Tagline and reins2fa.com. |
| `hero-tall` | `reins-hero-9x16.mp4` | 1080×1920 | 30 s | The same, laid out for phones. |
| `hook-rmrf` | `reins-hook-rmrf-9x16.mp4` | 1080×1920 | 9 s | "Your AI just tried to rm -rf /": blocked, the phone shows the command, one tap denies it. |
| `hook-push` | `reins-hook-push-9x16.mp4` | 1080×1920 | 11 s | "Approve git push from your phone": the commits on the phone, one tap, pushed. |
| `hook-tokens` | `reins-hook-tokens-9x16.mp4` | 1080×1920 | 12.6 s | "Your agent never sees your tokens": the agent looks for a token, finds nothing, asks the phone; the phone merges with its own key. |
| `pay-wide` | `reins-pay-16x9.mp4` | 1920×1080 | 20 s | Reins Pay: the agent shops, the phone shows the exact cart, total, address and card, Face ID approves, the agent gets a one-time card for that cart only. When the cart changes, the card is declined and the phone asks again. |
| `pay-tall` | `reins-pay-9x16.mp4` | 1080×1920 | 20 s | The same, for phones. |

All films are 60 fps H.264 with AAC audio, mixed to about −14 LUFS with true peaks under −1 dBTP (what X, LinkedIn,
Instagram, TikTok and YouTube normalise to). Text is burned in and follows the voiceover word by word, so the films
work muted.

## Render

You need Rust, ffmpeg's build dependencies (see the fframes README), [Bun](https://bun.sh) for the sound, and a
Vulkan GPU.

```sh
bun sound/build.ts      # music and sound effects into assets/ (about 10 s)
./render.sh             # every film into ../../../reins-ads-out/ (OUT=dir to change), with loudness numbers
./render.sh pay-wide    # just one
```

The first build downloads prebuilt Skia and FFmpeg. If linking fails with `undefined symbol: x264_encoder_open_163`,
your system x264 is newer than the one the prebuilt FFmpeg was built against: build FFmpeg from source instead
(needs `nasm`; takes about 5 minutes once):

```sh
FFMPEG_FORCE_BUILD=1 cargo build --release
```

and keep `FFMPEG_FORCE_BUILD=1` set for later commands.

## Work on a film

Every command of the fframes CLI works per film (`--film` defaults to `hero-wide`):

```sh
R() { cargo run --release -- "$@"; }
R --film hero-tall timeline                  # beats (scenes) and every sound with its time and gain
R --film hero-tall strip all -n 16           # contact sheet, writes strip.png
R --film hero-wide frame Keys@1s,Cta@end     # full-size PNGs into frames/
R --film hook-rmrf onion "Phone@0..Phone@1s" -n 6   # motion trail, writes onion.png
R --film pay-wide inspect                    # cut-off text, missing fonts or glyphs, broken SVG
R --film pay-wide audio analyze --waveform w.png    # loudness, peaks, silence
R --film hero-wide preview 8s                # real-time window with sound
```

Beats are named scenes (`Threat`, `Rewind`, `Logo`, `Stop`, `Approve`, `Keys`, `Tagline`, `Cta` in the hero), so
`strip Keys` or `frame Stop@2s` address them directly. The picture is drawn from one continuous clock, so the
phone and the terminal stay alive across beats.

## Where things are

| Path | What |
| --- | --- |
| `src/films/hero.rs` | The hero: beats, every time (most on a word of the voiceover), captions, sound cues, layouts per format. |
| `src/films/hooks.rs` | The three hooks and the vertical stage they share (terminal that moves aside for the phone). |
| `src/films/pay.rs` | Reins Pay, kept separate so it can follow the feature. `STATUS` is the line on the end card ("Reins Pay. Coming soon."; change it when it ships), `ITEMS` the cart. |
| `src/ui/phone.rs` | The phone: island banners, approval sheet per kind of request (force push, push, `rm -rf`, email, merge, payment), Face ID, taps, Activity and Keys. |
| `src/ui/terminal.rs` | The agent's terminal: typed commands, waiting spinner, results. |
| `src/ui/mod.rs` | Backdrop, kinetic words, the Reins mark and lockup, pills, end card. |
| `src/kit.rs` | Formats, palette, springs and eases, text measuring, icons. |
| `sound/build.ts` | The scores (one per film, at 120 BPM on the films' beats) and the sound effects, synthesised from scratch with `sound/dsp.ts` and `sound/instruments.ts` (shared with the launch film). |
| `media/` | Geist (symlinks to the Android app's fonts) and the voiceover lines (`voNN.mp3`). |

Moving a beat: change its constant in `src/films/*.rs`; captions, phone, terminal and sound cues follow. If it moves a
musical section, change the matching line in `sound/build.ts` and re-run it.

## Voiceover

The voice is the ElevenLabs "Sarah" take recorded for the launch film (`marketing/launch-video`), cut into lines; the
lines used here are small mono copies in `media/`. The words each caption lands on come from that take's
alignment. Reins Pay reuses existing lines ("You see exactly what will happen.", "Agents get the result. Never the
key."); for Pay-specific lines, record a new take with the launch film's `scripts/voiceover.ts`, save the clips as
`media/voNN.mp3` and point `pay::audio` at them.
