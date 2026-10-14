//! Hold to confirm, as on the phone: for what cannot be undone with a click ("Until I resume", disconnecting this
//! computer). Pressing starts a fill that sweeps across in [`HOLD`] with a small ring and "Keep holding"; letting go
//! early (or Esc) cancels; holding to the end does it.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, Context, Div, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, SharedString, Stateful, Styled as _, div, px, relative,
};

use super::Root;
use super::icons::{Icon, icon};
use super::parts::ring;
use crate::model::Model;
use crate::pause::PauseFor;
use crate::theme::Palette;

/// How long a hold takes.
pub const HOLD: Duration = Duration::from_millis(2000);

/// What a hold does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// Pause git until the user resumes.
    PauseUntilResume,
    /// Disconnect this computer (forget the session, pair again).
    Disconnect,
}

/// A hold in progress: what, and which press (a new press starts the fill again).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hold {
    pub what: Held,
    pub press: u64,
}

impl Root {
    pub(super) fn hold_start(&mut self, what: Held, cx: &mut Context<'_, Self>) {
        self.presses += 1;
        let press = self.presses;
        self.hold = Some(Hold {
            what,
            press,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOLD).await;
            let _gone = this.update(cx, |this, cx| {
                if this.hold.is_some_and(|h| h.press == press) {
                    this.hold = None;
                    this.held(what, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Let go: a hold that did not reach the end does nothing.
    pub(super) fn hold_cancel(&mut self, cx: &mut Context<'_, Self>) {
        if self.hold.take().is_some() {
            cx.notify();
        }
    }

    fn held(&mut self, what: Held, cx: &mut Context<'_, Self>) {
        match what {
            Held::PauseUntilResume => {
                self.pause_sheet = false;
                self.model.update(cx, |m, cx| m.pause_for(PauseFor::Manual, cx));
            }
            Held::Disconnect => self.model.update(cx, Model::sign_out),
        }
        cx.notify();
    }

    /// The press being held for `what`, if one is.
    pub(super) fn holding(&self, what: Held) -> Option<u64> {
        self.hold.filter(|h| h.what == what).map(|h| h.press)
    }

    /// Makes `area` the place to hold for `what`.
    pub(super) fn hold_area(area: Stateful<Div>, what: Held, cx: &Context<'_, Self>) -> Stateful<Div> {
        area.cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| this.hold_start(what, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.hold_cancel(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.hold_cancel(cx)))
    }

    /// The red button held to confirm: `label` with `idle` before it; while held, the fill and "Keep holding".
    pub(super) fn hold_button(
        &self,
        id: impl Into<ElementId>,
        what: Held,
        idle: Icon,
        label: impl Into<SharedString>,
        pal: Palette,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let fg = pal.on_danger();
        let b = div()
            .id(id.into())
            .relative()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .h(px(38.0))
            .rounded(px(11.0))
            .bg(pal.danger)
            .text_color(fg)
            .text_size(px(13.0))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap();
        let b = Self::hold_area(b, what, cx);
        match self.holding(what) {
            Some(press) => b.child(fill(press, gpui::black().opacity(0.22))).child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .child(progress_ring(press, fg, 16.0))
                    .child("Keep holding"),
            ),
            None => b.child(div().flex().items_center().gap(px(7.0)).child(icon(idle, 14.0, fg)).child(label.into())),
        }
    }
}

/// The fill that sweeps across while a press is held.
pub fn fill(press: u64, color: gpui::Hsla) -> impl IntoElement {
    div().absolute().left_0().top_0().bottom_0().w(px(0.0)).bg(color).with_animation(
        SharedString::from(format!("hold-fill-{press}")),
        Animation::new(HOLD),
        |el, t| el.w(relative(t)),
    )
}

/// The small ring that fills while a press is held.
pub fn progress_ring(press: u64, color: gpui::Hsla, side: f32) -> impl IntoElement {
    div().size(px(side)).flex_none().with_animation(
        SharedString::from(format!("hold-ring-{press}")),
        Animation::new(HOLD),
        move |el, t| el.child(ring(t, color, color.opacity(0.25), side, 2.0)),
    )
}
