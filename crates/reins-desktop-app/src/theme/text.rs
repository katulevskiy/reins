//! Signal's type scale (`tokens/signal.tokens.json`, `screens/png/Type.png`) as styles on any element:
//! `div().title().child("Activity")`. Three voices: the serif for what you decide on, the sans for what you read, mono
//! in capitals for what the machine says. Nothing is bold: the heaviest weight is 600, and the serif is always 400.
//!
//! GPUI has no letter-spacing: the serif's slight negative tracking is left out (a few hundredths of an em), and the
//! label's +0.16em is drawn by [`tracked`], one character at a time.

#![allow(dead_code, reason = "Signal's foundation: the whole set is here; the screens adopt it in phase 2")]

use gpui::{Div, FontWeight, ParentElement as _, SharedString, Styled, div, px};

use super::{MONO, SANS, SERIF};

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

/// Asks on the desktop: serif 64/62.
pub const ASK_XL: TextStyle = style(SERIF, 64.0, 62.0, FontWeight::NORMAL, false);
/// The ask on a field: serif 45/46.
pub const ASK: TextStyle = style(SERIF, 45.0, 46.0, FontWeight::NORMAL, false);
/// Screen titles: serif 44/44.
pub const TITLE: TextStyle = style(SERIF, 44.0, 44.0, FontWeight::NORMAL, false);
/// Numbers that are the point: serif 40.
pub const STAT: TextStyle = style(SERIF, 40.0, 40.0, FontWeight::NORMAL, false);
/// Sheet titles: serif 32/34, with one italic word.
pub const SHEET_TITLE: TextStyle = style(SERIF, 32.0, 34.0, FontWeight::NORMAL, false);
/// The ask on a waiting card: serif 25/26.
pub const ASK_SMALL: TextStyle = style(SERIF, 25.0, 26.0, FontWeight::NORMAL, false);
/// One quiet line under a title: serif italic 24/30, in `ink-2`.
pub const MOOD: TextStyle = style(SERIF, 24.0, 30.0, FontWeight::NORMAL, true);
/// Decision buttons: sans 16.5, 600.
pub const BUTTON: TextStyle = style(SANS, 16.5, 20.0, FontWeight::SEMIBOLD, false);
/// Row titles: sans 15.5, 500.
pub const ROW: TextStyle = style(SANS, 15.5, 20.0, FontWeight::MEDIUM, false);
/// Explanations: sans 15/22.
pub const BODY: TextStyle = style(SANS, 15.0, 22.0, FontWeight::NORMAL, false);
/// Row subtitles, hints: sans 13/18.
pub const META: TextStyle = style(SANS, 13.0, 18.0, FontWeight::NORMAL, false);
/// Tags: sans 11.5, 600.
pub const TAG: TextStyle = style(SANS, 11.5, 14.0, FontWeight::SEMIBOLD, false);
/// Label lines and section labels, in capitals: mono 11.5/14, 500, tracked +0.16em ([`tracked`]).
pub const LABEL: TextStyle = style(MONO, 11.5, 14.0, FontWeight::MEDIUM, false);
/// Branches, accounts, SHAs, the timer: mono 13.5.
pub const VALUE: TextStyle = style(MONO, 13.5, 18.0, FontWeight::NORMAL, false);

/// The whole scale by name, for the gallery.
pub const SCALE: [(&str, TextStyle); 15] = [
    ("ask-xl", ASK_XL),
    ("ask", ASK),
    ("title", TITLE),
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
    // Desktop rows and buttons are a size down from the phone's (`Desk-*.dc.html`).
    ("desk-row", style(SANS, 14.5, 19.0, FontWeight::MEDIUM, false)),
];

/// The tracking of [`LABEL`], in ems.
pub const LABEL_TRACKING: f32 = 0.16;

/// Signal's type styles on any element.
pub trait SignalText: Styled + Sized {
    /// Sets `s`: family, size, line height, weight, italic or upright.
    fn signal(self, s: TextStyle) -> Self {
        let el = self.font_family(s.family).text_size(px(s.size)).line_height(px(s.line)).font_weight(s.weight);
        if s.italic {
            el.italic()
        } else {
            el.not_italic()
        }
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

    /// The label's face; for its tracking, lay it out with [`tracked`].
    fn label(self) -> Self {
        self.signal(LABEL)
    }

    fn value(self) -> Self {
        self.signal(VALUE)
    }
}

impl<T: Styled> SignalText for T {}

/// A label line, in capitals and tracked +0.16em: GPUI has no letter-spacing, so each character is its own box with
/// the tracking after it (the label's colour is the caller's).
pub fn tracked(text: impl Into<SharedString>) -> Div {
    tracked_at(text, LABEL)
}

/// [`tracked`] in another mono style (its size sets the gap).
pub fn tracked_at(text: impl Into<SharedString>, s: TextStyle) -> Div {
    let text: SharedString = text.into();
    let gap = s.size * LABEL_TRACKING;
    let mut line = div().flex().flex_none().items_center().whitespace_nowrap().signal(s);
    for c in text.to_uppercase().chars() {
        // A space is as wide as a character in mono, with the tracking after it as everywhere else.
        line = line.child(div().flex_none().mr(px(gap)).child(SharedString::from(c.to_string())));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_bold_and_the_serif_is_regular() {
        for (name, s) in SCALE {
            assert!(s.weight.0 <= 600.0, "{name}");
            if s.family == SERIF {
                assert_eq!(s.weight, FontWeight::NORMAL, "{name}");
            }
            assert!(s.line >= s.size * 0.9, "{name}");
        }
        const { assert!(MOOD.italic) };
        assert_eq!((ASK_XL.size, ASK_XL.line), (64.0, 62.0));
        assert_eq!((LABEL.family, LABEL.size), (MONO, 11.5));
    }
}
