//! Reins Next, light and dark (`design/next`): paper, ink and hairlines for everything you read, frosted panes over a
//! dithered wallpaper on Home and Activity, a grained field of colour only where something needs a decision
//! ([`Field`]); and its two voices, Sora (Geologica under it for Cyrillic) and JetBrains Mono ([`text`]). The colours
//! are the generated tokens ([`tokens`], the same values Signal had) by their names ([`Palette::paper`],
//! [`Palette::ink`], …), in light or dark as the system says or as picked in Settings ([`Appearance`]).

pub mod motion;
pub mod text;
#[rustfmt::skip]
pub mod tokens;

use std::borrow::Cow;

use gpui::{App, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, Hsla, Rgba, Window, WindowAppearance};
use reins_desktop::journal::Kind;

pub use text::NextText;
use tokens::Token;

/// Titles, asks, big numbers: Sora at 500 (the boards' display face).
pub const DISPLAY: &str = "Sora";
/// Everything you read.
pub const SANS: &str = "Sora";
/// Machine values: branches, hashes, addresses, codes, diffs, timers.
pub const MONO: &str = "JetBrains Mono";
/// Under Sora for Cyrillic only (it has no Cyrillic): Geologica 500.
pub const CYRILLIC: &str = "Geologica";

/// The faces (SIL Open Font License 1.1, `assets/fonts/FONTS-NOTICE.txt`): Sora 400, 500 and 600, Geologica 500 (its
/// Cyrillic only), JetBrains Mono 400 and 500.
const FONTS: [&[u8]; 6] = [
    include_bytes!("../../assets/fonts/Sora-Regular.ttf"),
    include_bytes!("../../assets/fonts/Sora-Medium.ttf"),
    include_bytes!("../../assets/fonts/Sora-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/Geologica-Medium.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Medium.ttf"),
];

pub fn register_fonts(cx: &App) {
    register_fonts_in(cx.text_system());
}

fn register_fonts_in(text: &gpui::TextSystem) {
    if let Err(e) = text.add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect()) {
        log::warn!("cannot load the fonts: {e}");
    }
}

/// The window's font: Sora, falling back to Geologica for what Sora lacks (Cyrillic). Set once on the root; every
/// element under it keeps the fallback whatever family it picks.
#[must_use]
pub fn root_font() -> Font {
    Font {
        family: SANS.into(),
        features: FontFeatures::default(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![CYRILLIC.to_owned()])),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
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

/// The desktop sidebar's ground (`.side` in `DeskNext-System.dc.html`: `paper-sunken` 70% into `paper`; not a phone
/// token).
const SIDE: Token = Token {
    light: 0x00EF_EFF2,
    light_a: 1.0,
    dark: 0x0020_2025,
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
    /// Deletions, destructive rows, a denial: red on paper.
    pub danger: Hsla,
    /// Additions, approvals, Ready: green on paper.
    pub success: Hsla,
    /// Flags: amber on paper.
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

    /// For the appearance picked in Settings (`System`: the window's, light or dark), unless `REINS_APPEARANCE` says
    /// `light` or `dark` (for screenshots of both).
    #[must_use]
    pub fn of(window: &Window, picked: Appearance) -> Self {
        let forced = match std::env::var("REINS_APPEARANCE").as_deref() {
            Ok("light") => Some(false),
            Ok("dark") => Some(true),
            _ => None,
        };
        let system = || matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
        if forced.or_else(|| picked.dark()).unwrap_or_else(system) {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

/// Light or dark paper, as picked in Settings: the system's, or one of the two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    /// Dark paper or not; `None`: as the system says.
    #[must_use]
    pub fn dark(self) -> Option<bool> {
        match self {
            Self::System => None,
            Self::Light => Some(false),
            Self::Dark => Some(true),
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use gpui::font;

    use super::*;

    #[test]
    fn the_palette_is_the_tokens() {
        for pal in [Palette::light(), Palette::dark()] {
            assert_eq!(pal.danger, pal.token(tokens::DANGER));
            assert_eq!(pal.success, pal.token(tokens::SUCCESS));
            assert_eq!(pal.warning, pal.token(tokens::WARNING));
        }
        assert_eq!(Palette::light().danger, rgba(0x00B9_1C1C, 1.0));
        assert_eq!(Palette::light().side(), rgba(0x00EF_EFF2, 1.0));
        assert_eq!(Palette::dark().side(), rgba(0x0020_2025, 1.0));
    }

    /// The appearance picked is saved by name; System follows the window.
    #[test]
    fn appearances_are_saved_by_name() {
        assert_eq!(serde_json::to_string(&Appearance::Dark).unwrap(), "\"dark\"");
        assert_eq!(serde_json::from_str::<Appearance>("\"light\"").unwrap(), Appearance::Light);
        assert_eq!(Appearance::default(), Appearance::System);
        assert_eq!(Appearance::System.dark(), None);
        assert_eq!(Appearance::Dark.dark(), Some(true));
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

    /// The faces are registered: Sora's 400, 500 and 600 are three faces (700 falls to 600, the heaviest there is),
    /// JetBrains Mono has its two, and Geologica is there for the Cyrillic Sora lacks.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_faces_resolve() {
        let platform = gpui_platform::current_platform(true);
        let text = gpui::TextSystem::new(platform.text_system());
        register_fonts_in(&text);
        let names = text.all_font_names();
        for family in [DISPLAY, SANS, MONO, CYRILLIC] {
            assert!(names.iter().any(|n| n == family), "{family} is not registered: {names:?}");
        }
        let weight = |family: &'static str, w: FontWeight| {
            let mut f = font(family);
            f.weight = w;
            text.resolve_font(&f)
        };
        let faces = [
            weight(SANS, FontWeight::NORMAL),
            weight(SANS, FontWeight::MEDIUM),
            weight(SANS, FontWeight::SEMIBOLD),
        ];
        assert!(faces[0] != faces[1] && faces[1] != faces[2] && faces[0] != faces[2], "{faces:?}");
        assert_eq!(weight(SANS, FontWeight::BOLD), faces[2]);
        assert_ne!(weight(MONO, FontWeight::NORMAL), weight(MONO, FontWeight::MEDIUM));
        // Sora has no Cyrillic; Geologica has it and nothing of Latin.
        let size = gpui::px(16.0);
        assert!(text.advance(faces[0], size, 'a').is_ok());
        let geologica = weight(CYRILLIC, FontWeight::MEDIUM);
        assert!(text.advance(geologica, size, 'Ж').unwrap().width > gpui::px(0.0));
        assert!(root_font().fallbacks.is_some_and(|f| f.fallback_list() == [CYRILLIC]));
    }
}
