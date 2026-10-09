// Instruments and sound effects, synthesised from scratch (see dsp.ts). Shared by every film's
// score in build.ts. Taken from the launch film's soundtrack (marketing/launch-video) so the
// ads sound like the same brand, plus a few effects of their own (rewind, buzz, chime, card).

import {
  SR,
  SVF,
  Saw,
  Stereo,
  TAU,
  adsr,
  buf,
  clamp,
  expInterp,
  midi,
  noise,
  reverb,
} from './dsp';

// ---------------------------------------------------------------- instruments

export function kick(level = 1, tone = 1) {
  const s = buf(0.55);
  let ph = 0;
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    const f = 44 * tone + 120 * Math.exp(-t * 32);
    ph += (TAU * f) / SR;
    const body = Math.sin(ph) * Math.exp(-t * 6.5);
    const click = t < 0.003 ? noise() * (1 - t / 0.003) * 0.5 : 0;
    s[i] = Math.tanh((body + click) * 1.8) * level;
  }
  return s;
}

export function clap(level = 1) {
  const s = buf(0.35);
  const f = new SVF();
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    let env = Math.exp(-t * 18);
    for (const o of [0, 0.011, 0.023]) if (t >= o && t < o + 0.01) env = Math.max(env, Math.exp(-(t - o) * 300));
    f.run(noise() * env, 1400, 1.4);
    s[i] = f.band * 1.6 * level;
  }
  return s;
}

export function hat(open = false, level = 1) {
  const s = buf(open ? 0.35 : 0.08);
  const f = new SVF();
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    f.run(noise(), 8000, 0.8);
    s[i] = f.high * Math.exp(-t * (open ? 11 : 60)) * level;
  }
  return s;
}

export function crash() {
  const s = buf(2.5);
  const f = new SVF();
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    f.run(noise(), 5500, 0.6);
    s[i] = f.high * Math.exp(-t * 1.8) * 0.6;
  }
  return s;
}

/** Saw bass with a plucked filter; `cut` is the filter ceiling. */
export function bass(note: number, dur: number, cut = 700, level = 1) {
  const s = buf(dur + 0.15);
  const a = new Saw();
  const b = new Saw();
  const f = new SVF();
  const hz = midi(note);
  let ph = 0;
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    ph += (TAU * hz) / SR;
    const env = adsr(t, dur, 0.004, 0.18, 0.55, 0.08);
    const raw = (a.next(hz) + b.next(hz * 1.004)) * 0.5;
    f.run(raw, 120 + cut * Math.exp(-t * 9) + cut * 0.25, 0.9);
    s[i] = (f.low * 0.8 + Math.sin(ph) * 0.7) * env * level;
  }
  return s;
}

/** Detuned-saw pad, returned in stereo with voices spread across the field. */
export function pad(notes: number[], dur: number, o: { cut?: number; cutEnd?: number; attack?: number; release?: number } = {}) {
  const { cut = 1400, cutEnd = cut, attack = 0.5, release = 1.4 } = o;
  const total = dur + release;
  const out = new Stereo(total);
  notes.forEach((n, vi) => {
    const s = buf(total);
    const oscs = [new Saw(), new Saw(), new Saw()];
    const det = [1, 1.0045, 0.9958];
    const f = new SVF();
    const hz = midi(n);
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      let v = 0;
      for (let k = 0; k < 3; k++) v += oscs[k].next(hz * det[k] * (1 + 0.0015 * Math.sin(TAU * (0.2 + k * 0.07) * t)));
      f.run(v / 3, cut + (cutEnd - cut) * clamp(t / total, 0, 1), 0.8);
      s[i] = f.low * adsr(t, dur, attack, 0.6, 0.85, release);
    }
    out.add(s, 0, 0.32, ((vi / Math.max(1, notes.length - 1)) * 2 - 1) * 0.7);
  });
  return out;
}

export function pluck(note: number, level = 1) {
  const s = buf(0.6);
  const a = new Saw();
  const f = new SVF();
  const hz = midi(note);
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    f.run(a.next(hz) + Math.sin(TAU * hz * 2 * t) * 0.2, 300 + 4200 * Math.exp(-t * 22), 1.2);
    s[i] = f.low * Math.exp(-t * 7) * level;
  }
  return s;
}

export function bell(note: number, dur = 1.6, level = 1) {
  const s = buf(dur);
  const hz = midi(note);
  const partials = [
    [1, 1, 3],
    [2.0, 0.35, 5],
    [3.01, 0.18, 7],
    [4.17, 0.1, 9],
  ];
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    let v = 0;
    for (const [r, g, d] of partials) v += Math.sin(TAU * hz * r * t) * g * Math.exp(-t * d);
    s[i] = v * Math.min(1, t / 0.002) * 0.5 * level;
  }
  return s;
}

export function riser(dur: number) {
  const s = buf(dur);
  const f = new SVF();
  const a = new Saw();
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    const p = t / dur;
    f.run(noise() * 0.8 + a.next(expInterp(110, 880, p)) * 0.25, expInterp(200, 9000, p), 2.2);
    s[i] = f.band * p ** 2.2;
  }
  return s;
}

export function subDrop(from: number, to: number, dur: number, level = 1) {
  const s = buf(dur);
  let ph = 0;
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    ph += (TAU * expInterp(from, to, t / dur)) / SR;
    s[i] = Math.tanh(Math.sin(ph) * 1.6) * Math.exp(-t * (3 / dur)) * Math.min(1, t / 0.003) * level;
  }
  return s;
}

export function drone(note: number, dur: number) {
  const s = buf(dur);
  const a = new Saw();
  const f = new SVF();
  const hz = midi(note);
  for (let i = 0; i < s.length; i++) {
    const t = i / SR;
    f.run(a.next(hz), 140 + 900 * (t / dur) ** 2, 1.1);
    s[i] = f.low * 0.7 + Math.sin(TAU * hz * 2 * t) * 0.35;
  }
  return s;
}

// ---------------------------------------------------------------- sound effects

export function sweepNoise(dur: number, from: number, peak: number, to: number, q: number, peakAt = 0.45) {
  const s = buf(dur);
  const f = new SVF();
  for (let i = 0; i < s.length; i++) {
    const p = i / s.length;
    const cut = p < peakAt ? expInterp(from, peak, p / peakAt) : expInterp(peak, to, (p - peakAt) / (1 - peakAt));
    f.run(noise(), cut, q);
    const env = p < peakAt ? (p / peakAt) ** 1.6 : (1 - (p - peakAt) / (1 - peakAt)) ** 1.3;
    s[i] = f.band * env;
  }
  return s;
}

export function stereoSweep(mono: Float32Array, fromPan = -0.7, toPan = 0.7) {
  const out = new Stereo(mono.length / SR);
  for (let i = 0; i < mono.length; i++) {
    const p = i / mono.length;
    const a = (((fromPan + (toPan - fromPan) * p) + 1) * Math.PI) / 4;
    out.L[i] = mono[i] * Math.cos(a) * Math.SQRT2;
    out.R[i] = mono[i] * Math.sin(a) * Math.SQRT2;
  }
  return out;
}

export function withVerb(dry: Stereo, wet = 0.3, tail = 1.5, room = 0.8) {
  const out = new Stereo(dry.seconds + tail);
  out.mix(dry);
  out.mix(reverb(dry, room, 0.3, tail), 0, wet);
  return out;
}

export const mono = (s: Float32Array, pan = 0) => {
  const out = new Stereo(s.length / SR);
  out.add(s, 0, 1, pan);
  return out;
};

export const SFX: Record<string, () => Stereo> = {
  whoosh: () => withVerb(stereoSweep(sweepNoise(0.6, 250, 4500, 900, 1.3)), 0.25, 1),
  whooshLow: () => withVerb(stereoSweep(sweepNoise(0.8, 120, 1800, 300, 1.1, 0.5), 0.6, -0.6), 0.25, 1),
  swish: () => withVerb(stereoSweep(sweepNoise(0.3, 900, 7000, 2500, 1.6, 0.35)), 0.15, 0.6),
  impact: () => {
    const s = buf(1.4);
    const k = subDrop(95, 32, 1.4, 1);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      f.run(noise(), 2400 * Math.exp(-t * 6) + 200, 0.7);
      s[i] = k[i] * 0.9 + f.low * Math.exp(-t * 9) * 0.7 + Math.sin(TAU * 110 * t) * Math.exp(-t * 10) * 0.4;
    }
    return withVerb(mono(s), 0.35, 2, 0.85);
  },
  pop: () => {
    const s = buf(0.14);
    let ph = 0;
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      ph += (TAU * (520 + 520 * Math.min(1, t / 0.025))) / SR;
      s[i] = Math.sin(ph) * Math.exp(-t * 38) * Math.min(1, t / 0.001);
    }
    return withVerb(mono(s), 0.12, 0.4);
  },
  tick: () => {
    const s = buf(0.03);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      f.run(noise(), 3500, 0.9);
      s[i] = (f.high * 0.8 + Math.sin(TAU * 2100 * t) * 0.4) * Math.exp(-t * 400);
    }
    return mono(s);
  },
  glitch: () => {
    const s = buf(0.45);
    let i = 0;
    while (i < s.length) {
      const segLen = Math.round((0.012 + Math.abs(noise()) * 0.03) * SR);
      const kind = Math.floor(Math.abs(noise()) * 3);
      const hz = 80 + Math.abs(noise()) * 1800;
      for (let j = 0; j < segLen && i < s.length; j++, i++) {
        const t = i / SR;
        const env = Math.exp(-t * 5);
        if (kind === 0) s[i] = Math.sign(Math.sin((TAU * hz * j) / SR)) * 0.5 * env;
        else if (kind === 1) s[i] = Math.round(noise() * 4) / 4 * 0.6 * env;
        else s[i] = 0;
      }
    }
    const thump = subDrop(120, 40, 0.3, 0.8);
    for (let k = 0; k < thump.length; k++) s[k] += thump[k];
    return withVerb(mono(s), 0.12, 0.5);
  },
  boom: () => {
    const s = buf(3);
    const k = subDrop(80, 26, 3, 1);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      f.run(noise(), 900 * Math.exp(-t * 3) + 80, 0.7);
      s[i] = k[i] + f.low * Math.exp(-t * 4) * 0.8;
    }
    return withVerb(mono(s), 0.45, 3, 0.9);
  },
  collapse: () => {
    const dur = 0.55;
    const s = buf(dur);
    const f = new SVF();
    const a = new Saw();
    for (let i = 0; i < s.length; i++) {
      const p = i / s.length;
      f.run(noise() * 0.8 + a.next(expInterp(600, 60, p)) * 0.4, expInterp(7000, 150, p ** 0.7), 1.6);
      s[i] = f.band * p ** 1.8;
    }
    return stereoSweep(s, 0.8, 0);
  },
  notify: () => {
    const out = new Stereo(1.2);
    out.add(bell(81, 1.1, 0.8), 0, 0.8, -0.1);
    out.add(bell(88, 1.0, 0.8), 0.09, 0.8, 0.1);
    return withVerb(out, 0.25, 1);
  },
  tap: () => {
    const s = buf(0.08);
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      s[i] = Math.sin(TAU * 1700 * t) * Math.exp(-t * 900) * 0.6 + Math.sin(TAU * 160 * t) * Math.exp(-t * 70) * 0.8;
    }
    return mono(s);
  },
  haptic: () => {
    const s = buf(0.3);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      const on = (t < 0.07 ? 1 : 0) + (t > 0.13 && t < 0.2 ? 1 : 0);
      f.run(Math.sign(Math.sin(TAU * 165 * t)) * on, 380, 0.7);
      s[i] = f.low * 0.9;
    }
    return mono(s);
  },
  success: () => {
    const out = new Stereo(1.4);
    [79, 86, 91].forEach((n, i) => out.add(bell(n, 1.2, 0.7), i * 0.065, 0.7, i * 0.25 - 0.25));
    return withVerb(out, 0.3, 1.2);
  },
  deny: () => {
    const s = buf(0.4);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      const on = t < 0.11 ? 1 : t > 0.16 && t < 0.29 ? 1 : 0;
      const hz = t < 0.14 ? 233 : 196;
      f.run(Math.sign(Math.sin(TAU * hz * t)) * on, 1100, 0.8);
      s[i] = f.low * 0.7;
    }
    return withVerb(mono(s), 0.15, 0.6);
  },
  shimmer: () => {
    const out = new Stereo(1.6);
    const notes = [84, 88, 91, 93, 96, 100];
    notes.forEach((n, i) => out.add(bell(n, 1.4, 0.4), i * 0.07, 0.6, (i / 5) * 1.6 - 0.8));
    return withVerb(out, 0.6, 2.5, 0.88);
  },
  lock: () => {
    const s = buf(0.4);
    const f = new SVF();
    for (let i = 0; i < s.length; i++) {
      const t = i / SR;
      f.run(noise(), 4000, 1.2);
      const clicks = Math.exp(-t * 700) + (t > 0.05 ? Math.exp(-(t - 0.05) * 600) : 0);
      s[i] = f.band * clicks * 1.2 + Math.sin(TAU * 95 * t) * Math.exp(-t * 25) * 0.7;
    }
    const out = mono(s);
    out.add(bell(96, 0.6, 0.25), 0.05, 0.4);
    return withVerb(out, 0.2, 0.6);
  },
};

