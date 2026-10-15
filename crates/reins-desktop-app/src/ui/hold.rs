//! Hold to confirm, as on the phone: for what cannot be undone with a click ("Until I resume", disconnecting this
//! computer, shredding originals). Pressing starts a fill that sweeps across in [`HOLD`] (1200 ms) with a small ring
//! and "Keep holding"; letting go early (or Esc) cancels and the fill drains in 180 ms; holding to the end does it.
//! The fill follows the time held, frame by frame ([`kit::hold::Press`]), so Reduce Motion never shortens it.

use std::time::Instant;

use gpui::{
    Context, Div, ElementId, InteractiveElement as _, MouseButton, SharedString, Stateful, Styled as _, Window,
};

use super::icons::Glyph;
use super::{Root, kit};
use crate::model::Model;
use crate::pause::PauseFor;
use crate::theme::Palette;
pub use crate::theme::motion::HOLD;
use kit::hold::{HoldLook, Press};

/// What a hold does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// Pause git until the user resumes.
    PauseUntilResume,
    /// Disconnect this computer (forget the session, pair again).
    Disconnect,
    /// Delete for good the originals a secrets import sent to the trash.
    PurgeOriginals,
    /// The gallery's hold button (does nothing).
    Gallery,
}

/// A hold in progress or draining: what, which press (a new press starts the fill again), and how far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hold {
    pub what: Held,
    pub press: u64,
    pub state: Press,
}

impl Root {
    pub(super) fn hold_start(&mut self, what: Held, cx: &mut Context<'_, Self>) {
        self.presses += 1;
        let press = self.presses;
        self.hold = Some(Hold {
            what,
            press,
            state: Press::Holding(Instant::now()),
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOLD).await;
            let _gone = this.update(cx, |this, cx| {
                if this.hold.is_some_and(|h| h.press == press && h.state.holding()) {
                    this.hold = None;
                    this.held(what, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Let go: a hold that did not reach the end does nothing, and its fill drains.
    pub(super) fn hold_cancel(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(h) = &mut self.hold
            && h.state.holding()
        {
            h.state = h.state.release(Instant::now());
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
            Held::PurgeOriginals => self.model.update(cx, Model::purge_originals),
            Held::Gallery => self.gallery_held = true,
        }
        cx.notify();
    }

    /// How full the fill for `what` is now, while held or draining.
    pub(super) fn hold_progress(&self, what: Held) -> Option<f32> {
        self.hold.filter(|h| h.what == what).and_then(|h| h.state.progress(Instant::now()))
    }

    /// Draws the next frame while a fill moves; forgets a drain that has emptied.
    pub(super) fn hold_frame(&mut self, window: &Window) {
        if let Some(h) = self.hold {
            if h.state.progress(Instant::now()).is_some() {
                window.request_animation_frame();
            } else {
                self.hold = None;
            }
        }
    }

    /// Makes `area` the place to hold for `what`.
    pub(super) fn hold_area(area: Stateful<Div>, what: Held, cx: &Context<'_, Self>) -> Stateful<Div> {
        area.cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| this.hold_start(what, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.hold_cancel(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.hold_cancel(cx)))
    }

    /// The red button held to confirm (38 px): `label` with `idle` before it; while held, the fill and "Keep
    /// holding".
    #[allow(clippy::too_many_arguments, reason = "a button's parts")]
    pub(super) fn hold_button(
        &self,
        id: impl Into<ElementId>,
        what: Held,
        idle: Glyph,
        label: impl Into<SharedString>,
        done: impl Into<SharedString>,
        pal: Palette,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let look = self.hold_progress(what).map_or(HoldLook::Idle, HoldLook::Filling);
        let b = kit::hold_button(id, idle, label, done, look, 38.0, pal);
        Self::hold_area(b, what, cx)
    }
}
