//! The agent's terminal: commands typed at a set speed, output lines, a "waiting for your phone"
//! spinner and result lines. Lines scroll up once the window is full.

use fframes::{FFramesContext, Frame, Svgr};

use crate::kit::*;

#[derive(Clone, Copy)]
pub enum Line {
    /// A command typed from `at`, 30 characters a second.
    Cmd(f32, &'static str),
    /// Output that appears at `at`.
    Out(f32, &'static str, &'static str),
    /// A spinner and a note from `at` until the second value, then a pause mark.
    Wait(f32, f32, &'static str),
    /// A check (true) or a cross (false) and a line in green or red.
    Done(f32, bool, &'static str),
}

pub const CPS: f32 = 30.0;

impl Line {
    fn at(&self) -> f32 {
        match *self {
            Line::Cmd(a, _) | Line::Out(a, _, _) | Line::Wait(a, _, _) | Line::Done(a, _, _) => a,
        }
    }
}

pub struct Term<'l> {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub title: &'static str,
    pub lines: &'l [Line],
    pub size: f32,
    /// Red glow around the window (0..1).
    pub alarm: f32,
}

/// The window, centred on (x, y). `t` is the terminal's own clock (it can run backwards).
pub fn terminal(frame: &mut Frame, ctx: &FFramesContext, t: f32, o: &Term) -> Svgr<'static> {
    let (left, top) = (o.x - o.w / 2.0, o.y - o.h / 2.0);
    let bar = o.size * 2.3;
    let lh = o.size * 1.6;
    let pad = o.size * 1.4;
    let body_top = top + bar + pad * 0.9;
    let max_lines = ((o.h - bar - pad * 1.4) / lh).floor().max(1.0) as usize;

    let shown: Vec<&Line> = o.lines.iter().filter(|l| l.at() <= t).collect();
    // Scroll by one line, springing, every time a line beyond the window's height arrives.
    let scroll: f32 = shown.iter().skip(max_lines).map(|l| sp(t, l.at(), M::Snappy)).sum();
    let cw = measure(frame, ctx, MONO, 400, o.size, "0");

    let mut rows = vec![];
    for (i, line) in shown.iter().enumerate() {
        let y = body_top + (i as f32 - scroll) * lh + o.size * 0.35;
        let fade = clamp01((y - body_top + lh * 0.6) / (lh * 0.8));
        if fade <= 0.0 {
            continue;
        }
        let enter = sp(t, line.at(), M::Snappy);
        let last = i + 1 == shown.len();
        let row = match **line {
            Line::Cmd(at, cmd) => {
                let n = (((t - at) * CPS).floor().max(0.0) as usize).min(cmd.chars().count());
                let typed: String = cmd.chars().take(n).collect();
                let blink = if last && ((t * 2.0).fract() < 0.6 || n < cmd.chars().count()) {
                    1.0
                } else {
                    0.0
                };
                let cx = left + pad + cw * 2.0 + cw * n as f32;
                fframes::svgr!(<g>
                    {text(left + pad, y, "$", MONO, o.size, 600, c::ACCENT, "start")}
                    {text(left + pad + cw * 2.0, y, typed, MONO, o.size, 500, c::TEXT, "start")}
                    {rect_o(cx + 2.0, y - o.size * 0.82, cw * 0.62, o.size * 1.05, 2.0, c::LILAC, blink)}
                </g>)
            }
            Line::Out(_, s, color) => group(
                tr(0.0, (1.0 - enter) * 10.0),
                clamp01(enter * 1.5),
                vec![text(left + pad + cw * 2.0, y, s, MONO, o.size, 400, color, "start")],
            ),
            Line::Wait(at, until, s) => {
                let spinning = t < until;
                let a = (t - at) * 540.0;
                let ix = left + pad + cw * 2.0 + o.size * 0.45;
                let iy = y - o.size * 0.34;
                let r = o.size * 0.42;
                let ic = if spinning {
                    let arc = format!("M{:.2} {:.2} a{r:.2} {r:.2} 0 1 1 {:.2} {:.2}", ix + r, iy, -r, -r);
                    let rot = format!("rotate({a:.1} {ix:.2} {iy:.2})");
                    fframes::svgr!(<g>
                        <circle cx={ix} cy={iy} r={r} fill="none" stroke={c::WARNING} stroke-opacity="0.25" stroke-width={o.size * 0.14} />
                        <path d={arc} transform={rot} fill="none" stroke={c::WARNING} stroke-width={o.size * 0.14} stroke-linecap="round" />
                    </g>)
                } else {
                    fframes::svgr!(<g>
                        {rect(ix - r * 0.55, iy - r * 0.7, r * 0.36, r * 1.4, 1.5, c::TERTIARY)}
                        {rect(ix + r * 0.2, iy - r * 0.7, r * 0.36, r * 1.4, 1.5, c::TERTIARY)}
                    </g>)
                };
                let pulse = if spinning {
                    0.75 + 0.25 * ((t - at) * 6.0).sin()
                } else {
                    0.55
                };
                group(
                    tr(0.0, (1.0 - enter) * 10.0),
                    clamp01(enter * 1.5),
                    vec![
                        ic,
                        group(
                            String::new(),
                            pulse,
                            vec![text(
                                ix + o.size * 1.0,
                                y,
                                s,
                                MONO,
                                o.size,
                                500,
                                if spinning {
                                    c::WARNING
                                } else {
                                    c::TERTIARY
                                },
                                "start",
                            )],
                        ),
                    ],
                )
            }
            Line::Done(_, ok, s) => {
                let color = if ok {
                    c::SUCCESS
                } else {
                    c::DANGER
                };
                let ix = left + pad + cw * 2.0 + o.size * 0.45;
                let pop = sp(t, line.at(), M::Bouncy);
                group(
                    tr(0.0, (1.0 - enter) * 10.0),
                    clamp01(enter * 1.5),
                    vec![
                        group(
                            trs(ix, y - o.size * 0.34, lerp(0.4, 1.0, pop)),
                            1.0,
                            vec![icon(
                                if ok {
                                    Icon::Check
                                } else {
                                    Icon::X
                                },
                                0.0,
                                0.0,
                                o.size * 1.05,
                                color,
                                3.0,
                            )],
                        ),
                        text(ix + o.size * 1.0, y, s, MONO, o.size, 600, color, "start"),
                    ],
                )
            }
        };
        rows.push(group(String::new(), fade, vec![row]));
    }

    let clip = format!("term-{}", (o.x * 7.0 + o.y * 13.0 + o.w) as i64);
    let clip_url = format!("url(#{clip})");
    let glow = clamp01(o.alarm);
    fframes::svgr!(<g>
        <defs>
            <clipPath id={clip.clone()}>
                <rect x={left} y={top + bar} width={o.w} height={o.h - bar} />
            </clipPath>
        </defs>
        {rect_o(left - 18.0, top - 18.0, o.w + 36.0, o.h + 36.0, 34.0, c::DANGER_DEEP, glow * 0.18)}
        {rect_o(left, top + 24.0, o.w, o.h, 22.0, "#000000", 0.55)}
        {rect(left, top, o.w, o.h, 22.0, "#0C0C0E")}
        {rect(left, top, o.w, bar, 22.0, c::RAISED)}
        {rect(left, top + bar - 22.0, o.w, 22.0, 0.0, c::RAISED)}
        {rect(left, top + bar - 1.0, o.w, 1.0, 0.0, c::HAIRLINE)}
        {circle(left + bar * 0.5, top + bar / 2.0, bar * 0.13, "#3A3A40", 1.0)}
        {circle(left + bar * 0.85, top + bar / 2.0, bar * 0.13, "#3A3A40", 1.0)}
        {circle(left + bar * 1.2, top + bar / 2.0, bar * 0.13, "#3A3A40", 1.0)}
        {text(o.x, top + bar / 2.0 + o.size * 0.3, o.title, MONO, o.size * 0.82, 500, c::TERTIARY, "middle")}
        <g clip-path={clip_url}>{rows}</g>
        {stroke_rect(left, top, o.w, o.h, 22.0, "#FFFFFF", 1.5, 0.09)}
        {stroke_rect(left, top, o.w, o.h, 22.0, c::DANGER, 3.0, glow)}
    </g>)
}
