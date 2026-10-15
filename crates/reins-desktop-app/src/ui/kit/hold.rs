//! Hold to confirm, for what can't be undone: the fill runs linearly over [`motion::HOLD`] (1200 ms) while pressed;
//! letting go early drains it in [`motion::HOLD_DRAIN`]. The fill is worked out from the time elapsed at each frame,
//! so Reduce Motion never shortens or skips it.

use std::time::Instant;

use gpui::{
    AnyElement, Div, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Stateful,
    Styled as _, div, px, relative,
};

use crate::theme::{Palette, SANS, motion};
use crate::ui::icons::{Glyph, glyph};
use crate::ui::parts::ring;

/// A press on a hold button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Held since then.
    Holding(Instant),
    /// Let go early with the fill at `from` (per mille), draining since then.
    Draining {
        from: u16,
        since: Instant,
    },
}

impl Press {
    /// How full, 0…1, at `now`; `None` once a drain has emptied.
    #[must_use]
    pub fn progress(self, now: Instant) -> Option<f32> {
        match self {
            Self::Holding(since) => {
                Some((now.saturating_duration_since(since).as_secs_f32() / motion::HOLD.as_secs_f32()).min(1.0))
            }
            Self::Draining {
                from,
                since,
            } => {
                let gone = now.saturating_duration_since(since).as_secs_f32() / motion::HOLD_DRAIN.as_secs_f32();
                (gone < 1.0).then(|| f32::from(from) / 1000.0 * (1.0 - gone))
            }
        }
    }

    /// Let go at `now`: the fill drains from where it got to.
    #[must_use]
    pub fn release(self, now: Instant) -> Self {
        match self {
            Self::Holding(_) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "0…1000")]
                let from = (self.progress(now).unwrap_or(0.0) * 1000.0).round() as u16;
                Self::Draining {
                    from,
                    since: now,
                }
            }
            drain @ Self::Draining {
                ..
            } => drain,
        }
    }

    #[must_use]
    pub fn holding(self) -> bool {
        matches!(self, Self::Holding(_))
    }
}

/// The fill that sweeps across, `progress` of the width.
pub fn fill(progress: f32, color: Hsla) -> impl IntoElement {
    div().absolute().left_0().top_0().bottom_0().w(relative(progress.clamp(0.0, 1.0))).bg(color)
}

/// The small ring that fills with it.
pub fn progress_ring(progress: f32, color: Hsla, track: Hsla, side: f32) -> impl IntoElement {
    ring(progress, color, track, side, 2.0)
}

/// What a hold button shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HoldLook {
    /// "Hold to delete", waiting for a press.
    Idle,
    /// Pressed (or draining), this full.
    Filling(f32),
    /// Done: solid red with a check ("Deleted").
    Done,
}

/// The red hold button on paper: `danger` on `danger-wash`, `danger-fill-2` filling while held, "Keep holding" with
/// a ring; solid red once done. `height` 54 on the phone, 38 on the desktop.
pub fn hold_button(
    id: impl Into<ElementId>,
    lead: Glyph,
    label: impl Into<SharedString>,
    done_label: impl Into<SharedString>,
    look: HoldLook,
    height: f32,
    pal: Palette,
) -> Stateful<Div> {
    let text = if height >= 50.0 {
        16.5
    } else {
        14.0
    };
    let b = div()
        .id(id.into())
        .relative()
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .h(px(height))
        .rounded_full()
        .font_family(SANS)
        .text_size(px(text))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .whitespace_nowrap();
    let line = |mark: AnyElement, words: SharedString, color: Hsla| {
        div().relative().flex().items_center().gap(px(8.0)).text_color(color).child(mark).child(words)
    };
    match look {
        HoldLook::Idle => b.bg(pal.danger_wash()).cursor_pointer().child(line(
            glyph(lead, 16.0, pal.danger).into_any_element(),
            label.into(),
            pal.danger,
        )),
        HoldLook::Filling(p) => {
            b.bg(pal.danger_wash()).cursor_pointer().child(fill(p, pal.danger_fill_2())).child(line(
                progress_ring(p, pal.danger, pal.danger_fill_2(), 16.0).into_any_element(),
                "Keep holding".into(),
                pal.danger,
            ))
        }
        HoldLook::Done => b.bg(crate::theme::Field::Delete.field()).child(line(
            glyph(Glyph::Check, 16.0, pal.on_field()).into_any_element(),
            done_label.into(),
            pal.on_field(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_hold_takes_1200_ms_and_drains_in_180() {
        let t0 = Instant::now();
        let press = Press::Holding(t0);
        assert_eq!(press.progress(t0), Some(0.0));
        let half = press.progress(t0 + Duration::from_millis(600)).unwrap();
        assert!((half - 0.5).abs() < 1e-3);
        assert_eq!(press.progress(t0 + Duration::from_millis(1200)), Some(1.0));
        assert_eq!(press.progress(t0 + Duration::from_secs(9)), Some(1.0));
        // Let go at 600 ms: it drains from half to nothing in 180 ms.
        let let_go = t0 + Duration::from_millis(600);
        let drain = press.release(let_go);
        assert!(!drain.holding());
        assert!((drain.progress(let_go).unwrap() - 0.5).abs() < 1e-3);
        assert!((drain.progress(let_go + Duration::from_millis(90)).unwrap() - 0.25).abs() < 1e-2);
        assert_eq!(drain.progress(let_go + Duration::from_millis(180)), None);
        assert_eq!(motion::HOLD, Duration::from_millis(1200));
    }
}
