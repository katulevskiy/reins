// Renders every sound the ads use into ../assets/ (gitignored): one music bed per film, scored to
// that film's beats, and one WAV per sound effect. Run with `bun sound/build.ts` (about 10 s).
//
// The music runs at 120 BPM (a beat is 0.5 s, a bar 2 s), and every film puts its cuts on that grid.
// The section times below mirror the timeline constants in src/films/*.rs: move a beat there, move
// it here too, then re-run this script.

import { mkdirSync } from 'node:fs';
import { SR, SVF, Saw, Stereo, TAU, buf, clamp, expInterp, fadeEdges, midi, noise, normalize, pingPong, reseed, reverb, softClip, writeWav } from './dsp';
import { SFX, bass, bell, clap, crash, drone, hat, kick, mono, pad, pluck, riser, subDrop, sweepNoise, stereoSweep, withVerb } from './instruments';

const OUT = new URL('../assets/', import.meta.url).pathname;
mkdirSync(OUT, { recursive: true });

const BEAT = 0.5;

// ---------------------------------------------------------------- a score, section by section

/** Dm9, Bbmaj7, Fadd9, Cadd9: the warm loop that plays once Reins is in charge. */
const WARM: [number, number[]][] = [
  [38, [53, 57, 60, 64]],
  [34, [50, 53, 57, 60]],
  [41, [53, 57, 60, 67]],
  [36, [52, 55, 60, 62]],
];

class Score {
  drums: Stereo;
  synth: Stereo; // ducked by the kick
  send: Stereo; // reverb
  echo: Stereo; // ping-pong delay
  kicks: number[] = [];
  mutes: [number, number][] = [];

  constructor(public length: number) {
    this.drums = new Stereo(length + 1);
    this.synth = new Stereo(length + 1);
    this.send = new Stereo(length + 1);
    this.echo = new Stereo(length + 1);
  }

  kick(t: number, level = 1) {
    this.drums.add(kick(level), t, 0.75);
    this.kicks.push(t);
  }

  /** Dark drone, D minor pulse getting brighter, kicks and hats piling up into `to`. */
  tension(from: number, to: number, o: { kicksFrom?: number; hatsFrom?: number; heartbeat?: boolean } = {}) {
    const len = to - from;
    this.synth.add(drone(38, len + 0.3), from, 0.26);
    const chords: [number, number[]][] = [
      [38, [50, 53, 57]],
      [34, [46, 50, 53]],
      [31, [43, 46, 50, 55]],
      [33, [45, 49, 52]],
    ];
    let bar = 0;
    for (let t = from; t < to - 0.01; t += 2, bar++) {
      const [root, chord] = chords[bar % chords.length];
      const d = Math.min(2, to - t);
      const p = (t - from) / len;
      this.synth.mix(pad(chord, d, { cut: 450 + p * 900, attack: 0.15, release: 0.3 }), t, 0.6);
      for (let k = t; k < t + d - 0.01; k += BEAT / 2) {
        const q = (k - from) / len;
        this.synth.add(bass(root, BEAT / 2 - 0.03, 350 + q * 1500, 0.75), k, 0.5);
      }
    }
    if (o.heartbeat)
      for (let t = from; t < (o.kicksFrom ?? to) - 0.01; t += 1) {
        this.drums.add(kick(0.7, 0.8), t, 0.5);
        this.drums.add(kick(0.45, 0.8), t + 0.24, 0.42);
      }
    for (let t = o.kicksFrom ?? to; t < to - 0.2; t += BEAT) this.kick(t);
    for (let t = o.hatsFrom ?? to; t < to; t += BEAT / 4) {
      const p = (t - (o.hatsFrom ?? to)) / Math.max(0.01, to - (o.hatsFrom ?? to));
      this.drums.add(hat(false, 0.25 + p * 0.5), t, 0.5, Math.sin(t * 7) * 0.4);
    }
    const riseFrom = Math.max(from, to - 2.5);
    this.drums.add(riser(to - riseFrom), riseFrom, 0.3);
  }

  /** A short bed of suspense: drone and a held pad, no drums. For the moment a request waits. */
  suspense(from: number, to: number, chord = [50, 53, 57, 62]) {
    this.synth.add(drone(38, to - from + 0.5), from, 0.18);
    const p = pad(chord, to - from, { cut: 600, cutEnd: 1400, attack: 0.4, release: 0.6 });
    this.synth.mix(p, from, 0.55);
    this.send.mix(p, from, 0.3);
    for (let t = from; t < to - 0.01; t += BEAT * 2) this.synth.add(pluck(74, 0.25), t, 0.5, 0.3);
  }

  /** Everything is silent between `from` and `to`. */
  mute(from: number, to: number) {
    this.mutes.push([from, to]);
  }

  /** Warm F major swell, sub drop and a run of bells: the logo. */
  reveal(at: number, until: number) {
    const r = new Stereo(until - at + 3);
    r.mix(pad([53, 57, 60, 67], until - at + 0.4, { cut: 900, cutEnd: 3200, attack: 0.3, release: 1.2 }), 0, 0.9);
    r.add(subDrop(midi(29) * 1.5, midi(29), until - at, 0.7), 0, 0.28);
    const sparkle = [77, 81, 84, 89, 91, 93, 96];
    for (let i = 0; i < 10; i++) r.add(bell(sparkle[(i * 3) % sparkle.length], 1.2, 0.18), 0.12 + i * 0.12, 0.5, ((i % 4) / 3) * 1.4 - 0.7);
    this.synth.mix(r, at);
    this.send.mix(r, at, 0.8);
  }

  /** The groove: pads, bass, four on the floor, claps, hats, and a 16th arpeggio after `arpFrom`. */
  groove(from: number, to: number, o: { arpFrom?: number; claps?: boolean; lift?: number; light?: boolean } = {}) {
    const { arpFrom = from, claps = true, lift = 0, light = false } = o;
    for (let bar = 0, t = from; t < to - 0.01; bar++, t += BEAT * 4) {
      const [root, chord] = WARM[bar % WARM.length];
      const d = Math.min(BEAT * 4, to - t);
      const p = pad(chord.map((n) => n + lift), d, { cut: 1300, cutEnd: 1800, attack: 0.08, release: 0.5 });
      this.synth.mix(p, t, light ? 0.45 : 0.6);
      this.send.mix(p, t, 0.25);
      for (let k = 0; k < 8 && t + k * (BEAT / 2) < to - 0.01; k++) {
        const n = k % 4 === 3 ? root + 12 : root;
        this.synth.add(bass(n, BEAT / 2 - 0.04, 650, 0.9), t + k * (BEAT / 2), light ? 0.45 : 0.6);
      }
      const tones = [...chord, chord[1] + 12, chord[2] + 12];
      const shape = [0, 2, 4, 5, 3, 1, 2, 4];
      for (let k = 0; k < 16; k++) {
        const at = t + k * (BEAT / 4);
        if (at >= to - 0.01 || at < arpFrom - 0.01) continue;
        const note = tones[shape[k % shape.length] % tones.length] + 12 + lift;
        const pl = pluck(note, k % 4 === 0 ? 0.55 : 0.38);
        this.synth.add(pl, at, 0.8, Math.sin(k * 1.3) * 0.5);
        this.echo.add(pl, at, 0.35);
      }
    }
    for (let t = from; t < to - 0.01; t += BEAT) {
      if (!light || Math.round((t - from) / BEAT) % 2 === 0) this.kick(t, light ? 0.8 : 0.95);
      const beat = Math.round((t - from) / BEAT);
      if (claps && beat % 2 === 1) {
        const c = clap(0.8);
        this.drums.add(c, t, 0.5);
        this.send.add(c, t, 0.4);
      }
      this.drums.add(hat(false, 0.5), t + BEAT / 2, 0.6, 0.25);
    }
    this.drums.add(crash(), from, 0.32);
  }

  /** Pads only, opening up, with a riser into `to`: a breath before the last hit. */
  breakdown(from: number, to: number) {
    const len = to - from;
    const p = pad([53, 57, 60, 64, 69], len, { cut: 700, cutEnd: 4200, attack: 0.6, release: 0.4 });
    this.synth.mix(p, from, 0.7);
    this.send.mix(p, from, 0.5);
    this.synth.add(bass(41, len - 0.1, 400, 0.6), from, 0.45);
    for (let i = 0; i * BEAT < len - 0.01; i++) {
      const pl = pluck([77, 81, 84, 88][i % 4], 0.3);
      this.synth.add(pl, from + i * BEAT, 0.6, Math.sin(i) * 0.6);
      this.echo.add(pl, from + i * BEAT, 0.4);
    }
    this.drums.add(riser(Math.min(len, 3)), to - Math.min(len, 3), 0.32);
    for (let t = to - 1; t < to - 0.01; t += BEAT / 4) this.drums.add(clap(0.3 + (t - (to - 1)) * 0.6), t, 0.45);
  }

  /** One big Fmaj9 with bells, ringing out to the end. */
  outro(at: number) {
    this.kick(at, 1);
    this.drums.add(crash(), at, 0.42);
    const fin = pad([41, 48, 53, 57, 60, 64, 67], 2.5, { cut: 2600, cutEnd: 700, attack: 0.02, release: 3.2 });
    this.synth.mix(fin, at, 0.75);
    this.send.mix(fin, at, 0.5);
    this.synth.add(subDrop(midi(29) * 1.3, midi(29), 4, 0.8), at, 0.3);
    [72, 76, 79, 84, 88].forEach((n, i) => {
      const b = bell(n, 2.2, 0.25);
      this.synth.add(b, at + 0.5 + i * 0.16, 0.45, i * 0.3 - 0.6);
      this.send.add(b, at + 0.5 + i * 0.16, 0.6);
    });
  }

  render() {
    this.kicks.sort((a, b) => a - b);
    let ki = 0;
    this.synth.shape((t) => {
      while (ki + 1 < this.kicks.length && this.kicks[ki + 1] <= t) ki++;
      const k = this.kicks[ki];
      if (k === undefined || t < k) return 1;
      return 1 - 0.5 * Math.exp(-(t - k) / 0.12);
    });
    const master = new Stereo(this.length + 1);
    master.mix(this.drums, 0, 0.85);
    master.mix(this.synth, 0, 1);
    master.mix(reverb(this.send, 0.88, 0.35, 0), 0, 0.9);
    master.mix(pingPong(this.echo, (BEAT * 3) / 4, 0.38, 3000), 0, 0.5);
    const end = this.length;
    master.shape((t) => {
      for (const [a, b] of this.mutes) {
        if (t >= a && t < b) return clamp((a + 0.03 - t) / 0.03, 0, 1) + clamp((t - (b - 0.01)) / 0.01, 0, 1);
      }
      if (t > end - 2.2) return clamp((end - 0.05 - t) / 2.1, 0, 1);
      return 1;
    });
    for (const ch of [master.L, master.R]) {
      const hp = new SVF();
      const low = new SVF();
      for (let i = 0; i < ch.length; i++) {
        hp.run(ch[i], 32, 0.7);
        low.run(hp.high, 140, 0.7);
        ch[i] = hp.high - low.low * 0.5;
      }
    }
    softClip(master, 1.1);
    return fadeEdges(normalize(master, 0.8), 0.005, 0.2);
  }
}

// ---------------------------------------------------------------- the films

const FILMS: Record<string, () => Stereo> = {
  // src/films/hero.rs: threat 0-5.6 (hits 3, 3.8, 4.8), rewind 5.6-6.2, logo 6.4, replay 7.5, tagline 21.5, CTA 26, end 30.
  hero: () => {
    const s = new Score(30);
    s.tension(0, 5.6, { heartbeat: true, kicksFrom: 3, hatsFrom: 3.8 });
    s.mute(5.6, 6.4);
    s.reveal(6.4, 7.5);
    s.groove(7.5, 21.5, { arpFrom: 9 });
    s.breakdown(21.5, 26);
    s.outro(26);
    return s.render();
  },
  // src/films/hooks.rs, "rm -rf": typed from the first frame, blocked 0.95, denied 4.58, end card 6, end 9.
  hook_rmrf: () => {
    const s = new Score(9);
    s.tension(0, 1, { kicksFrom: 0, hatsFrom: 0 });
    s.mute(1, 1.25);
    s.suspense(1.25, 4.5);
    s.groove(4.5, 6, { arpFrom: 4.5, light: true });
    s.outro(6);
    return s.render();
  },
  // "git push": a light groove from the first frame, approve at 4.94, end card 8.7, end 11.
  hook_push: () => {
    const s = new Score(11);
    s.groove(0, 5, { arpFrom: 1.5, light: true, claps: false });
    s.groove(5, 8.5, { arpFrom: 5 });
    s.outro(8.5);
    return s.render();
  },
  // "tokens": suspense while the agent looks for a token, groove once the phone does the work.
  hook_tokens: () => {
    const s = new Score(12.6);
    s.suspense(0, 3.5);
    s.groove(3.5, 6, { arpFrom: 3.5, light: true, claps: false });
    s.groove(6, 10.3, { arpFrom: 6 });
    s.outro(10.3);
    return s.render();
  },
  // src/films/pay.rs: shopping groove, the cart waits 2.5-7.5, Face ID done 8.45, the cart changes 13.4,
  // declined 14, end card 16.5, end 20.
  pay: () => {
    const s = new Score(20);
    s.groove(0, 2.5, { arpFrom: 0, light: true, claps: false });
    s.suspense(2.5, 8.5, [53, 57, 60, 64]);
    s.groove(8.5, 13.5, { arpFrom: 8.5 });
    s.suspense(13.5, 14.5);
    s.breakdown(14.5, 16.5);
    s.outro(16.5);
    return s.render();
  },
};

// ---------------------------------------------------------------- effects of the ads' own

const EXTRA: Record<string, () => Stereo> = {
  /** Tape rewinding: chirps racing upwards, then a pitch-down stop. */
  rewind: () => {
    const dur = 0.75;
    const s = buf(dur);
    const a = new Saw();
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      const p = t / dur;
      const chirp = 600 + 2400 * ((t * 11) % 1);
      f.run(a.next(chirp * (1 + p)) * 0.5 + noise() * 0.4, expInterp(1500, 6000, p), 1.5);
      s[i] = f.band * Math.min(1, t / 0.05) * (1 - p ** 3) * 0.9;
    }
    return withVerb(stereoSweep(s, 0.6, -0.6), 0.2, 0.6);
  },
  /** A phone buzzing twice on a table. */
  buzz: () => {
    const s = buf(0.55);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      const on = (t < 0.16 ? 1 : 0) + (t > 0.24 && t < 0.4 ? 1 : 0);
      const env = on * Math.min(1, ((t % 0.24) / 0.01));
      f.run(Math.sign(Math.sin(TAU * 150 * t)) + noise() * 0.3, 700, 1.2);
      s[i] = f.low * env * 0.9;
    }
    return mono(s);
  },
  /** Bright two-note chime: a payment went through. */
  chime: () => {
    const out = new Stereo(1.6);
    out.add(bell(84, 1.4, 0.8), 0, 0.8, -0.2);
    out.add(bell(91, 1.4, 0.8), 0.08, 0.8, 0.2);
    out.add(bell(96, 1.2, 0.5), 0.16, 0.6, 0);
    return withVerb(out, 0.35, 1.4);
  },
  /** A red stamp: short, dry, low. */
  stamp: () => {
    const s = buf(0.5);
    const k = subDrop(140, 45, 0.5, 1);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      f.run(noise(), 1800 * Math.exp(-t * 20) + 150, 0.8);
      s[i] = k[i] * 0.9 + f.low * Math.exp(-t * 25) * 0.9;
    }
    return withVerb(mono(s), 0.15, 0.6);
  },
  /** Two-tone alarm for the moment a destructive command lands. */
  alarm: () => {
    const s = buf(0.7);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      const hz = Math.floor(t / 0.12) % 2 === 0 ? 880 : 660;
      f.run(Math.sign(Math.sin(TAU * hz * t)), 2400, 0.7);
      s[i] = f.low * 0.45 * Math.exp(-t * 2.5) * Math.min(1, t / 0.005);
    }
    return withVerb(mono(s), 0.2, 0.8);
  },
  /** A Face ID style "scan": a soft rising sweep. */
  scan: () => withVerb(stereoSweep(sweepNoise(0.7, 400, 5000, 3000, 2.2, 0.8), -0.3, 0.3), 0.3, 0.8),
};

const started = performance.now();
for (const [name, make] of Object.entries({ ...SFX, ...EXTRA })) {
  reseed(name.length * 7919 + name.charCodeAt(0));
  writeWav(`${OUT}sfx_${name}.wav`, fadeEdges(normalize(make(), 0.9)));
}
for (const [name, make] of Object.entries(FILMS)) {
  reseed(42);
  writeWav(`${OUT}music_${name}.wav`, make());
}
console.log(`sound rendered in ${((performance.now() - started) / 1000).toFixed(1)} s → ${OUT}`);
