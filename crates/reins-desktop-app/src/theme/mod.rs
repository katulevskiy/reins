//! Reins Signal, light and dark (`design/signal`): paper, ink and hairlines for everything you read, a solid field of
//! colour only where something needs a decision ([`Field`]); and its three voices, Instrument Serif, Instrument Sans
//! and Geist Mono ([`text`]). The colours are the generated tokens ([`tokens`]); the names the screens used before
//! Signal ([`Palette::background`], [`Palette::accent`], …) are kept as aliases of the new values until the screens
//! move to the new names.

pub mod motion;
pub mod text;
#[rustfmt::skip]
pub mod tokens;

use std::borrow::Cow;

use gpui::{App, Hsla, Rgba, Window, WindowAppearance};
use reins_desktop::journal::Kind;

pub use text::SignalText;
use tokens::Token;

/// Asks, titles, big numbers and the mood line. Never bold; the target of an ask is in italic.
pub const SERIF: &str = "Instrument Serif";
/// Everything you read.
pub const SANS: &str = "Instrument Sans";
/// Labels in capitals and machine values (branches, SHAs, timers).
pub const MONO: &str = "Geist Mono";
/// The sans before Signal, still registered as the fallback.
#[allow(dead_code, reason = "named in the font tests")]
pub const FALLBACK_SANS: &str = "Geist";

/// The faces, as the phone apps ship them (SIL Open Font License 1.1, `ios/Shared/Fonts/FONTS-NOTICE.txt`). No 700:
/// Signal's heaviest weight is 600.
const FONTS: [&[u8]; 11] = [
    include_bytes!("../../../../ios/Shared/Fonts/InstrumentSerif-Regular.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/InstrumentSerif-Italic.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/InstrumentSans-Regular.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/InstrumentSans-Medium.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/InstrumentSans-SemiBold.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/GeistMono.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/GeistMono-Medium.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/Geist.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/Geist-Medium.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../../../ios/Shared/Fonts/Geist-Bold.ttf"),
];

pub fn register_fonts(cx: &App) {
    register_fonts_in(cx.text_system());
}

fn register_fonts_in(text: &gpui::TextSystem) {
    if let Err(e) = text.add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect()) {
        log::warn!("cannot load the Signal fonts: {e}");
    }
}

/// A colour from `0xRRGGBB` and an alpha.
#[must_use]
pub fn rgba(rgb: u32, alpha: f32) -> Hsla {
    Rgba {
        r: f32::from(u8::try_from((rgb >> 16) & 0xFF).unwrap_or(0)) / 255.0,
        g: f32::from(u8::try_from((rgb >> 8) & 0xFF).unwrap_or(0)) / 255.0,
        b: f32::from(u8::try_from(rgb & 0xFF).unwrap_or(0)) / 255.0,
        a: alpha,
    }
    .into()
}

/// A token's colour in light or dark.
#[must_use]
pub fn token(t: Token, dark: bool) -> Hsla {
    if dark {
        rgba(t.dark, t.dark_a)
    } else {
        rgba(t.light, t.light_a)
    }
}

/// The desktop sidebar's ground (`--side` in `Desk-Home.dc.html`; not a phone token).
const SIDE: Token = Token {
    light: 0x00EE_EEF2,
    light_a: 1.0,
    dark: 0x000C_0C0F,
    dark_a: 1.0,
};

/// A decision surface: what kind of thing is asked. The solid colour is the same in both themes; the kind ink (for a
/// tile on paper) is lighter in dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// Writes or sends.
    Push,
    /// Reads or searches.
    Read,
    /// Can't be undone, pays, bypasses.
    Delete,
    /// Power already handed over (a grant, a work session, Overnight).
    Grant,
    /// Connects something new.
    Pair,
    /// Changes an account.
    Accounts,
    /// Reins is broken and needs you.
    Warn,
}

impl Field {
    pub const ALL: [Self; 7] =
        [Self::Push, Self::Read, Self::Delete, Self::Grant, Self::Pair, Self::Accounts, Self::Warn];

    /// The solid field token.
    #[must_use]
    pub fn token(self) -> Token {
        match self {
            Self::Push => tokens::FIELD_PUSH,
            Self::Read => tokens::FIELD_READ,
            Self::Delete => tokens::FIELD_DELETE,
            Self::Grant => tokens::FIELD_GRANT,
            Self::Pair => tokens::FIELD_PAIR,
            Self::Accounts => tokens::FIELD_ACCOUNTS,
            Self::Warn => tokens::FIELD_WARN,
        }
    }

    /// The kind ink token.
    #[must_use]
    pub fn kind_token(self) -> Token {
        match self {
            Self::Push => tokens::KIND_PUSH,
            Self::Read => tokens::KIND_READ,
            Self::Delete => tokens::KIND_DELETE,
            Self::Grant => tokens::KIND_GRANT,
            Self::Pair => tokens::KIND_PAIR,
            Self::Accounts => tokens::KIND_ACCOUNTS,
            Self::Warn => tokens::KIND_WARN,
        }
    }

    /// The solid colour of the field (the same in light and dark).
    #[must_use]
    pub fn field(self) -> Hsla {
        token(self.token(), false)
    }

    /// The field as `0xRRGGBB`, for the grain baked over it.
    #[must_use]
    pub fn rgb(self) -> u32 {
        self.token().light
    }

    /// The kind's ink on paper: a kind tile's glyph.
    #[must_use]
    pub fn kind(self, pal: &Palette) -> Hsla {
        token(self.kind_token(), pal.dark)
    }

    /// A kind tile's fill: the kind ink at 13% (14% on dark paper).
    #[must_use]
    pub fn tile(self, pal: &Palette) -> Hsla {
        self.kind(pal).opacity(if pal.dark {
            0.14
        } else {
            0.13
        })
    }

    /// The field's name on its label line ("PUSH").
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Push => "Push",
            Self::Read => "Read",
            Self::Delete => "Delete",
            Self::Grant => "Grant",
            Self::Pair => "Pair",
            Self::Accounts => "Accounts",
            Self::Warn => "Warn",
        }
    }

    /// What the colour means.
    #[must_use]
    pub fn means(self) -> &'static str {
        match self {
            Self::Push => "Writes or sends",
            Self::Read => "Reads or searches",
            Self::Delete => "Can't be undone",
            Self::Grant => "Power handed over",
            Self::Pair => "Connects something new",
            Self::Accounts => "Changes an account",
            Self::Warn => "Reins needs you",
        }
    }

    /// The field a request of `kind` asks on (before a rule group makes it riskier).
    #[must_use]
    #[allow(dead_code, reason = "phase 2's waiting field asks with it")]
    pub fn of(kind: Kind) -> Self {
        match kind {
            // git through Reins is pushes; commands run things, MCP tools act.
            Kind::Git | Kind::Command | Kind::Mcp => Self::Push,
            // Reading files, keys and secrets, signing in with a key.
            Kind::File | Kind::Api | Kind::Secrets | Kind::Ssh => Self::Read,
            // A question from `reins ask`: Reins asking for the agent.
            Kind::Ask => Self::Grant,
        }
    }
}

/// What a tile or an icon on paper is coloured by: a field's kind ink, success (green, additions and approvals), or
/// nothing (neutral: `paper-sunken` under `ink`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tone {
    Field(Field),
    Success,
    Neutral,
}

impl From<Field> for Tone {
    fn from(f: Field) -> Self {
        Self::Field(f)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    // The names before Signal, now aliases of its tokens. Phase 2 moves the screens to the token methods below and
    // these go.
    /// `paper`.
    pub background: Hsla,
    /// The desktop sidebar (`side`).
    pub sidebar: Hsla,
    /// `paper-raised`.
    pub elevated: Hsla,
    /// `ink`.
    pub text: Hsla,
    /// `ink-2`.
    pub secondary: Hsla,
    /// `ink-meta`.
    pub tertiary: Hsla,
    /// `rule`.
    pub hairline: Hsla,
    /// `violet` (text and glyphs on paper; fills use [`Self::violet_fill`]).
    pub accent: Hsla,
    /// `violet` at 12%.
    pub accent_soft: Hsla,
    /// `on-field`: white on `violet-fill`.
    pub on_accent: Hsla,
    /// `paper-sunken`.
    pub control_fill: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    /// Dark paper.
    pub dark: bool,
}

impl Palette {
    #[must_use]
    pub fn light() -> Self {
        Self::new(false)
    }

    #[must_use]
    pub fn dark() -> Self {
        Self::new(true)
    }

    fn new(dark: bool) -> Self {
        let t = |t: Token| token(t, dark);
        Self {
            background: t(tokens::PAPER),
            sidebar: t(SIDE),
            elevated: t(tokens::PAPER_RAISED),
            text: t(tokens::INK),
            secondary: t(tokens::INK_2),
            tertiary: t(tokens::INK_META),
            hairline: t(tokens::RULE),
            accent: t(tokens::VIOLET),
            accent_soft: t(tokens::VIOLET).opacity(0.12),
            on_accent: t(tokens::ON_FIELD),
            control_fill: t(tokens::PAPER_SUNKEN),
            danger: t(tokens::DANGER),
            success: t(tokens::SUCCESS),
            warning: t(tokens::WARNING),
            dark,
        }
    }

    /// A token in this palette's theme.
    #[must_use]
    pub fn token(&self, t: Token) -> Hsla {
        token(t, self.dark)
    }

    // Signal's names.

    /// The ground of every screen.
    #[must_use]
    pub fn paper(&self) -> Hsla {
        self.token(tokens::PAPER)
    }

    /// Sheets, inputs, plates, header buttons.
    #[must_use]
    pub fn paper_raised(&self) -> Hsla {
        self.token(tokens::PAPER_RAISED)
    }

    /// Wells, segmented tracks, soft buttons, tags, a switch that is off.
    #[must_use]
    pub fn paper_sunken(&self) -> Hsla {
        self.token(tokens::PAPER_SUNKEN)
    }

    /// The desktop sidebar's ground.
    #[must_use]
    pub fn side(&self) -> Hsla {
        self.token(SIDE)
    }

    /// Titles, row titles, values.
    #[must_use]
    pub fn ink(&self) -> Hsla {
        self.token(tokens::INK)
    }

    /// Row keys, body copy.
    #[must_use]
    pub fn ink_2(&self) -> Hsla {
        self.token(tokens::INK_2)
    }

    /// Meta, dates, section labels.
    #[must_use]
    pub fn ink_meta(&self) -> Hsla {
        self.token(tokens::INK_META)
    }

    /// Chevrons and empty rings only, never text.
    #[must_use]
    pub fn ink_faint(&self) -> Hsla {
        self.token(tokens::INK_FAINT)
    }

    /// The hairline under rows and around plates.
    #[must_use]
    pub fn rule(&self) -> Hsla {
        self.token(tokens::RULE)
    }

    /// The stronger hairline: outline buttons, inputs.
    #[must_use]
    pub fn rule_2(&self) -> Hsla {
        self.token(tokens::RULE_2)
    }

    /// The main button: ink turned into a plate.
    #[must_use]
    pub fn inverse(&self) -> Hsla {
        self.token(tokens::INVERSE)
    }

    /// What goes on [`Self::inverse`].
    #[must_use]
    pub fn on_inverse(&self) -> Hsla {
        self.token(tokens::ON_INVERSE)
    }

    /// Violet as text or a glyph on paper.
    #[must_use]
    pub fn violet(&self) -> Hsla {
        self.token(tokens::VIOLET)
    }

    /// Violet as a fill: switches, the Auto chip, badges (white on it).
    #[must_use]
    pub fn violet_fill(&self) -> Hsla {
        self.token(tokens::VIOLET_FILL)
    }

    /// Behind a red button on paper.
    #[must_use]
    pub fn danger_wash(&self) -> Hsla {
        self.token(tokens::DANGER_WASH)
    }

    /// The fill of a hold button as you hold.
    #[must_use]
    pub fn danger_fill_2(&self) -> Hsla {
        self.token(tokens::DANGER_FILL_2)
    }

    /// Behind sheets.
    #[must_use]
    pub fn scrim(&self) -> Hsla {
        self.token(tokens::SCRIM)
    }

    /// Text and glyphs on a field.
    #[must_use]
    pub fn on_field(&self) -> Hsla {
        self.token(tokens::ON_FIELD)
    }

    /// Secondary text on a field (14 px and up).
    #[must_use]
    pub fn on_field_2(&self) -> Hsla {
        self.token(tokens::ON_FIELD_2)
    }

    /// Translucent buttons on a field.
    #[must_use]
    pub fn on_field_wash(&self) -> Hsla {
        self.token(tokens::ON_FIELD_WASH)
    }

    /// The colour of `tone` on paper: a kind's ink, success, or `ink-2`.
    #[must_use]
    pub fn tone(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Field(f) => f.kind(self),
            Tone::Success => self.success,
            Tone::Neutral => self.ink_2(),
        }
    }

    /// A tile's fill for `tone`: the ink at 13%, or `paper-sunken`.
    #[must_use]
    pub fn tone_tile(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Field(f) => f.tile(self),
            Tone::Success => self.success.opacity(0.13),
            Tone::Neutral => self.paper_sunken(),
        }
    }

    // The names before Signal (methods), as aliases.

    /// `kind-read`.
    #[must_use]
    pub fn blue(&self) -> Hsla {
        Field::Read.kind(self)
    }

    /// `kind-push`.
    #[must_use]
    pub fn orange(&self) -> Hsla {
        Field::Push.kind(self)
    }

    /// `kind-pair` (the old teal).
    #[must_use]
    pub fn teal(&self) -> Hsla {
        Field::Pair.kind(self)
    }

    /// `ink-meta`.
    #[must_use]
    pub fn gray(&self) -> Hsla {
        self.ink_meta()
    }

    /// `paper-sunken`.
    #[must_use]
    pub fn well(&self) -> Hsla {
        self.paper_sunken()
    }

    /// `paper-sunken`.
    #[must_use]
    pub fn tile(&self) -> Hsla {
        self.paper_sunken()
    }

    /// `rule`.
    #[must_use]
    pub fn tile_border(&self) -> Hsla {
        self.rule()
    }

    /// `paper-raised`.
    #[must_use]
    pub fn popover(&self) -> Hsla {
        self.paper_raised()
    }

    /// `rule`.
    #[must_use]
    pub fn popover_border(&self) -> Hsla {
        self.rule()
    }

    /// For the window's appearance (light or dark), unless `REINS_APPEARANCE` says `light` or `dark` (for
    /// screenshots of both).
    #[must_use]
    pub fn of(window: &Window) -> Self {
        match std::env::var("REINS_APPEARANCE").as_deref() {
            Ok("light") => return Self::light(),
            Ok("dark") => return Self::dark(),
            _ => {}
        }
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use gpui::{FontStyle, FontWeight, font};

    use super::*;

    #[test]
    fn the_old_names_are_the_new_values() {
        for pal in [Palette::light(), Palette::dark()] {
            assert_eq!(pal.background, pal.paper());
            assert_eq!(pal.elevated, pal.paper_raised());
            assert_eq!(pal.control_fill, pal.paper_sunken());
            assert_eq!((pal.text, pal.secondary, pal.tertiary), (pal.ink(), pal.ink_2(), pal.ink_meta()));
            assert_eq!(pal.hairline, pal.rule());
            assert_eq!(pal.accent, pal.violet());
        }
        assert_eq!(Palette::light().danger, rgba(0x00B9_1C1C, 1.0));
        assert_eq!(Palette::light().sidebar, rgba(0x00EE_EEF2, 1.0));
        assert_eq!(Palette::dark().sidebar, rgba(0x000C_0C0F, 1.0));
    }

    /// Fields stay, paper flips.
    #[test]
    fn fields_are_the_same_in_both_themes() {
        for f in Field::ALL {
            assert_eq!(f.token().light, f.token().dark, "{f:?}");
            assert_eq!(f.field(), token(f.token(), true));
        }
        assert_ne!(Field::Push.kind(&Palette::light()), Field::Push.kind(&Palette::dark()));
        assert_eq!(Field::Delete.field(), rgba(0x00B9_1C1C, 1.0));
    }

    #[test]
    fn every_kind_has_a_field() {
        for kind in Kind::ALL {
            let f = Field::of(kind);
            assert!(Field::ALL.contains(&f), "{kind:?}");
        }
        assert_eq!(Field::of(Kind::Git), Field::Push);
        assert_eq!(Field::of(Kind::Secrets), Field::Read);
    }

    /// The faces are registered: the italic is the real italic, and 400, 500 and 600 are three faces (700 falls to
    /// 600, the heaviest there is).
    #[test]
    #[cfg(target_os = "linux")]
    fn the_signal_faces_resolve() {
        let platform = gpui_platform::current_platform(true);
        let text = gpui::TextSystem::new(platform.text_system());
        register_fonts_in(&text);
        let names = text.all_font_names();
        for family in [SERIF, SANS, MONO, FALLBACK_SANS] {
            assert!(names.iter().any(|n| n == family), "{family} is not registered: {names:?}");
        }
        let serif = text.resolve_font(&font(SERIF));
        let italic = text.resolve_font(&font(SERIF).italic());
        assert_ne!(serif, italic, "the italic is the regular face, slanted");
        assert_eq!(text.get_font_for_id(italic).map(|f| f.style), Some(FontStyle::Italic));
        let size = gpui::px(24.0);
        assert_ne!(
            text.advance(serif, size, 'a').unwrap().width,
            text.advance(italic, size, 'a').unwrap().width,
            "the italic has its own glyphs"
        );
        let weight = |w: FontWeight| {
            let mut f = font(SANS);
            f.weight = w;
            text.resolve_font(&f)
        };
        let faces = [weight(FontWeight::NORMAL), weight(FontWeight::MEDIUM), weight(FontWeight::SEMIBOLD)];
        assert!(faces[0] != faces[1] && faces[1] != faces[2] && faces[0] != faces[2], "{faces:?}");
        assert_eq!(weight(FontWeight::BOLD), faces[2]);
    }
}
