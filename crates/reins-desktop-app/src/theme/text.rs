//! Next's type scale on the desktop (`design/next/desktop/project/DeskNext-*.dc.html`) as styles on any element:
//! `div().title().child("Activity")`. Two voices: Sora for everything you read (titles at 500, the boards' display
//! `.d`: a size of `--s` × 0.74, a line of 1.04), JetBrains Mono for what the machine says (branches, hashes, codes,
//! timers). Nothing is bold: the heaviest weight is 600.
//!
//! GPUI has no letter-spacing: the display's -0.03em and the section line's +0.015em are left out.

#![allow(dead_code, reason = "the whole scale is here; not every screen uses every style")]

use gpui::{Div, FontWeight, ParentElement as _, SharedString, Styled, div, px};

use super::{DISPLAY, MONO, SANS};

/// A style of the scale: family, size, line height, weight, italic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub family: &'static str,
    pub size: f32,
    pub line: f32,
    pub weight: FontWeight,
    pub italic: bool,
}

const fn style(family: &'static str, size: f32, line: f32, weight: FontWeight, italic: bool) -> TextStyle {
    TextStyle {
        family,
        size,
        line,
        weight,
        italic,
    }
}

/// A display line: `--s` of the boards, × 0.74, 500, a line of 1.04 (and a pixel, so descenders are not cut).
const fn display(s: f32) -> TextStyle {
    let size = s * 0.74;
    style(DISPLAY, size, size * 1.04 + 1.0, FontWeight::MEDIUM, false)
}

/// The running mode's clock (`--s: 100`).
pub const CLOCK: TextStyle = display(100.0);
/// The welcome's words (`--s: 64`).
pub const HERO: TextStyle = display(64.0);
/// The ask on a field across the window (`--s: 56`).
pub const ASK_XL: TextStyle = display(56.0);
/// "Reins is on" at the end of the setup (`--s: 48`).
pub const ASK: TextStyle = display(48.0);
/// Page titles (`--s: 46`).
pub const TITLE: TextStyle = display(46.0);
/// A step's title in the setup (`--s: 40`).
pub const STEP_TITLE: TextStyle = display(40.0);
/// A mode's name on its sheet (`--s: 36`).
pub const MODE_TITLE: TextStyle = display(36.0);
/// Numbers and states on Home's cards (`--s: 34`).
pub const STAT: TextStyle = display(34.0);
/// Sheet titles (`--s: 34`).
pub const SHEET_TITLE: TextStyle = display(34.0);
/// A panel's title, the ask on a card (`--s: 30`).
pub const ASK_SMALL: TextStyle = display(30.0);
/// The line under a page title (`.dsub`): 14.5 in `ink-2`.
pub const MOOD: TextStyle = style(SANS, 14.5, 20.0, FontWeight::NORMAL, false);
/// Buttons (`.btn.sm`): 15, 600.
pub const BUTTON: TextStyle = style(SANS, 15.0, 20.0, FontWeight::SEMIBOLD, false);
/// Row titles (`.win .rt`): 15, 500.
pub const ROW: TextStyle = style(SANS, 15.0, 20.0, FontWeight::MEDIUM, false);
/// Explanations: 14.5/21.
pub const BODY: TextStyle = style(SANS, 14.5, 21.0, FontWeight::NORMAL, false);
/// Row subtitles, hints (`.sub`): 13/17.
pub const META: TextStyle = style(SANS, 13.0, 17.0, FontWeight::NORMAL, false);
/// Tags and small pills (`.pill`): 12.5, 600.
pub const TAG: TextStyle = style(SANS, 12.5, 16.0, FontWeight::SEMIBOLD, false);
/// Section lines (`.sec`): 13, 600, in `ink-2`, as written (no capitals).
pub const LABEL: TextStyle = style(SANS, 13.0, 17.0, FontWeight::SEMIBOLD, false);
/// Branches, accounts, hashes, the timer: mono 13.
pub const VALUE: TextStyle = style(MONO, 13.0, 18.0, FontWeight::NORMAL, false);

/// The whole scale by name, for the gallery.
pub const SCALE: [(&str, TextStyle); 18] = [
    ("clock", CLOCK),
    ("hero", HERO),
    ("ask-xl", ASK_XL),
    ("ask", ASK),
    ("title", TITLE),
    ("step-title", STEP_TITLE),
    ("mode-title", MODE_TITLE),
    ("stat", STAT),
    ("sheet-title", SHEET_TITLE),
    ("ask-small", ASK_SMALL),
    ("mood", MOOD),
    ("button", BUTTON),
    ("row", ROW),
    ("body", BODY),
    ("meta", META),
    ("tag", TAG),
    ("label", LABEL),
    ("value", VALUE),
];

/// Next's type styles on any element.
pub trait NextText: Styled + Sized {
    /// Sets `s`: family, size, line height, weight, italic or upright.
    fn signal(self, s: TextStyle) -> Self {
        let el = self.font_family(s.family).text_size(px(s.size)).line_height(px(s.line)).font_weight(s.weight);
        if s.italic {
            el.italic()
        } else {
            el.not_italic()
        }
    }

    fn hero(self) -> Self {
        self.signal(HERO)
    }

    fn ask_xl(self) -> Self {
        self.signal(ASK_XL)
    }

    fn ask(self) -> Self {
        self.signal(ASK)
    }

    fn title(self) -> Self {
        self.signal(TITLE)
    }

    fn step_title(self) -> Self {
        self.signal(STEP_TITLE)
    }

    fn stat(self) -> Self {
        self.signal(STAT)
    }

    fn sheet_title(self) -> Self {
        self.signal(SHEET_TITLE)
    }

    fn ask_small(self) -> Self {
        self.signal(ASK_SMALL)
    }

    fn mood(self) -> Self {
        self.signal(MOOD)
    }

    fn button(self) -> Self {
        self.signal(BUTTON)
    }

    fn row(self) -> Self {
        self.signal(ROW)
    }

    fn body(self) -> Self {
        self.signal(BODY)
    }

    fn meta(self) -> Self {
        self.signal(META)
    }

    fn tag(self) -> Self {
        self.signal(TAG)
    }

    /// A section line's face.
    fn label(self) -> Self {
        self.signal(LABEL)
    }

    fn value(self) -> Self {
        self.signal(VALUE)
    }
}

impl<T: Styled> NextText for T {}

/// A section line (`.sec`): Sora 13 at 600, as written (the colour is the caller's).
pub fn tracked(text: impl Into<SharedString>) -> Div {
    tracked_at(text, LABEL)
}

/// A line of `text` in style `s`, on one line.
pub fn tracked_at(text: impl Into<SharedString>, s: TextStyle) -> Div {
    div().flex().flex_none().items_center().whitespace_nowrap().signal(s).child(text.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_bold_and_the_display_is_medium() {
        for (name, s) in SCALE {
            assert!(s.weight.0 <= 600.0, "{name}");
            if s.family == DISPLAY && s.size > 20.0 {
                assert_eq!(s.weight, FontWeight::MEDIUM, "{name}");
            }
            assert!(s.line >= s.size * 0.9, "{name}");
            assert!(!s.italic, "{name}: Next has no italic");
        }
        // The boards' `--s: 46` page title.
        assert!((TITLE.size - 34.04).abs() < 0.01);
        assert_eq!((LABEL.family, LABEL.size), (SANS, 13.0));
        assert_eq!(VALUE.family, MONO);
    }
}
