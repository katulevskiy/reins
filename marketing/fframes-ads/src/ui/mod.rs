//! Pieces every film shares: the backdrop, kinetic headlines, the Reins mark, pills and the end card.

pub mod phone;
pub mod terminal;

use fframes::{FFramesContext, Frame, Svgr};

use crate::kit::*;

// ---------------------------------------------------------------- backdrop

/// Near-black with two slow violet and blue glows; `danger` (0..1) floods it red.
pub fn backdrop(t: f32, w: f32, h: f32, danger: f32, calm: f32) -> Svgr<'static> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let ax = -hw * 0.45 + (t * 0.21).sin() * 90.0;
    let ay = -hh * 0.4 + (t * 0.17).cos() * 70.0;
    let bx = hw * 0.4 + (t * 0.13).cos() * 110.0;
    let by = hh * 0.45 + (t * 0.19).sin() * 80.0;
    let r = w.max(h) * 0.62;
    let violet = 0.55 * (1.0 - danger) + 0.25 * calm;
    fframes::svgr!(<g>
        <defs>
            <radialGradient id="glow-violet">
                <stop offset="0" stop-color="#4B3BB8" stop-opacity="0.55" />
                <stop offset="1" stop-color="#4B3BB8" stop-opacity="0" />
            </radialGradient>
            <radialGradient id="glow-blue">
                <stop offset="0" stop-color="#1D3A6E" stop-opacity="0.5" />
                <stop offset="1" stop-color="#1D3A6E" stop-opacity="0" />
            </radialGradient>
            <radialGradient id="glow-red">
                <stop offset="0" stop-color="#B91C1C" stop-opacity="0.6" />
                <stop offset="1" stop-color="#B91C1C" stop-opacity="0" />
            </radialGradient>
            <radialGradient id="vignette">
                <stop offset="0.55" stop-color="#000" stop-opacity="0" />
                <stop offset="1" stop-color="#000" stop-opacity="0.75" />
            </radialGradient>
        </defs>
        <rect x={-hw} y={-hh} width={w} height={h} fill={c::BG} />
        <circle cx={ax} cy={ay} r={r} fill="url(#glow-violet)" opacity={clamp01(violet)} />
        <circle cx={bx} cy={by} r={r * 0.9} fill="url(#glow-blue)" opacity={clamp01(0.7 * (1.0 - danger))} />
        <circle cx="0" cy="0" r={r * 1.1} fill="url(#glow-red)" opacity={clamp01(danger * 0.8)} />
        {dots(w, h)}
        <rect x={-hw} y={-hh} width={w} height={h} fill="url(#vignette)" />
    </g>)
}

/// A faint dot grid: texture that keeps big empty areas from looking flat.
fn dots(w: f32, h: f32) -> Svgr<'static> {
    let step = 54.0;
    let mut d = String::new();
    let (nx, ny) = ((w / step) as i32 / 2 + 1, (h / step) as i32 / 2 + 1);
    for i in -nx..=nx {
        for j in -ny..=ny {
            d.push_str(&format!("M{} {}h.01", i as f32 * step, j as f32 * step));
        }
    }
    fframes::svgr!(<path d={d} stroke="#FFFFFF" stroke-opacity="0.07" stroke-width="2.6" stroke-linecap="round" />)
}

/// A full-frame flash (impacts, the logo), fading over `len` seconds.
pub fn flash(t: f32, at: f32, len: f32, color: &str, peak: f32, w: f32, h: f32) -> Svgr<'static> {
    let dt = t - at;
    if !(0.0..len).contains(&dt) {
        return Svgr::empty();
    }
    let o = peak * (1.0 - dt / len).powf(2.0);
    rect_o(-w / 2.0, -h / 2.0, w, h, 0.0, color, o)
}

// ---------------------------------------------------------------- kinetic words

#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Text,
    Accent,
    Danger,
    Success,
    Dim,
}

impl Tone {
    pub fn color(self) -> &'static str {
        match self {
            Tone::Text => c::TEXT,
            Tone::Accent => c::LILAC,
            Tone::Danger => c::DANGER,
            Tone::Success => c::SUCCESS,
            Tone::Dim => c::SECONDARY,
        }
    }
}

/// A word and the second it lands (usually on the voiceover's word). "/" breaks the line.
pub type Word = (&'static str, f32, Tone);

pub struct Words<'w> {
    pub words: &'w [Word],
    pub size: f32,
    pub weight: u16,
    pub x: f32,
    pub y: f32,
    /// "start" or "middle".
    pub align: &'static str,
    pub max_width: f32,
    /// Leaves with a quick fade upwards from this second.
    pub exit: f32,
    pub spacing: f32,
}

impl Default for Words<'_> {
    fn default() -> Self {
        Words {
            words: &[],
            size: 96.0,
            weight: 700,
            x: 0.0,
            y: 0.0,
            align: "middle",
            max_width: 1500.0,
            exit: f32::MAX,
            spacing: -2.0,
        }
    }
}

/// Lays the words out in lines (measured with the real font) and springs each one in on its beat.
/// `y` is the baseline of the first line. Returns the drawing and the height of the block.
pub fn words(frame: &mut Frame, ctx: &FFramesContext, t: f32, o: &Words) -> (Svgr<'static>, f32) {
    let space = measure(frame, ctx, SANS, o.weight, o.size, " ") + o.spacing;
    let line_h = o.size * 1.12;
    // Greedy wrap.
    let mut lines: Vec<Vec<(usize, f32)>> = vec![vec![]];
    let mut line_w = 0.0;
    for (i, (w, _, _)) in o.words.iter().enumerate() {
        if *w == "/" {
            lines.push(vec![]);
            line_w = 0.0;
            continue;
        }
        let ww = measure(frame, ctx, SANS, o.weight, o.size, w) + o.spacing * (w.chars().count() as f32 - 1.0).max(0.0);
        let cur = lines.last_mut().unwrap();
        let next = if cur.is_empty() {
            ww
        } else {
            line_w + space + ww
        };
        if next > o.max_width && !cur.is_empty() {
            lines.push(vec![(i, ww)]);
            line_w = ww;
        } else {
            cur.push((i, ww));
            line_w = next;
        }
    }
    lines.retain(|l| !l.is_empty());
    let out = ease(t, o.exit, 0.28, E::In);
    let mut parts = vec![];
    for (li, line) in lines.iter().enumerate() {
        let total: f32 = line.iter().map(|(_, w)| w).sum::<f32>() + space * (line.len() as f32 - 1.0);
        let mut x = if o.align == "middle" {
            o.x - total / 2.0
        } else {
            o.x
        };
        let y = o.y + li as f32 * line_h;
        for &(i, ww) in line {
            let (w, at, tone) = o.words[i];
            let p = sp(t, at, M::Pop);
            if p > 0.001 {
                let rise = (1.0 - p) * o.size * 0.42 - out * o.size * 0.3;
                let s = lerp(0.9, 1.0, p);
                let cx = x + ww / 2.0;
                let transform =
                    format!("translate({:.2} {:.2}) scale({s:.4}) translate({:.2} 0)", cx, y + rise, -ww / 2.0);
                let opacity = clamp01(p * 1.6) * (1.0 - out);
                parts.push(group(
                    transform,
                    opacity,
                    vec![text_ls(0.0, 0.0, w, SANS, o.size, o.weight, tone.color(), "start", o.spacing)],
                ));
            }
            x += ww + space;
        }
    }
    (fframes::svgr!(<g>{parts}</g>), lines.len() as f32 * line_h)
}

// ---------------------------------------------------------------- the Reins mark

/// The app icon: a violet tile with a shield and a check. `p` builds it (0..1, springy),
/// `draw` draws the shield and the check on.
pub fn mark(x: f32, y: f32, size: f32, p: f32, draw: f32) -> Svgr<'static> {
    if p <= 0.001 {
        return Svgr::empty();
    }
    let s = size / 108.0 * p;
    let shield = 56.0;
    let check = 12.0;
    let d1 = format!("{:.2} 200", shield * clamp01(draw));
    let d2 = format!("{:.2} 200", check * clamp01((draw - 0.55) / 0.45));
    let transform = format!("translate({x:.2} {y:.2}) scale({s:.4}) translate(-54 -54)");
    fframes::svgr!(<g transform={transform} opacity={clamp01(p * 2.0)}>
        <rect width="108" height="108" rx="24" fill={c::DEEP} />
        <rect width="108" height="108" rx="24" fill="none" stroke="#FFFFFF" stroke-opacity="0.08" stroke-width="1.2" />
        <g transform="translate(27 27) scale(2.25)" fill="none" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12,3l7.5,3v5.5c0,4.6 -3.1,8.4 -7.5,9.5 -4.4,-1.1 -7.5,-4.9 -7.5,-9.5V6z" stroke={c::LILAC} stroke-width="1.5" stroke-dasharray={d1} />
            <path d="M8.7,12l2.4,2.4 4.2,-4.6" stroke="#FFFFFF" stroke-width="1.7" stroke-dasharray={d2} />
        </g>
    </g>)
}

/// Mark and "Reins" side by side, centred on (x, y).
pub fn lockup(frame: &mut Frame, ctx: &FFramesContext, t: f32, at: f32, x: f32, y: f32, size: f32) -> Svgr<'static> {
    let p = sp(t, at, M::Bouncy);
    let draw = ease(t, at + 0.05, 0.7, E::OutExpo);
    let word = sp(t, at + 0.18, M::Snappy);
    let fs = size * 0.78;
    let ww = measure(frame, ctx, SANS, 700, fs, "Reins") - 0.03 * fs * 4.0;
    let gap = size * 0.28;
    let total = size + gap + ww;
    let left = x - total / 2.0;
    let mx = lerp(x - size / 2.0, left + size / 2.0, clamp01(word));
    fframes::svgr!(<g>
        {mark(mx, y, size, p, draw)}
        {group(tr(left + size + gap + (1.0 - word) * -30.0, y + fs * 0.36), clamp01(word * 1.4), vec![
            text_ls(0.0, 0.0, "Reins", SANS, fs, 700, c::TEXT, "start", -0.03 * fs)
        ])}
    </g>)
}

// ---------------------------------------------------------------- pills

/// A rounded label with an optional icon, centred on (x, y).
#[allow(clippy::too_many_arguments)]
pub fn pill(
    frame: &mut Frame,
    ctx: &FFramesContext,
    x: f32,
    y: f32,
    label: &str,
    size: f32,
    fg: &str,
    bg: &str,
    bg_opacity: f32,
    ic: Option<Icon>,
    mono: bool,
) -> (Svgr<'static>, f32) {
    let family = if mono {
        MONO
    } else {
        SANS
    };
    let tw = measure(frame, ctx, family, 600, size, label);
    let h = size * 2.1;
    let pad = size * 0.9;
    let iw = if ic.is_some() {
        size * 1.25
    } else {
        0.0
    };
    let w = tw + pad * 2.0 + iw;
    let left = x - w / 2.0;
    let icon_svg = match ic {
        Some(i) => icon(i, left + pad + size * 0.5, y, size * 1.05, fg, 2.4),
        None => Svgr::empty(),
    };
    (
        fframes::svgr!(<g>
            {rect_o(left, y - h / 2.0, w, h, h / 2.0, bg, bg_opacity)}
            {stroke_rect(left, y - h / 2.0, w, h, h / 2.0, fg, 1.5, 0.22)}
            {icon_svg}
            {text(left + pad + iw, y + size * 0.36, label.to_owned(), family, size, 600, fg, "start")}
        </g>),
        w,
    )
}

// ---------------------------------------------------------------- end card

/// Mark, wordmark, a line under it and the address. Everything springs in from `at`.
pub fn end_card(
    frame: &mut Frame,
    ctx: &FFramesContext,
    t: f32,
    at: f32,
    y: f32,
    tall: bool,
    line: &'static str,
) -> Svgr<'static> {
    let size = if tall {
        190.0
    } else {
        180.0
    };
    let lock = lockup(frame, ctx, t, at, 0.0, y, size);
    let lp = sp(t, at + 0.45, M::Soft);
    let up = sp(t, at + 0.7, M::Bouncy);
    let line_size = if tall {
        64.0
    } else {
        58.0
    };
    let (url, _) = pill(
        frame,
        ctx,
        0.0,
        0.0,
        "reins2fa.com",
        if tall {
            52.0
        } else {
            46.0
        },
        c::TEXT,
        c::ACCENT,
        0.22,
        None,
        false,
    );
    fframes::svgr!(<g>
        {lock}
        {group(tr(0.0, y + size * 0.95 + (1.0 - lp) * 30.0), clamp01(lp * 1.5), vec![
            text(0.0, 0.0, line, SANS, line_size, 600, c::TEXT, "middle")
        ])}
        {group(trs(0.0, y + size * 0.95 + line_size * 2.3, lerp(0.85, 1.0, up)), clamp01(up * 1.5), vec![url])}
    </g>)
}
