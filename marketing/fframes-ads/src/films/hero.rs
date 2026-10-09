//! The 30 second hero ad.
//!
//! An agent force-pushes, deletes and emails everyone. The tape rewinds, Reins appears, and the
//! same push now waits for the phone: one tap stops it, one tap approves the next one. Keys stay
//! on the phone. "Give your AI agents real power without handing them control of your life."
//!
//! Every time below is in seconds from the start, and most land on a word of the voiceover
//! (the comments name the word). sound/build.ts scores the music to the same beats.

use fframes::{AudioTrack, FFramesContext, Frame, Svgr};

use super::{music, sfx, typing, vo};
use crate::kit::*;
use crate::ui::phone::{Kind, Phone, Req, phone};
use crate::ui::terminal::{Line, Term, terminal};
use crate::ui::{Tone, Word, Words, backdrop, end_card, flash, lockup, words};

pub const BEATS: &[(&str, f32)] = &[
    ("Threat", 5.6),
    ("Rewind", 0.6),
    ("Logo", 1.25),
    ("Stop", 6.15),
    ("Approve", 3.0),
    ("Keys", 4.8),
    ("Tagline", 4.5),
    ("Cta", 4.1),
];

// The three disasters: typed, then executed on "force-pushes", "deletes", "emails everyone".
const CMD1: f32 = 2.0;
const HIT1: f32 = 3.0;
const CMD2: f32 = 3.2;
const HIT2: f32 = 3.8;
const CMD3: f32 = 3.95;
const HIT3: f32 = 4.8;
/// The tape runs backwards from here to the first frame.
const REWIND: f32 = 5.6;
const REWIND_END: f32 = 6.2;
const LOGO: f32 = 6.4;
const REPLAY: f32 = 7.45;
const CMD4: f32 = 7.6;
const PHONE_IN: f32 = 7.9;
const KEYS: f32 = 16.7;
const RESULT: f32 = 19.55;
const OUT: f32 = 21.35;
const CTA: f32 = 26.0;

const REQS: [Req; 2] = [
    Req {
        kind: Kind::ForcePush,
        buzz: 8.6,
        up: 8.95,
        tap: 13.2,
        approve: false,
    },
    Req {
        kind: Kind::Push,
        buzz: 14.55,
        up: 14.9,
        tap: 15.65,
        approve: true,
    },
];

const THREAT_LINES: &[Line] = &[
    Line::Out(-1.0, "# agent: cleaning up the repo", c::TERTIARY),
    Line::Out(-1.0, "found 3 things to tidy", c::SECONDARY),
    Line::Cmd(CMD1, "git push --force origin main"),
    Line::Out(HIT1, "+ 8f3c2a1...e2a4c66 main (forced update)", c::DANGER),
    Line::Cmd(CMD2, "rm -rf ~/octo"),
    Line::Out(HIT2, "removed 18,402 files", c::DANGER),
    Line::Cmd(CMD3, "mail all-staff@acme.com"),
    Line::Out(HIT3, "sent to 1,204 people", c::DANGER),
];

const REPLAY_LINES: &[Line] = &[
    Line::Out(REPLAY, "# agent: cleaning up the repo", c::TERTIARY),
    Line::Cmd(CMD4, "git push --force origin main"),
    Line::Wait(8.58, 13.25, "waiting for your phone..."),
    Line::Done(13.3, false, "denied on your phone. nothing pushed."),
    Line::Cmd(13.75, "git push origin login"),
    Line::Wait(14.5, 15.7, "waiting for your phone..."),
    Line::Done(15.75, true, "pushed 5 commits to octo/app"),
    Line::Cmd(16.9, "echo $GITHUB_TOKEN"),
    Line::Out(17.6, "(empty: the token stays on the phone)", c::TERTIARY),
    Line::Done(20.0, true, "result: ok. key: never sent."),
];

// Captions, word by word on the voiceover.
const C_KEYS: &[Word] = &[
    ("Your", -0.2, Tone::Text),
    ("AI", 0.51, Tone::Text),
    ("agent", 0.91, Tone::Text),
    ("/", 0.0, Tone::Text),
    ("has", 1.39, Tone::Text),
    ("your", 1.63, Tone::Text),
    ("keys.", 1.87, Tone::Accent),
];
const C_SEE: &[Word] = &[
    ("See", 9.27, Tone::Text),
    ("exactly", 9.43, Tone::Accent),
    ("what", 10.15, Tone::Text),
    ("will", 10.31, Tone::Text),
    ("happen.", 10.47, Tone::Text),
];
const C_OFF: &[Word] = &[("Something", 11.51, Tone::Text), ("looks", 11.74, Tone::Text), ("off?", 12.02, Tone::Danger)];
const C_STOP: &[Word] = &[
    ("One", 12.67, Tone::Text),
    ("tap", 12.99, Tone::Text),
    ("stops", 13.23, Tone::Danger),
    ("it.", 13.63, Tone::Danger),
];
const C_DONE: &[Word] = &[("One", 15.48, Tone::Text), ("tap.", 15.64, Tone::Text), ("Done.", 16.28, Tone::Success)];
const C_NEVER: &[Word] = &[
    ("Your", 16.89, Tone::Text),
    ("keys", 17.05, Tone::Accent),
    ("never", 17.53, Tone::Text),
    ("leave", 17.93, Tone::Text),
    ("your", 18.09, Tone::Text),
    ("phone.", 18.25, Tone::Text),
];
const C_RESULT: &[Word] = &[
    ("Agents", 19.03, Tone::Text),
    ("get", 19.43, Tone::Text),
    ("the", 19.55, Tone::Text),
    ("result.", 19.95, Tone::Success),
    ("/", 0.0, Tone::Text),
    ("Never", 20.47, Tone::Accent),
    ("the", 20.71, Tone::Accent),
    ("key.", 20.87, Tone::Accent),
];
const TAGLINE: &[Word] = &[
    ("Give", 21.8, Tone::Text),
    ("your", 22.04, Tone::Text),
    ("AI", 22.36, Tone::Text),
    ("agents", 22.6, Tone::Text),
    ("real", 22.84, Tone::Accent),
    ("power", 23.24, Tone::Accent),
    ("/", 0.0, Tone::Text),
    ("without", 23.9, Tone::Dim),
    ("handing", 24.05, Tone::Dim),
    ("them", 24.2, Tone::Dim),
    ("control", 24.4, Tone::Text),
    ("of", 24.55, Tone::Text),
    ("your", 24.65, Tone::Text),
    ("life.", 24.8, Tone::Text),
];

pub fn audio() -> Vec<AudioTrack<'static>> {
    let mut a = vec![
        music("music_hero.wav", -11.0),
        vo("vo01.mp3", 0.15),
        vo("vo04.mp3", 2.55),
        vo("vo06.mp3", 6.15),
        vo("vo09.mp3", 8.9),
        vo("vo12.mp3", 11.2),
        vo("vo13.mp3", 12.5),
        vo("vo10.mp3", 15.3),
        vo("vo14.mp3", 16.6),
        vo("vo15.mp3", 18.9),
        vo("vo19.mp3", 21.6),
        vo("vo20.mp3", 26.0),
    ];
    for (at, cmd) in [
        (CMD1, "git push --force origin main"),
        (CMD2, "rm -rf ~/octo"),
        (CMD3, "mail all-staff@acme.com"),
        (CMD4, "git push --force origin main"),
        (13.75, "git push origin login"),
        (16.9, "echo $GITHUB_TOKEN"),
    ] {
        a.extend(typing(at, cmd, -17.0));
    }
    a.extend([
        sfx("sfx_glitch.wav", HIT1 - 0.02, -10.0),
        sfx("sfx_stamp.wav", HIT1, -8.0),
        sfx("sfx_glitch.wav", HIT2 - 0.02, -10.0),
        sfx("sfx_stamp.wav", HIT2, -7.0),
        sfx("sfx_alarm.wav", 4.58, -15.0),
        sfx("sfx_glitch.wav", HIT3 - 0.02, -9.0),
        sfx("sfx_stamp.wav", HIT3, -6.0),
        sfx("sfx_rewind.wav", REWIND, -9.0),
        sfx("sfx_whoosh.wav", LOGO - 0.25, -12.0),
        sfx("sfx_boom.wav", LOGO, -6.0),
        sfx("sfx_shimmer.wav", LOGO + 0.05, -12.0),
        sfx("sfx_whoosh.wav", REPLAY - 0.1, -14.0),
        sfx("sfx_swish.wav", PHONE_IN, -13.0),
        sfx("sfx_buzz.wav", REQS[0].buzz, -9.0),
        sfx("sfx_notify.wav", REQS[0].buzz, -12.0),
        sfx("sfx_swish.wav", REQS[0].up - 0.1, -15.0),
        sfx("sfx_tap.wav", REQS[0].tap, -7.0),
        sfx("sfx_deny.wav", REQS[0].tap + 0.08, -10.0),
        sfx("sfx_buzz.wav", REQS[1].buzz, -9.0),
        sfx("sfx_notify.wav", REQS[1].buzz, -12.0),
        sfx("sfx_swish.wav", REQS[1].up - 0.1, -15.0),
        sfx("sfx_tap.wav", REQS[1].tap, -7.0),
        sfx("sfx_success.wav", REQS[1].tap + 0.1, -9.0),
        sfx("sfx_whoosh.wav", KEYS - 0.1, -14.0),
        sfx("sfx_lock.wav", KEYS + 1.0, -9.0),
        sfx("sfx_swish.wav", RESULT, -11.0),
        sfx("sfx_pop.wav", RESULT + 0.42, -9.0),
        sfx("sfx_lock.wav", 20.87, -7.0),
        sfx("sfx_whooshLow.wav", OUT, -10.0),
        sfx("sfx_impact.wav", 24.4, -13.0),
        sfx("sfx_boom.wav", CTA, -8.0),
        sfx("sfx_shimmer.wav", CTA + 0.05, -12.0),
    ]);
    a
}

struct Layout {
    threat_words_y: f32,
    threat_size: f32,
    threat_term: (f32, f32, f32, f32, f32),
    stamp_size: f32,
    replay_term: (f32, f32, f32, f32, f32),
    phone: (f32, f32, f32),
    phone_from: (f32, f32),
    caption: (f32, f32, f32, f32),
    tagline: (f32, f32, f32),
    cta_y: f32,
}

fn layout<F: Format>() -> Layout {
    if F::TALL {
        Layout {
            threat_words_y: -470.0,
            threat_size: 108.0,
            threat_term: (0.0, 300.0, 960.0, 700.0, 32.0),
            stamp_size: 140.0,
            replay_term: (0.0, -490.0, 960.0, 240.0, 28.0),
            phone: (0.0, 170.0, 1040.0),
            phone_from: (0.0, 1250.0),
            caption: (0.0, -730.0, 76.0, 960.0),
            tagline: (-260.0, 100.0, 920.0),
            cta_y: -120.0,
        }
    } else {
        Layout {
            threat_words_y: -300.0,
            threat_size: 104.0,
            threat_term: (0.0, 165.0, 1200.0, 500.0, 34.0),
            stamp_size: 150.0,
            replay_term: (-400.0, 110.0, 900.0, 600.0, 30.0),
            phone: (480.0, 0.0, 940.0),
            phone_from: (1100.0, 0.0),
            caption: (-400.0, -330.0, 72.0, 900.0),
            tagline: (-150.0, 104.0, 1560.0),
            cta_y: -90.0,
        }
    }
}

pub fn draw<F: Format>(frame: &mut Frame, ctx: &FFramesContext, t: f32) -> Svgr<'static> {
    let (w, h) = (F::W as f32, F::H as f32);
    let l = layout::<F>();
    // The threat runs on its own clock, which the rewind drives backwards to zero.
    let tt = if t < REWIND {
        t
    } else if t < REWIND_END {
        (REWIND - (t - REWIND) / (REWIND_END - REWIND) * REWIND).max(0.0)
    } else {
        -10.0
    };
    let rewinding = (REWIND..REWIND_END).contains(&t);
    let danger = if t < REWIND_END {
        0.4 * sp(tt, HIT1, M::Soft) + 0.3 * sp(tt, HIT2, M::Soft) + 0.3 * sp(tt, HIT3, M::Soft)
    } else {
        0.0
    };
    let calm = sp(t, LOGO, M::Soft);

    let mut layers = vec![backdrop(t, w, h, danger, calm)];

    // ---- threat (and its rewind)
    if t < REWIND_END {
        let (sx, sy) = shake(tt, &[(HIT1, 14.0), (HIT2, 16.0), (HIT3, 22.0)]);
        let (tx, ty, tw, th, ts) = l.threat_term;
        let alarm = danger * (0.6 + 0.4 * (tt * 9.0).sin().abs());
        let term = terminal(
            frame,
            ctx,
            tt,
            &Term {
                x: tx,
                y: ty,
                w: tw,
                h: th,
                title: "agent · ~/octo/app",
                lines: THREAT_LINES,
                size: ts,
                alarm,
            },
        );
        let (head, _) = words(
            frame,
            ctx,
            tt,
            &Words {
                words: C_KEYS,
                size: l.threat_size,
                y: l.threat_words_y,
                max_width: w * 0.86,
                exit: 2.75,
                ..Default::default()
            },
        );
        let stamps = vec![
            stamp(frame, ctx, tt, HIT1 - 0.02, HIT2 - 0.05, "Force-pushes.", l.threat_words_y + 20.0, l.stamp_size, w),
            stamp(frame, ctx, tt, HIT2 - 0.02, CMD3 + 0.5, "Deletes.", l.threat_words_y + 20.0, l.stamp_size, w),
            stamp(frame, ctx, tt, 4.56, 99.0, "Emails everyone.", l.threat_words_y + 20.0, l.stamp_size, w),
        ];
        let collapse = ease(t, REWIND_END - 0.12, 0.12, E::In);
        layers.push(group(
            trs(sx, sy, 1.0 - collapse * 0.1),
            1.0 - collapse,
            vec![term, head, fframes::svgr!(<g>{stamps}</g>)],
        ));
        for at in [HIT1, HIT2, HIT3] {
            layers.push(flash(tt, at, 0.3, c::DANGER, 0.22, w, h));
        }
    }
    if rewinding {
        layers.push(rewind_overlay(t, w, h));
    }

    // ---- logo
    let logo_v = 1.0 - ease(t, REPLAY - 0.2, 0.22, E::In);
    if t >= REWIND_END - 0.05 && logo_v > 0.0 {
        layers.push(flash(t, LOGO, 0.5, c::LILAC, 0.35, w, h));
        let size = if F::TALL {
            190.0
        } else {
            170.0
        };
        layers.push(group(
            trs(0.0, 0.0, lerp(1.0, 0.92, 1.0 - logo_v)),
            logo_v,
            vec![lockup(frame, ctx, t, LOGO, 0.0, 0.0, size)],
        ));
    }

    // ---- replay: the terminal and the phone
    if (REPLAY..OUT + 0.6).contains(&t) {
        let enter = sp(t, REPLAY, M::Snappy);
        let leave = ease(t, OUT, 0.35, E::In);
        let (tx, ty, tw, th, ts) = l.replay_term;
        let alarm = 0.5 * life(t, REQS[0].buzz, REQS[0].tap + 0.1, M::Soft);
        let term = terminal(
            frame,
            ctx,
            t,
            &Term {
                x: tx,
                y: ty,
                w: tw,
                h: th,
                title: "agent · ~/octo/app",
                lines: REPLAY_LINES,
                size: ts,
                alarm,
            },
        );
        layers.push(group(
            trs(0.0, leave * -60.0, lerp(0.94, 1.0, enter)),
            clamp01(enter * 1.4) * (1.0 - leave),
            vec![term],
        ));

        let p = sp(t, PHONE_IN, M::Heavy);
        let (px, py, ph) = l.phone;
        let (fx, fy) = l.phone_from;
        let x = lerp(px + fx, px, p) + leave * fx * 0.6;
        let y = lerp(py + fy, py, p) + leave * fy * 0.6;
        layers.push(phone(
            frame,
            ctx,
            t,
            &Phone {
                x,
                y,
                height: ph,
                reqs: &REQS,
                keys_at: Some(KEYS),
                wake: 1.0,
            },
        ));
        layers.push(packets::<F>(t, &l));

        let (cx, cy, cs, cw) = l.caption;
        let align = "middle";
        for (ws, exit) in
            [(C_SEE, 11.2), (C_OFF, 12.55), (C_STOP, 14.3), (C_DONE, 16.62), (C_NEVER, 18.85), (C_RESULT, OUT)]
        {
            let (svg, _) = words(
                frame,
                ctx,
                t,
                &Words {
                    words: ws,
                    size: cs,
                    x: cx,
                    y: cy,
                    max_width: cw,
                    exit,
                    align,
                    ..Default::default()
                },
            );
            layers.push(svg);
        }
    }

    // ---- tagline
    if (OUT..CTA + 0.4).contains(&t) {
        let (ty, ts, tw) = l.tagline;
        let (svg, _) = words(
            frame,
            ctx,
            t,
            &Words {
                words: TAGLINE,
                size: ts,
                y: ty,
                max_width: tw,
                exit: CTA - 0.3,
                ..Default::default()
            },
        );
        layers.push(svg);
    }

    // ---- call to action
    if t >= CTA - 0.1 {
        layers.push(flash(t, CTA, 0.45, c::LILAC, 0.25, w, h));
        layers.push(end_card(frame, ctx, t, CTA, l.cta_y, F::TALL, "Keep the reins."));
    }

    fframes::svgr!(<g>{layers}</g>)
}

/// A word slammed onto the screen: big, red, landing from 1.5x with a tilt.
#[allow(clippy::too_many_arguments)]
fn stamp(
    frame: &mut Frame,
    ctx: &FFramesContext,
    t: f32,
    at: f32,
    until: f32,
    s: &'static str,
    y: f32,
    size: f32,
    w: f32,
) -> Svgr<'static> {
    let p = sp(t, at, M::Pop);
    let out = ease(t, until, 0.12, E::In);
    if p <= 0.001 || out >= 1.0 {
        return Svgr::empty();
    }
    let mut size = size;
    let width = measure(frame, ctx, SANS, 800, size, s);
    if width > w * 0.9 {
        size *= w * 0.9 / width;
    }
    let scale = lerp(1.3, 1.0, p);
    let transform = format!("translate(0 {y:.2}) rotate({:.2}) scale({scale:.4})", lerp(-6.0, -2.5, p));
    fframes::svgr!(<g transform={transform} opacity={clamp01(p * 3.0) * (1.0 - out)}>
        {text_ls(0.0, size * 0.35, s, SANS, size, 800, c::DANGER, "middle", -0.03 * size)}
    </g>)
}

/// The rewind: scanlines tearing across the frame and a rewind badge.
fn rewind_overlay(t: f32, w: f32, h: f32) -> Svgr<'static> {
    let n = (t * 60.0).floor();
    let mut lines = vec![];
    for i in 0..9 {
        let y = (hash(n * 0.37 + i as f32 * 3.1) - 0.5) * h;
        let th = 2.0 + hash(n + i as f32) * 10.0;
        lines.push(rect_o(-w / 2.0, y, w, th, 0.0, "#FFFFFF", 0.05 + hash(i as f32 * 7.0 + n) * 0.12));
    }
    let (bx, by) = (
        -w / 2.0 + 96.0,
        -h / 2.0
            + if h > w {
                300.0
            } else {
                90.0
            },
    );
    fframes::svgr!(<g>
        {lines}
        <path d={format!("M{} {} l-30 22 l30 22z M{} {} l-30 22 l30 22z", bx + 30.0, by - 22.0, bx + 62.0, by - 22.0)} fill={c::TEXT} />
        {text_ls(bx + 84.0, by + 13.0, "REWIND", MONO, 36.0, 600, c::TEXT, "start", 4.0)}
    </g>)
}

/// After the approval: the result flies from the phone to the agent, the key tries and stays.
fn packets<F: Format>(t: f32, l: &Layout) -> Svgr<'static> {
    let (px, py, ph) = l.phone;
    let (tx, ty, _, th, _) = l.replay_term;
    let from = if F::TALL {
        (px, py - ph * 0.42)
    } else {
        (px - 140.0, py - 120.0)
    };
    let to = if F::TALL {
        (tx, ty + th * 0.2)
    } else {
        (tx + 120.0, ty + 40.0)
    };
    let mut out = vec![];
    // The result.
    let p = ease(t, RESULT, 0.42, E::OutExpo);
    let v = life(t, RESULT - 0.05, RESULT + 0.5, M::Snappy);
    if v > 0.001 {
        let (x, y) = (lerp(from.0, to.0, p), lerp(from.1, to.1, p));
        out.push(group(
            trs(x, y, lerp(0.7, 1.0, v)),
            v,
            vec![
                rect(-120.0, -34.0, 240.0, 68.0, 34.0, c::SUCCESS),
                icon(Icon::Check, -72.0, 0.0, 28.0, "#062B1E", 3.2),
                text(-46.0, 11.0, "result", SANS, 30.0, 700, "#062B1E", "start"),
            ],
        ));
    }
    // The key: leaves the phone, gets pulled back and locked.
    let k_out = sp(t, 20.4, M::Snappy) - sp(t, 20.8, M::Bouncy);
    let k_v = life(t, 20.38, 21.3, M::Snappy);
    if k_v > 0.001 {
        let reach = 0.22 * k_out;
        let (x, y) = (lerp(from.0, to.0, reach), lerp(from.1, to.1, reach));
        let locked = sp(t, 20.87, M::Bouncy);
        let ic = if locked > 0.5 {
            Icon::Lock
        } else {
            Icon::Key
        };
        out.push(group(
            trs(x, y, lerp(0.7, 1.0, k_v) * lerp(1.0, 1.08, (locked * std::f32::consts::PI).sin().abs())),
            k_v,
            vec![
                rect(-100.0, -34.0, 200.0, 68.0, 34.0, c::WARNING),
                icon(ic, -54.0, 0.0, 28.0, "#2B2000", 3.0),
                text(-28.0, 11.0, "key", SANS, 30.0, 700, "#2B2000", "start"),
            ],
        ));
    }
    fframes::svgr!(<g>{out}</g>)
}
