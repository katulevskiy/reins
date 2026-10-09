//! Short vertical hooks for Reels, Shorts and TikTok (9:16, 9-12 s). Each one opens on its claim
//! from the first frame, shows it happen in the terminal and on the phone, and ends on the mark.
//!
//! They share one stage: a big terminal under the opening line, which shrinks to the top when the
//! phone slides up, and leaves with the phone before the end card.

use fframes::{AudioTrack, FFramesContext, Frame, Svgr};

use super::{music, sfx, typing, vo};
use crate::kit::*;
use crate::ui::phone::{Kind, Phone, Req, phone};
use crate::ui::terminal::{Line, Term, terminal};
use crate::ui::{Tone, Word, Words, backdrop, end_card, flash, words};

struct Stage<'a> {
    lines: &'a [Line],
    reqs: &'a [Req],
    /// The phone slides up and the terminal moves out of its way.
    phone_in: f32,
    keys_at: Option<f32>,
    /// Terminal and phone leave; the end card follows.
    out: f32,
    /// Red glow on the terminal from..until.
    alarm: (f32, f32),
}

const CAPTION_Y: f32 = -730.0;
const OPEN_Y: f32 = -560.0;
const PHONE: (f32, f32, f32) = (0.0, 170.0, 1040.0);
const SMALL_TERM: (f32, f32, f32, f32, f32) = (0.0, -490.0, 960.0, 240.0, 28.0);
const BIG_TERM: (f32, f32, f32, f32, f32) = (0.0, 40.0, 960.0, 560.0, 40.0);

fn stage(frame: &mut Frame, ctx: &FFramesContext, t: f32, s: &Stage) -> Svgr<'static> {
    let leave = ease(t, s.out, 0.35, E::In);
    if leave >= 1.0 {
        return Svgr::empty();
    }
    let m = sp(t, s.phone_in - 0.1, M::Snappy);
    let mix = |a: (f32, f32, f32, f32, f32), b: (f32, f32, f32, f32, f32)| {
        (lerp(a.0, b.0, m), lerp(a.1, b.1, m), lerp(a.2, b.2, m), lerp(a.3, b.3, m), lerp(a.4, b.4, m))
    };
    let (tx, ty, tw, th, ts) = mix(BIG_TERM, SMALL_TERM);
    let alarm = 0.6 * life(t, s.alarm.0, s.alarm.1, M::Soft);
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
            lines: s.lines,
            size: ts,
            alarm,
        },
    );
    let p = sp(t, s.phone_in, M::Heavy);
    let (px, py, ph) = PHONE;
    let ph_svg = if p > 0.001 {
        phone(
            frame,
            ctx,
            t,
            &Phone {
                x: px,
                y: lerp(py + 1250.0, py, p) + leave * 900.0,
                height: ph,
                reqs: s.reqs,
                keys_at: s.keys_at,
                wake: 1.0,
            },
        )
    } else {
        Svgr::empty()
    };
    fframes::svgr!(<g>
        {group(tr(0.0, -leave * 80.0), 1.0 - leave, vec![term])}
        {ph_svg}
    </g>)
}

fn caption(frame: &mut Frame, ctx: &FFramesContext, t: f32, ws: &[Word], exit: f32) -> Svgr<'static> {
    words(
        frame,
        ctx,
        t,
        &Words {
            words: ws,
            size: 76.0,
            y: CAPTION_Y,
            max_width: 960.0,
            exit,
            ..Default::default()
        },
    )
    .0
}

fn opener(frame: &mut Frame, ctx: &FFramesContext, t: f32, ws: &[Word], exit: f32) -> Svgr<'static> {
    words(
        frame,
        ctx,
        t,
        &Words {
            words: ws,
            size: 104.0,
            y: OPEN_Y,
            max_width: 960.0,
            exit,
            ..Default::default()
        },
    )
    .0
}

fn phone_sfx(r: &Req, a: &mut Vec<AudioTrack<'static>>) {
    a.extend([
        sfx("sfx_buzz.wav", r.buzz, -9.0),
        sfx("sfx_notify.wav", r.buzz, -12.0),
        sfx("sfx_swish.wav", r.up - 0.1, -15.0),
        sfx("sfx_tap.wav", r.tap, -7.0),
        if r.approve {
            sfx("sfx_success.wav", r.tap + 0.1, -9.0)
        } else {
            sfx("sfx_deny.wav", r.tap + 0.08, -10.0)
        },
    ]);
}

// ---------------------------------------------------------------- "Your AI just tried to rm -rf /"

pub const RMRF_BEATS: &[(&str, f32)] = &[("Hook", 1.2), ("Phone", 4.6), ("End", 3.2)];
const RMRF_OUT: f32 = 5.7;
const RMRF_REQ: [Req; 1] = [Req {
    kind: Kind::RmRf,
    buzz: 1.6,
    up: 1.95,
    tap: 4.58,
    approve: false,
}];
const RMRF_LINES: &[Line] = &[
    Line::Out(-1.0, "# agent: freeing up disk space", c::TERTIARY),
    Line::Cmd(-0.05, "rm -rf /"),
    Line::Wait(0.95, 4.6, "blocked. waiting for your phone..."),
    Line::Done(4.65, false, "denied on your phone. nothing ran."),
];
const RMRF_OPEN: &[Word] = &[
    ("Your", -0.3, Tone::Text),
    ("AI", -0.2, Tone::Text),
    ("just", -0.1, Tone::Text),
    ("tried", 0.0, Tone::Text),
    ("to", 0.1, Tone::Text),
];
const RMRF_C1: &[Word] = &[("Something", 2.76, Tone::Text), ("looks", 2.99, Tone::Text), ("off?", 3.27, Tone::Danger)];
const RMRF_C2: &[Word] =
    &[("One", 3.92, Tone::Text), ("tap", 4.24, Tone::Text), ("stops", 4.48, Tone::Danger), ("it.", 4.88, Tone::Danger)];

pub fn rmrf_audio() -> Vec<AudioTrack<'static>> {
    let mut a =
        vec![music("music_hook_rmrf.wav", -11.0), vo("vo12.mp3", 2.45), vo("vo13.mp3", 3.75), vo("vo21.mp3", 6.3)];
    a.extend(typing(0.0, "rm -rf /", -15.0));
    a.extend([sfx("sfx_alarm.wav", 0.95, -11.0), sfx("sfx_stamp.wav", 0.95, -7.0), sfx("sfx_whoosh.wav", 1.1, -14.0)]);
    phone_sfx(&RMRF_REQ[0], &mut a);
    a.extend([
        sfx("sfx_whooshLow.wav", RMRF_OUT, -11.0),
        sfx("sfx_boom.wav", RMRF_OUT + 0.3, -8.0),
        sfx("sfx_shimmer.wav", RMRF_OUT + 0.35, -12.0),
    ]);
    a
}

pub fn draw_rmrf<F: Format>(frame: &mut Frame, ctx: &FFramesContext, t: f32) -> Svgr<'static> {
    let (w, h) = (F::W as f32, F::H as f32);
    let danger = 0.6 * life(t, 0.95, 1.6, M::Soft);
    let mut l = vec![backdrop(t, w, h, danger, 0.0)];
    let (sx, sy) = shake(t, &[(0.95, 18.0)]);
    l.push(group(
        tr(sx, sy),
        1.0,
        vec![stage(
            frame,
            ctx,
            t,
            &Stage {
                lines: RMRF_LINES,
                reqs: &RMRF_REQ,
                phone_in: 1.2,
                keys_at: None,
                out: RMRF_OUT,
                alarm: (0.95, 4.6),
            },
        )],
    ));
    l.push(opener(frame, ctx, t, RMRF_OPEN, 1.1));
    l.push(big_command(t, 1.1));
    l.push(flash(t, 0.95, 0.35, c::DANGER, 0.3, w, h));
    l.push(caption(frame, ctx, t, RMRF_C1, 3.75));
    l.push(caption(frame, ctx, t, RMRF_C2, RMRF_OUT));
    if t >= RMRF_OUT {
        l.push(end_card(frame, ctx, t, RMRF_OUT + 0.3, -120.0, true, "Your agent asks first."));
    }
    fframes::svgr!(<g>{l}</g>)
}

/// The opening command, huge, under "Your AI just tried to".
fn big_command(t: f32, exit: f32) -> Svgr<'static> {
    let p = sp(t, -0.25, M::Pop);
    let out = ease(t, exit, 0.2, E::In);
    let n = ((t + 0.05) * 30.0).floor().clamp(0.0, 8.0) as usize;
    let typed: String = "rm -rf /".chars().take(n.max(2)).collect();
    group(
        trs(0.0, -360.0, lerp(0.9, 1.0, p) * (1.0 + out * 0.2)),
        clamp01(p * 2.0) * (1.0 - out),
        vec![text(0.0, 0.0, typed, MONO, 150.0, 600, c::DANGER, "middle")],
    )
}

// ---------------------------------------------------------------- "Approve git push from your phone"

pub const PUSH_BEATS: &[(&str, f32)] = &[("Hook", 1.5), ("Phone", 4.0), ("Result", 3.0), ("End", 2.5)];
const PUSH_OUT: f32 = 8.4;
const PUSH_REQ: [Req; 1] = [Req {
    kind: Kind::Push,
    buzz: 1.6,
    up: 1.95,
    tap: 4.94,
    approve: true,
}];
const PUSH_LINES: &[Line] = &[
    Line::Out(-1.0, "# agent: ship the login fix", c::TERTIARY),
    Line::Cmd(0.25, "git push origin login"),
    Line::Wait(1.0, 4.98, "waiting for your phone..."),
    Line::Done(5.05, true, "pushed 5 commits to octo/app"),
    Line::Out(5.4, "token used: none on this laptop", c::TERTIARY),
];
const PUSH_OPEN: &[Word] = &[
    ("Approve", -0.3, Tone::Text),
    ("git", -0.2, Tone::Accent),
    ("push", -0.1, Tone::Accent),
    ("/", 0.0, Tone::Text),
    ("from", 0.0, Tone::Text),
    ("your", 0.1, Tone::Text),
    ("phone.", 0.2, Tone::Text),
];
const PUSH_C1: &[Word] = &[
    ("See", 2.57, Tone::Text),
    ("exactly", 2.73, Tone::Accent),
    ("what", 3.45, Tone::Text),
    ("will", 3.61, Tone::Text),
    ("happen.", 3.77, Tone::Text),
];
const PUSH_C2: &[Word] = &[("One", 4.78, Tone::Text), ("tap.", 4.94, Tone::Text), ("Done.", 5.58, Tone::Success)];
const PUSH_C3: &[Word] = &[
    ("Agents", 6.23, Tone::Text),
    ("get", 6.63, Tone::Text),
    ("the", 6.75, Tone::Text),
    ("result.", 7.15, Tone::Success),
    ("/", 0.0, Tone::Text),
    ("Never", 7.67, Tone::Accent),
    ("the", 7.91, Tone::Accent),
    ("key.", 8.07, Tone::Accent),
];

pub fn push_audio() -> Vec<AudioTrack<'static>> {
    let mut a =
        vec![music("music_hook_push.wav", -11.0), vo("vo09.mp3", 2.2), vo("vo10.mp3", 4.6), vo("vo15.mp3", 6.1)];
    a.extend(typing(0.25, "git push origin login", -16.0));
    phone_sfx(&PUSH_REQ[0], &mut a);
    a.extend([
        sfx("sfx_pop.wav", 5.4, -12.0),
        sfx("sfx_whooshLow.wav", PUSH_OUT, -11.0),
        sfx("sfx_boom.wav", PUSH_OUT + 0.3, -8.0),
        sfx("sfx_shimmer.wav", PUSH_OUT + 0.35, -12.0),
    ]);
    a
}

pub fn draw_push<F: Format>(frame: &mut Frame, ctx: &FFramesContext, t: f32) -> Svgr<'static> {
    let (w, h) = (F::W as f32, F::H as f32);
    let mut l = vec![backdrop(t, w, h, 0.0, 0.3)];
    l.push(stage(
        frame,
        ctx,
        t,
        &Stage {
            lines: PUSH_LINES,
            reqs: &PUSH_REQ,
            phone_in: 1.3,
            keys_at: None,
            out: PUSH_OUT,
            alarm: (0.0, 0.0),
        },
    ));
    l.push(opener(frame, ctx, t, PUSH_OPEN, 1.25));
    l.push(caption(frame, ctx, t, PUSH_C1, 4.4));
    l.push(caption(frame, ctx, t, PUSH_C2, 6.0));
    l.push(caption(frame, ctx, t, PUSH_C3, PUSH_OUT));
    if t >= PUSH_OUT {
        l.push(end_card(frame, ctx, t, PUSH_OUT + 0.3, -120.0, true, "Your phone holds the keys."));
    }
    fframes::svgr!(<g>{l}</g>)
}

// ---------------------------------------------------------------- "Your agent never sees your tokens"

pub const TOKENS_BEATS: &[(&str, f32)] = &[("Hook", 3.4), ("Phone", 3.4), ("Keys", 3.2), ("End", 2.6)];
const TOKENS_OUT: f32 = 10.0;
const TOKENS_KEYS: f32 = 6.9;
const TOKENS_REQ: [Req; 1] = [Req {
    kind: Kind::Merge,
    buzz: 3.85,
    up: 4.2,
    tap: 6.09,
    approve: true,
}];
const TOKENS_LINES: &[Line] = &[
    Line::Cmd(0.0, "echo $GITHUB_TOKEN"),
    Line::Out(0.75, "(empty)", c::TERTIARY),
    Line::Cmd(1.2, "cat ~/.config/gh/hosts.yml"),
    Line::Out(2.15, "no such file", c::TERTIARY),
    Line::Cmd(2.7, "gh pr merge 42"),
    Line::Wait(3.3, 6.12, "asking your phone..."),
    Line::Done(6.2, true, "merged #42"),
    Line::Out(8.3, "result: ok. token: never seen.", c::SUCCESS),
];
const TOKENS_OPEN: &[Word] = &[
    ("Your", -0.3, Tone::Text),
    ("agent", -0.2, Tone::Text),
    ("never", -0.1, Tone::Accent),
    ("sees", 0.0, Tone::Text),
    ("/", 0.0, Tone::Text),
    ("your", 0.1, Tone::Text),
    ("tokens.", 0.2, Tone::Accent),
];
const TOKENS_C1: &[Word] = &[
    ("Your", 3.35, Tone::Text),
    ("phone", 3.45, Tone::Accent),
    ("does", 3.55, Tone::Text),
    ("the", 3.65, Tone::Text),
    ("work.", 3.75, Tone::Text),
];
const TOKENS_C2: &[Word] = &[("One", 5.93, Tone::Text), ("tap.", 6.09, Tone::Text), ("Done.", 6.73, Tone::Success)];
const TOKENS_C3: &[Word] = &[
    ("Agents", 7.43, Tone::Text),
    ("get", 7.83, Tone::Text),
    ("the", 7.95, Tone::Text),
    ("result.", 8.35, Tone::Success),
    ("/", 0.0, Tone::Text),
    ("Never", 8.87, Tone::Accent),
    ("the", 9.11, Tone::Accent),
    ("key.", 9.27, Tone::Accent),
];

pub fn tokens_audio() -> Vec<AudioTrack<'static>> {
    let mut a = vec![music("music_hook_tokens.wav", -11.0), vo("vo10.mp3", 5.75), vo("vo15.mp3", 7.3)];
    for (at, cmd) in [(0.0, "echo $GITHUB_TOKEN"), (1.2, "cat ~/.config/gh/hosts.yml"), (2.7, "gh pr merge 42")] {
        a.extend(typing(at, cmd, -16.0));
    }
    phone_sfx(&TOKENS_REQ[0], &mut a);
    a.extend([
        sfx("sfx_whoosh.wav", TOKENS_KEYS - 0.1, -14.0),
        sfx("sfx_lock.wav", TOKENS_KEYS + 1.0, -9.0),
        sfx("sfx_pop.wav", 8.3, -11.0),
        sfx("sfx_whooshLow.wav", TOKENS_OUT, -11.0),
        sfx("sfx_boom.wav", TOKENS_OUT + 0.3, -8.0),
        sfx("sfx_shimmer.wav", TOKENS_OUT + 0.35, -12.0),
    ]);
    a
}

pub fn draw_tokens<F: Format>(frame: &mut Frame, ctx: &FFramesContext, t: f32) -> Svgr<'static> {
    let (w, h) = (F::W as f32, F::H as f32);
    let mut l = vec![backdrop(t, w, h, 0.0, 0.3)];
    l.push(stage(
        frame,
        ctx,
        t,
        &Stage {
            lines: TOKENS_LINES,
            reqs: &TOKENS_REQ,
            phone_in: 3.5,
            keys_at: Some(TOKENS_KEYS),
            out: TOKENS_OUT,
            alarm: (0.0, 0.0),
        },
    ));
    l.push(opener(frame, ctx, t, TOKENS_OPEN, 3.0));
    l.push(caption(frame, ctx, t, TOKENS_C1, 5.7));
    l.push(caption(frame, ctx, t, TOKENS_C2, 7.25));
    l.push(caption(frame, ctx, t, TOKENS_C3, TOKENS_OUT));
    if t >= TOKENS_OUT {
        l.push(end_card(frame, ctx, t, TOKENS_OUT + 0.3, -120.0, true, "Keys stay on your phone."));
    }
    fframes::svgr!(<g>{l}</g>)
}
