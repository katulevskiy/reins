// A tiny synthesis toolkit: oscillators, filters, envelopes, a Freeverb and a WAV writer.
// Everything is deterministic (seeded noise), so the same code always renders the same sound.

import { writeFileSync } from 'node:fs';

export const SR = 48000;
const TAU = Math.PI * 2;

export const midi = (n: number) => 440 * 2 ** ((n - 69) / 12);

let seed = 1234567;
export function reseed(s: number) {
  seed = s >>> 0 || 1;
}
export function noise() {
  seed ^= seed << 13;
  seed ^= seed >>> 17;
  seed ^= seed << 5;
  return ((seed >>> 0) / 4294967295) * 2 - 1;
}

/** Stereo buffer with helpers to mix mono or stereo material into it. */
export class Stereo {
  L: Float32Array;
  R: Float32Array;
  constructor(public seconds: number) {
    const n = Math.ceil(seconds * SR);
    this.L = new Float32Array(n);
    this.R = new Float32Array(n);
  }
  get length() {
    return this.L.length;
  }
  /** Add a mono signal at `at` seconds, equal-power panned (-1 left, 1 right). */
  add(sig: Float32Array, at: number, gain = 1, pan = 0) {
    const o = Math.round(at * SR);
    const a = ((pan + 1) * Math.PI) / 4;
    const gl = Math.cos(a) * gain * Math.SQRT2;
    const gr = Math.sin(a) * gain * Math.SQRT2;
    for (let i = 0; i < sig.length; i++) {
      const j = o + i;
      if (j < 0 || j >= this.L.length) continue;
      this.L[j] += sig[i] * gl;
      this.R[j] += sig[i] * gr;
    }
  }
  mix(other: Stereo, at = 0, gain = 1) {
    const o = Math.round(at * SR);
    for (let i = 0; i < other.length; i++) {
      const j = o + i;
      if (j < 0 || j >= this.L.length) continue;
      this.L[j] += other.L[i] * gain;
      this.R[j] += other.R[i] * gain;
    }
  }
  /** Multiply by a gain curve evaluated per sample at time t. */
  shape(fn: (t: number) => number) {
    for (let i = 0; i < this.L.length; i++) {
      const g = fn(i / SR);
      this.L[i] *= g;
      this.R[i] *= g;
    }
  }
}

export const buf = (seconds: number) => new Float32Array(Math.ceil(seconds * SR));

/** Topology-preserving state-variable filter (Zavalishin). */
export class SVF {
  private ic1 = 0;
  private ic2 = 0;
  low = 0;
  band = 0;
  high = 0;
  run(x: number, cutoff: number, q = 0.707) {
    const fc = Math.min(Math.max(cutoff, 20), SR * 0.45);
    const g = Math.tan((Math.PI * fc) / SR);
    const k = 1 / q;
    const a1 = 1 / (1 + g * (g + k));
    const a2 = g * a1;
    const a3 = g * a2;
    const v3 = x - this.ic2;
    const v1 = a1 * this.ic1 + a2 * v3;
    const v2 = this.ic2 + a2 * this.ic1 + a3 * v3;
    this.ic1 = 2 * v1 - this.ic1;
    this.ic2 = 2 * v2 - this.ic2;
    this.low = v2;
    this.band = v1;
    this.high = x - k * v1 - v2;
    return v2;
  }
}

/** Band-limited-ish saw using PolyBLEP. */
export function polyBlep(t: number, dt: number) {
  if (t < dt) {
    t /= dt;
    return t + t - t * t - 1;
  }
  if (t > 1 - dt) {
    t = (t - 1) / dt;
    return t * t + t + t + 1;
  }
  return 0;
}

export class Saw {
  phase = (noise() + 1) / 2;
  next(freq: number) {
    const dt = freq / SR;
    const v = 2 * this.phase - 1 - polyBlep(this.phase, dt);
    this.phase += dt;
    if (this.phase >= 1) this.phase -= 1;
    return v;
  }
}

/** Attack / decay / sustain / release envelope for a note of `dur` seconds (release follows). */
export function adsr(t: number, dur: number, a: number, d: number, s: number, r: number) {
  if (t < 0) return 0;
  let v: number;
  if (t < a) v = t / a;
  else if (t < a + d) v = 1 - ((t - a) / d) * (1 - s);
  else v = s;
  if (t > dur) {
    const held = dur < a ? dur / a : dur < a + d ? 1 - ((dur - a) / d) * (1 - s) : s;
    v = held * Math.max(0, 1 - (t - dur) / r);
  }
  return v;
}

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
export const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
export const expInterp = (a: number, b: number, t: number) => a * (b / a) ** clamp(t, 0, 1);

// ---------- Freeverb ----------

class Comb {
  private buf: Float32Array;
  private i = 0;
  private store = 0;
  constructor(size: number, private feedback: number, private damp: number) {
    this.buf = new Float32Array(size);
  }
  run(x: number) {
    const out = this.buf[this.i];
    this.store = out * (1 - this.damp) + this.store * this.damp;
    this.buf[this.i] = x + this.store * this.feedback;
    if (++this.i >= this.buf.length) this.i = 0;
    return out;
  }
}

class Allpass {
  private buf: Float32Array;
  private i = 0;
  constructor(size: number) {
    this.buf = new Float32Array(size);
  }
  run(x: number) {
    const b = this.buf[this.i];
    const out = -x + b;
    this.buf[this.i] = x + b * 0.5;
    if (++this.i >= this.buf.length) this.i = 0;
    return out;
  }
}

/** Returns a wet-only stereo reverb of `input`, with `tail` extra seconds. */
export function reverb(input: Stereo, room = 0.84, damp = 0.3, tail = 3): Stereo {
  const scale = SR / 44100;
  const combs = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
  const aps = [556, 441, 341, 225];
  const mk = (spread: number) => ({
    c: combs.map((n) => new Comb(Math.round((n + spread) * scale), room, damp)),
    a: aps.map((n) => new Allpass(Math.round((n + spread) * scale))),
  });
  const l = mk(0);
  const r = mk(23);
  const out = new Stereo(input.seconds + tail);
  for (let i = 0; i < out.length; i++) {
    const x = i < input.length ? (input.L[i] + input.R[i]) * 0.015 : 0;
    let yl = 0;
    let yr = 0;
    for (const c of l.c) yl += c.run(x);
    for (const c of r.c) yr += c.run(x);
    for (const a of l.a) yl = a.run(yl);
    for (const a of r.a) yr = a.run(yr);
    out.L[i] = yl;
    out.R[i] = yr;
  }
  return out;
}

/** Ping-pong delay, wet only. */
export function pingPong(input: Stereo, time: number, feedback = 0.4, tone = 3500): Stereo {
  const d = Math.round(time * SR);
  const out = new Stereo(input.seconds + time * 8);
  const fl = new SVF();
  const fr = new SVF();
  for (let i = 0; i < out.length; i++) {
    const inL = i < input.length ? input.L[i] : 0;
    const inR = i < input.length ? input.R[i] : 0;
    const dl = i >= d ? out.R[i - d] : 0;
    const dr = i >= d ? out.L[i - d] : 0;
    out.L[i] = fl.run((inL + inR) * 0.5 + dl * feedback, tone);
    out.R[i] = fr.run(dr * feedback, tone);
  }
  return out;
}

export function normalize(s: Stereo, peak = 0.89) {
  let m = 0;
  for (let i = 0; i < s.length; i++) m = Math.max(m, Math.abs(s.L[i]), Math.abs(s.R[i]));
  if (m === 0) return s;
  const g = peak / m;
  for (let i = 0; i < s.length; i++) {
    s.L[i] *= g;
    s.R[i] *= g;
  }
  return s;
}

export function softClip(s: Stereo, drive = 1.2) {
  const k = Math.tanh(drive);
  for (let i = 0; i < s.length; i++) {
    s.L[i] = Math.tanh(s.L[i] * drive) / k;
    s.R[i] = Math.tanh(s.R[i] * drive) / k;
  }
  return s;
}

/** Short fades at both ends so nothing clicks. */
export function fadeEdges(s: Stereo, fadeIn = 0.003, fadeOut = 0.05) {
  const fi = Math.round(fadeIn * SR);
  const fo = Math.round(fadeOut * SR);
  for (let i = 0; i < fi && i < s.length; i++) {
    s.L[i] *= i / fi;
    s.R[i] *= i / fi;
  }
  for (let i = 0; i < fo && i < s.length; i++) {
    const j = s.length - 1 - i;
    s.L[j] *= i / fo;
    s.R[j] *= i / fo;
  }
  return s;
}

export function writeWav(path: string, s: Stereo) {
  const n = s.length;
  const data = Buffer.alloc(44 + n * 4);
  data.write('RIFF', 0);
  data.writeUInt32LE(36 + n * 4, 4);
  data.write('WAVE', 8);
  data.write('fmt ', 12);
  data.writeUInt32LE(16, 16);
  data.writeUInt16LE(1, 20);
  data.writeUInt16LE(2, 22);
  data.writeUInt32LE(SR, 24);
  data.writeUInt32LE(SR * 4, 28);
  data.writeUInt16LE(4, 32);
  data.writeUInt16LE(16, 34);
  data.write('data', 36);
  data.writeUInt32LE(n * 4, 40);
  for (let i = 0; i < n; i++) {
    data.writeInt16LE(Math.round(clamp(s.L[i], -1, 1) * 32767), 44 + i * 4);
    data.writeInt16LE(Math.round(clamp(s.R[i], -1, 1) * 32767), 46 + i * 4);
  }
  writeFileSync(path, data);
}

export { TAU };
