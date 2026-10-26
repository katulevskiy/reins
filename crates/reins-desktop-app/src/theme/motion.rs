//! Signal's motion (`screens/notes/Motion.md`, the `motion-time` tokens): the durations, the curves, and the two
//! accessibility settings read from the OS, Reduce Motion and Reduce Transparency.
//!
//! Under Reduce Motion, slides become 150 ms cross-fades and nothing breathes; holds keep their full length (they are
//! driven by elapsed time, not by an animation that could be skipped). The app keeps its own flag rather than GPUI's
//! `App::set_reduce_motion`: GPUI's skips every animation outright, the cross-fades included.

#![allow(dead_code, reason = "the motion tokens, as a set: not every one is used on the desktop")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{Animation, AnimationExt as _, ElementId, IntoElement, Styled, px};

/// Hold to delete, to pause until resumed, to disconnect.
pub const HOLD: Duration = Duration::from_millis(1200);
/// A released hold drains.
pub const HOLD_DRAIN: Duration = Duration::from_millis(180);
/// A field slides into place.
pub const FIELD_IN: Duration = Duration::from_millis(320);
/// The ask follows the field.
pub const ASK_DELAY: Duration = Duration::from_millis(60);
/// Colour and hover changes; and what a slide becomes under Reduce Motion.
pub const FADE: Duration = Duration::from_millis(150);
/// Moving between waiting asks cross-fades the field colour.
pub const KIND_CHANGE: Duration = Duration::from_millis(200);
/// An answered field folds into its kind tile.
pub const FOLD: Duration = Duration::from_millis(360);
/// A flicked card leaves.
pub const CARD_FLY: Duration = Duration::from_millis(220);
/// The tab pill slides.
pub const TAB_PILL: Duration = Duration::from_millis(240);
/// A sheet rises.
pub const SHEET_RISE: Duration = Duration::from_millis(280);
/// A pushed page slides in.
pub const DRILL_IN: Duration = Duration::from_millis(300);
/// A menu opens from its trigger…
pub const MENU_OPEN: Duration = Duration::from_millis(180);
/// …and closes.
pub const MENU_CLOSE: Duration = Duration::from_millis(120);
/// Numbers roll.
pub const NUMBERS: Duration = Duration::from_millis(240);
/// The bypass dot breathes, 35% to 100%.
pub const BREATHE: Duration = Duration::from_millis(1800);

/// The field's ease out: `cubic-bezier(0.2, 0.8, 0.2, 1)`.
pub const EASE_OUT: (f32, f32, f32, f32) = (0.2, 0.8, 0.2, 1.0);
/// The card spring: `cubic-bezier(0.32, 0.86, 0.32, 1)` (the board gives its first two numbers).
pub const SPRING: (f32, f32, f32, f32) = (0.32, 0.86, 0.32, 1.0);

/// A CSS `cubic-bezier(x1, y1, x2, y2)` easing: time in, progress out. GPUI wants progress within 0…1, so `y1` and
/// `y2` are kept there (the tab pill's overshoot is left out).
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 + Copy {
    let (y1, y2) = (y1.clamp(0.0, 1.0), y2.clamp(0.0, 1.0));
    // B(s) = 3(1-s)²s·p1 + 3(1-s)s²·p2 + s³, for x and for y.
    let at = |s: f32, p1: f32, p2: f32| {
        let r = 1.0 - s;
        (3.0 * r * r * s).mul_add(p1, (3.0 * r * s * s).mul_add(p2, s * s * s))
    };
    move |t: f32| {
        let t = t.clamp(0.0, 1.0);
        // x(s) is increasing for x1, x2 in 0…1: find s with x(s) = t by bisection (enough at 20 steps).
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..20 {
            let mid = f32::midpoint(lo, hi);
            if at(mid, x1, x2) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        at(f32::midpoint(lo, hi), y1, y2).clamp(0.0, 1.0)
    }
}

/// [`cubic_bezier`] of a tuple.
pub fn curve(c: (f32, f32, f32, f32)) -> impl Fn(f32) -> f32 + Copy {
    cubic_bezier(c.0, c.1, c.2, c.3)
}

static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);
static REDUCE_TRANSPARENCY: AtomicBool = AtomicBool::new(false);

/// Reduce Motion is on.
#[must_use]
pub fn reduce_motion() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed)
}

/// Reduce Transparency is on: no grain on fields.
#[must_use]
pub fn reduce_transparency() -> bool {
    REDUCE_TRANSPARENCY.load(Ordering::Relaxed)
}

/// Reads both settings from the OS, at start. `REINS_REDUCE_MOTION` and `REINS_REDUCE_TRANSPARENCY` (`1` or `0`)
/// say otherwise, for screenshots and tests.
pub fn init() {
    let env = |name: &str| std::env::var(name).ok().map(|v| !v.is_empty() && v != "0");
    let motion = env("REINS_REDUCE_MOTION").unwrap_or_else(os::reduce_motion);
    let transparency = env("REINS_REDUCE_TRANSPARENCY").unwrap_or_else(os::reduce_transparency);
    REDUCE_MOTION.store(motion, Ordering::Relaxed);
    REDUCE_TRANSPARENCY.store(transparency, Ordering::Relaxed);
    if motion || transparency {
        log::info!("reduce motion: {motion}, reduce transparency: {transparency}");
    }
}

/// How something enters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entrance {
    /// A field: slides down from above its place, [`FIELD_IN`], ease out.
    Field,
    /// A menu: fades in and slides 4 px down from its trigger, [`MENU_OPEN`] (GPUI cannot scale a div, so no grow).
    Menu,
    /// A sheet: rises 24 px and fades in, [`SHEET_RISE`].
    Sheet,
}

/// `el` entering as `how`; under Reduce Motion, a [`FADE`] cross-fade instead. `el` should be positioned (relative or
/// absolute): the slide moves its `top`.
pub fn enter<E: IntoElement + Styled + 'static>(
    el: E,
    id: impl Into<ElementId>,
    how: Entrance,
) -> gpui::AnimationElement<E> {
    if reduce_motion() {
        return el.with_animation(id, Animation::new(FADE), Styled::opacity);
    }
    let (duration, distance) = match how {
        Entrance::Field => (FIELD_IN, -24.0),
        Entrance::Menu => (MENU_OPEN, -4.0),
        Entrance::Sheet => (SHEET_RISE, 24.0),
    };
    el.with_animation(id, Animation::new(duration).with_easing(curve(EASE_OUT)), move |el, t| {
        el.top(px(distance * (1.0 - t))).opacity(t.mul_add(0.7, 0.3).min(1.0))
    })
}

/// A dot or chip that breathes 35% to 100% over [`BREATHE`] while something runs (a bypass, a request waiting);
/// still at full under Reduce Motion. GPUI only draws frames for a window that shows: a closed or hidden window does
/// not run it.
pub fn breathe<E: IntoElement + Styled + 'static>(el: E, id: impl Into<ElementId>) -> gpui::AnyElement {
    if reduce_motion() {
        return el.into_any_element();
    }
    el.with_animation(id, Animation::new(BREATHE).repeat(), |el, t| {
        // 100% → 35% → 100%, eased in and out.
        let wave = (t * std::f32::consts::TAU).cos().mul_add(0.5, 0.5);
        el.opacity(wave.mul_add(0.65, 0.35))
    })
    .into_any_element()
}

#[cfg(target_os = "linux")]
mod os {
    /// GNOME's "Animations" switch (`org.gnome.desktop.interface enable-animations`), which GTK apps follow too.
    pub fn reduce_motion() -> bool {
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
    }

    /// Linux desktops have no common "reduce transparency" setting: the grain stays on (`REINS_REDUCE_TRANSPARENCY`
    /// turns it off).
    pub fn reduce_transparency() -> bool {
        false
    }
}

#[cfg(target_os = "macos")]
mod os {
    use objc2_app_kit::NSWorkspace;

    pub fn reduce_motion() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }

    pub fn reduce_transparency() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
    }
}

#[cfg(windows)]
mod os {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    /// "Show animations in Windows" (`SPI_GETCLIENTAREAANIMATION`): the system keeps it in `UserPreferencesMask`,
    /// bit `(0x1042 − 0x1000) / 2 = 33`, byte 4's 0x02. (Reading it with `SystemParametersInfoW` needs `unsafe`,
    /// which this crate forbids.)
    pub fn reduce_motion() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Control Panel\Desktop")
            .and_then(|k| k.get_raw_value("UserPreferencesMask"))
            .ok()
            .and_then(|v| v.bytes.get(4).copied())
            .is_some_and(|byte| byte & 0x02 == 0)
    }

    /// "Transparency effects" off.
    pub fn reduce_transparency() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .and_then(|k| k.get_value::<u32, _>("EnableTransparency"))
            .is_ok_and(|v| v == 0)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod os {
    pub fn reduce_motion() -> bool {
        false
    }

    pub fn reduce_transparency() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hold_is_1200_ms_and_drains_in_180() {
        assert_eq!(HOLD, Duration::from_millis(1200));
        assert_eq!(HOLD_DRAIN, Duration::from_millis(180));
        assert_eq!(FADE, Duration::from_millis(150));
    }

    #[test]
    fn cubic_bezier_eases() {
        let ease = curve(EASE_OUT);
        assert!(ease(0.0).abs() < 1e-3);
        assert!((ease(1.0) - 1.0).abs() < 1e-3);
        // An ease out is well ahead at a quarter of the time, and never goes back.
        assert!(ease(0.25) > 0.5, "{}", ease(0.25));
        let mut last = 0.0;
        for i in 0..=100 {
            #[allow(clippy::cast_precision_loss, reason = "0…100")]
            let v = ease(i as f32 / 100.0);
            assert!(v + 1e-4 >= last && (0.0..=1.0).contains(&v));
            last = v;
        }
        // The straight line is linear.
        let linear = cubic_bezier(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0);
        assert!((linear(0.3) - 0.3).abs() < 1e-3);
    }
}
