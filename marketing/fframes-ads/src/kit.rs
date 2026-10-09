//! Shared look and motion: the Reins Dark palette, Geist, springs, text measuring and icons.
//!
//! Everything is drawn in "design units" around the centre of the frame. One unit is one pixel on
//! the 1080 px short side, so the same numbers work for 16:9 and 9:16; `Format` picks per-format
//! values where the layouts differ.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use fframes::{
    FFramesContext, FontQuery, Frame, Svgr,
    animation::{AnimationRuntime, Easing},
};

// ---------------------------------------------------------------- formats

pub trait Format: Send + Sync + 'static {
    const W: usize;
    const H: usize;
    const TALL: bool;
}

/// 1920x1080: X, LinkedIn, YouTube.
pub struct Wide;
/// 1080x1920: Reels, Shorts, TikTok.
pub struct Tall;

impl Format for Wide {
    const W: usize = 1920;
    const H: usize = 1080;
    const TALL: bool = false;
}

impl Format for Tall {
    const W: usize = 1080;
    const H: usize = 1920;
    const TALL: bool = true;
}

/// The wide or the tall value, for the format being rendered.
pub fn pick<F: Format, T>(wide: T, tall: T) -> T {
    if F::TALL {
        tall
    } else {
        wide
    }
}

// ---------------------------------------------------------------- palette and type

/// Reins Dark, from android/.../design/Palette.kt, plus the logo's violets.
pub mod c {
    pub const BG: &str = "#060606";
    pub const ELEVATED: &str = "#111113";
    pub const RAISED: &str = "#18181B";
    pub const CONTROL: &str = "#1E1E22";
    pub const TEXT: &str = "#E8E8EA";
    pub const SECONDARY: &str = "#A9A9AE";
    pub const TERTIARY: &str = "#6B6B72";
    pub const HAIRLINE: &str = "#26262B";
    pub const ACCENT: &str = "#8B7CF6";
    pub const LILAC: &str = "#C9C1FB";
    pub const DEEP: &str = "#170B33";
    pub const DANGER: &str = "#F87171";
    pub const DANGER_DEEP: &str = "#EF4444";
    pub const SUCCESS: &str = "#34D399";
    pub const WARNING: &str = "#FACC15";
    pub const SEARCH: &str = "#60A5FA";
    pub const SEND: &str = "#FB923C";
    pub const PAIR: &str = "#22D3EE";
}

pub const SANS: &str = "Geist";
pub const MONO: &str = "Geist Mono";

// ---------------------------------------------------------------- motion

/// Spring presets. Every movement in the ads is one of these or an ease below.
#[derive(Clone, Copy)]
pub enum M {
    /// UI-like, tiny overshoot.
    Snappy,
    /// Noticeable and friendly.
    Soft,
    /// Playful; used for pops and taps.
    Bouncy,
    /// Fast with a clear overshoot: words slamming in.
    Pop,
    /// Big, weighty objects: the phone, the terminal.
    Heavy,
}

static SPRINGS: LazyLock<[AnimationRuntime; 5]> = LazyLock::new(|| {
    let s = |mass, stiffness, damping| {
        AnimationRuntime::new(
            4.0,
            &Easing::Spring {
                mass,
                stiffness,
                damping,
            },
        )
    };
    [s(1.0, 300.0, 26.0), s(1.0, 150.0, 18.0), s(1.0, 220.0, 12.0), s(1.0, 420.0, 21.0), s(1.4, 190.0, 28.0)]
});

/// 0 before `at`, then a spring towards 1 (overshooting for the bouncier presets).
pub fn sp(t: f32, at: f32, m: M) -> f32 {
    let rt = &SPRINGS[m as usize];
    let dt = t - at;
    if dt <= 0.0 {
        0.0
    } else if dt >= rt.get_duration() {
        1.0
    } else {
        rt.solve(&dt)
    }
}

#[derive(Clone, Copy)]
pub enum E {
    /// Fast, then a long settle: slides.
    OutExpo,
    /// Slow start: exits.
    In,
    /// Camera moves and morphs.
    InOut,
    Linear,
}

static EASES: LazyLock<[AnimationRuntime; 4]> = LazyLock::new(|| {
    [
        AnimationRuntime::new(1.0, &Easing::CubicBezier(0.16, 1.0, 0.3, 1.0)),
        AnimationRuntime::new(1.0, &Easing::CubicBezier(0.55, 0.0, 1.0, 0.45)),
        AnimationRuntime::new(1.0, &Easing::CubicBezier(0.65, 0.0, 0.35, 1.0)),
        AnimationRuntime::new(1.0, &Easing::Linear),
    ]
});

/// 0 before `at`, 1 after `at + dur`, eased in between.
pub fn ease(t: f32, at: f32, dur: f32, e: E) -> f32 {
    let p = (t - at) / dur;
    if p <= 0.0 {
        0.0
    } else if p >= 1.0 {
        1.0
    } else {
        EASES[e as usize].solve(&p)
    }
}

/// In with a spring at `enter`, out quickly at `exit`.
pub fn life(t: f32, enter: f32, exit: f32, m: M) -> f32 {
    (sp(t, enter, m) - ease(t, exit, 0.25, E::In)).max(0.0)
}

pub fn lerp(a: f32, b: f32, p: f32) -> f32 {
    a + (b - a) * p
}

pub fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// A decaying shake for impacts: offsets in units, a fresh direction every frame.
pub fn shake(t: f32, hits: &[(f32, f32)]) -> (f32, f32) {
    let mut x = 0.0;
    let mut y = 0.0;
    for &(at, amp) in hits {
        let dt = t - at;
        if !(0.0..0.45).contains(&dt) {
            continue;
        }
        let k = amp * (-dt * 9.0).exp();
        let n = (t * 60.0).floor();
        x += (hash(n * 1.7 + at) - 0.5) * 2.0 * k;
        y += (hash(n * 3.1 + at * 2.0) - 0.5) * 2.0 * k;
    }
    (x, y)
}

/// Deterministic pseudo random number in 0..1.
pub fn hash(x: f32) -> f32 {
    let s = (x * 12.9898).sin() * 43758.547;
    s - s.floor()
}

// ---------------------------------------------------------------- text

type WidthKey = (&'static str, u16, String);
static WIDTHS: LazyLock<Mutex<HashMap<WidthKey, f32>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Width of `text` at `size` px, measured with the real font at 100 px once and cached.
pub fn measure(
    frame: &mut Frame,
    ctx: &FFramesContext,
    family: &'static str,
    weight: u16,
    size: f32,
    text: &str,
) -> f32 {
    let key = (family, weight, text.to_owned());
    if let Some(w) = WIDTHS.lock().unwrap().get(&key) {
        return w * size / 100.0;
    }
    let query = FontQuery {
        family,
        size: 100,
        weight,
        ..Default::default()
    };
    // `text_width` wants the text to live as long as the context; each distinct string is
    // leaked at most once because the result is cached.
    let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
    match frame.text_width(ctx, query, leaked) {
        Some(w) => {
            WIDTHS.lock().unwrap().insert(key, w as f32);
            w as f32 * size / 100.0
        }
        None => {
            text.chars().count() as f32
                * size
                * if family == MONO {
                    0.6
                } else {
                    0.55
                }
        }
    }
}

/// One line of text. `anchor` is start, middle or end.
#[allow(clippy::too_many_arguments)]
pub fn text(
    x: f32,
    y: f32,
    s: impl Into<String>,
    family: &'static str,
    size: f32,
    weight: u16,
    fill: &str,
    anchor: &'static str,
) -> Svgr<'static> {
    let s: String = s.into();
    fframes::svgr!(
        <text x={x} y={y} font-family={family} font-size={size} font-weight={weight.to_string()} fill={fill.to_owned()} text-anchor={anchor}>{s}</text>
    )
}

/// Like `text`, with letter spacing (negative for big titles, positive for small caps labels).
#[allow(clippy::too_many_arguments)]
pub fn text_ls(
    x: f32,
    y: f32,
    s: impl Into<String>,
    family: &'static str,
    size: f32,
    weight: u16,
    fill: &str,
    anchor: &'static str,
    spacing: f32,
) -> Svgr<'static> {
    let s: String = s.into();
    fframes::svgr!(
        <text x={x} y={y} font-family={family} font-size={size} font-weight={weight.to_string()} fill={fill.to_owned()} text-anchor={anchor} letter-spacing={spacing.to_string()}>{s}</text>
    )
}

// ---------------------------------------------------------------- shapes

pub fn tr(x: f32, y: f32) -> String {
    format!("translate({x:.2} {y:.2})")
}

pub fn trs(x: f32, y: f32, s: f32) -> String {
    format!("translate({x:.2} {y:.2}) scale({:.4})", s.max(0.0001))
}

pub fn rect(x: f32, y: f32, w: f32, h: f32, r: f32, fill: &str) -> Svgr<'static> {
    fframes::svgr!(<rect x={x} y={y} width={w.max(0.5)} height={h.max(0.5)} rx={r} fill={fill.to_owned()} />)
}

pub fn rect_o(x: f32, y: f32, w: f32, h: f32, r: f32, fill: &str, opacity: f32) -> Svgr<'static> {
    fframes::svgr!(<rect x={x} y={y} width={w.max(0.5)} height={h.max(0.5)} rx={r} fill={fill.to_owned()} opacity={clamp01(opacity)} />)
}

#[allow(clippy::too_many_arguments)]
pub fn stroke_rect(x: f32, y: f32, w: f32, h: f32, r: f32, color: &str, width: f32, opacity: f32) -> Svgr<'static> {
    fframes::svgr!(<rect x={x} y={y} width={w.max(0.5)} height={h.max(0.5)} rx={r} fill="none" stroke={color.to_owned()} stroke-width={width} opacity={clamp01(opacity)} />)
}

pub fn circle(cx: f32, cy: f32, r: f32, fill: &str, opacity: f32) -> Svgr<'static> {
    fframes::svgr!(<circle cx={cx} cy={cy} r={r.max(0.5)} fill={fill.to_owned()} opacity={clamp01(opacity)} />)
}

/// Wraps children in a group with a transform and an opacity; skips them when invisible.
pub fn group(transform: String, opacity: f32, children: Vec<Svgr<'static>>) -> Svgr<'static> {
    if opacity <= 0.002 {
        return Svgr::empty();
    }
    fframes::svgr!(<g transform={transform} opacity={clamp01(opacity)}>{children}</g>)
}

// ---------------------------------------------------------------- icons

/// Line icons on a 24 px grid (Lucide geometry), matching the app's outlined set.
#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Mail,
    Terminal,
    Key,
    Lock,
    Send,
    Branch,
    Check,
    X,
    Alert,
    Laptop,
    Shield,
    Cart,
    Card,
    Trash,
    Pin,
    FaceId,
    Pencil,
    Bot,
    Merge,
    Sparkles,
}

impl Icon {
    pub fn d(self) -> &'static str {
        match self {
            Icon::Mail => {
                "M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M22 7l-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"
            }
            Icon::Terminal => "M4 17l6-6-6-6 M12 19h8",
            Icon::Key => "M13 15.5a5.5 5.5 0 1 1-11 0a5.5 5.5 0 1 1 11 0z M21 2l-9.6 9.6 M15.5 7.5l3 3L22 7l-3-3",
            Icon::Lock => {
                "M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z M7 11V7a5 5 0 0 1 10 0v4"
            }
            Icon::Send => "M22 2l-7 20-4-9-9-4z M22 2L11 13",
            Icon::Branch => {
                "M6 3v12 M18 9a3 3 0 1 0 0-6a3 3 0 1 0 0 6z M6 21a3 3 0 1 0 0-6a3 3 0 1 0 0 6z M18 9a9 9 0 0 1-9 9"
            }
            Icon::Check => "M20 6L9 17l-5-5",
            Icon::X => "M18 6L6 18 M6 6l12 12",
            Icon::Alert => {
                "M21.73 18l-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3z M12 9v4 M12 17h.01"
            }
            Icon::Laptop => "M5 4h14a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M2 20h20",
            Icon::Shield => "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10 M9 12l2 2 4-4",
            Icon::Cart => {
                "M9 21a1 1 0 1 1-2 0a1 1 0 1 1 2 0z M20 21a1 1 0 1 1-2 0a1 1 0 1 1 2 0z M2.05 2.05h2l2.66 12.42a2 2 0 0 0 2 1.58h9.78a2 2 0 0 0 1.95-1.57l1.65-7.43H5.12"
            }
            Icon::Card => "M4 5h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z M2 10h20",
            Icon::Trash => "M3 6h18 M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6 M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2",
            Icon::Pin => {
                "M20 10c0 5-5.5 10.2-7.4 11.8a1 1 0 0 1-1.2 0C9.5 20.2 4 15 4 10a8 8 0 0 1 16 0 M15 10a3 3 0 1 1-6 0a3 3 0 1 1 6 0z"
            }
            Icon::FaceId => {
                "M3 7V5a2 2 0 0 1 2-2h2 M17 3h2a2 2 0 0 1 2 2v2 M21 17v2a2 2 0 0 1-2 2h-2 M7 21H5a2 2 0 0 1-2-2v-2 M8 14s1.5 2 4 2 4-2 4-2 M9 9h.01 M15 9h.01"
            }
            Icon::Pencil => "M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5z",
            Icon::Bot => {
                "M12 8V4H8 M6 8h12a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2z M2 14h2 M20 14h2 M15 13v2 M9 13v2"
            }
            Icon::Merge => {
                "M18 21a3 3 0 1 0 0-6a3 3 0 1 0 0 6z M6 9a3 3 0 1 0 0-6a3 3 0 1 0 0 6z M6 21V9a9 9 0 0 0 9 9"
            }
            Icon::Sparkles => {
                "M9.94 15.5A2 2 0 0 0 8.5 14.06l-6.13-1.58a.5.5 0 0 1 0-.96L8.5 9.94A2 2 0 0 0 9.94 8.5l1.58-6.13a.5.5 0 0 1 .96 0L14.06 8.5A2 2 0 0 0 15.5 9.94l6.13 1.58a.5.5 0 0 1 0 .96L15.5 14.06a2 2 0 0 0-1.44 1.44l-1.58 6.13a.5.5 0 0 1-.96 0z"
            }
        }
    }
}

/// An icon centred on (x, y), `size` px wide.
pub fn icon(i: Icon, x: f32, y: f32, size: f32, color: &str, stroke: f32) -> Svgr<'static> {
    let s = size / 24.0;
    let transform = format!("translate({:.2} {:.2}) scale({s:.4})", x - size / 2.0, y - size / 2.0);
    fframes::svgr!(
        <path transform={transform} d={i.d()} fill="none" stroke={color.to_owned()} stroke-width={stroke}
            stroke-linecap="round" stroke-linejoin="round" />
    )
}

/// An icon on a rounded tile tinted with its colour.
pub fn icon_tile(i: Icon, x: f32, y: f32, tile: f32, color: &str) -> Svgr<'static> {
    fframes::svgr!(<g>
        {rect_o(x - tile / 2.0, y - tile / 2.0, tile, tile, tile * 0.3, color, 0.15)}
        {icon(i, x, y, tile * 0.5, color, 2.0)}
    </g>)
}
