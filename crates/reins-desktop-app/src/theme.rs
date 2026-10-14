//! Reins Light / Reins Dark: the phone apps' palette (`ios/Shared/Design/Palette.swift`, the Android app's `RColors`)
//! and their typeface, Geist.

use std::borrow::Cow;

use gpui::{App, Hsla, Rgba, Window, WindowAppearance};

pub const FONT: &str = "Geist";
pub const MONO: &str = "Geist Mono";

/// The phone apps' copies of Geist (SIL Open Font License 1.1, `ios/Shared/Fonts/FONTS-NOTICE.txt`).
const FONTS: [&[u8]; 6] = [
    include_bytes!("../../../ios/Shared/Fonts/Geist.ttf"),
    include_bytes!("../../../ios/Shared/Fonts/Geist-Medium.ttf"),
    include_bytes!("../../../ios/Shared/Fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../../ios/Shared/Fonts/Geist-Bold.ttf"),
    include_bytes!("../../../ios/Shared/Fonts/GeistMono.ttf"),
    include_bytes!("../../../ios/Shared/Fonts/GeistMono-Medium.ttf"),
];

pub fn register_fonts(cx: &App) {
    if let Err(e) = cx.text_system().add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect()) {
        log::warn!("cannot load the Geist fonts: {e}");
    }
}

fn hex(rgb: u32) -> Hsla {
    Rgba {
        r: f32::from(u8::try_from((rgb >> 16) & 0xFF).unwrap_or(0)) / 255.0,
        g: f32::from(u8::try_from((rgb >> 8) & 0xFF).unwrap_or(0)) / 255.0,
        b: f32::from(u8::try_from(rgb & 0xFF).unwrap_or(0)) / 255.0,
        a: 1.0,
    }
    .into()
}

/// The colours things are told apart by, as on the phone: what a request is, how risky a rule is, which section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Violet,
    Green,
    Red,
    Yellow,
    Blue,
    Orange,
    Teal,
    Pink,
    Gray,
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub background: Hsla,
    /// The status window's sidebar.
    pub sidebar: Hsla,
    /// The selected sidebar item.
    pub selected: Hsla,
    pub elevated: Hsla,
    pub text: Hsla,
    pub secondary: Hsla,
    pub tertiary: Hsla,
    pub hairline: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub on_accent: Hsla,
    pub control_fill: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    /// Reins Dark.
    pub dark: bool,
}

impl Palette {
    #[must_use]
    pub fn light() -> Self {
        Self {
            background: hex(0x00F3_F3F5),
            sidebar: hex(0x00EA_EAEE),
            selected: hex(0x00FF_FFFF),
            elevated: hex(0x00FF_FFFF),
            text: hex(0x0027_272C),
            secondary: hex(0x0062_626A),
            tertiary: hex(0x0097_979F),
            hairline: hex(0x00E2_E2E6),
            accent: hex(0x005B_43E8),
            accent_soft: hex(0x005B_43E8).opacity(0.12),
            on_accent: hex(0x00FF_FFFF),
            control_fill: hex(0x0027_272C).opacity(0.075),
            danger: hex(0x00DC_2626),
            success: hex(0x0015_803D),
            warning: hex(0x00A1_6207),
            dark: false,
        }
    }

    #[must_use]
    pub fn dark() -> Self {
        Self {
            background: hex(0x0006_0606),
            sidebar: hex(0x000C_0C0E),
            selected: hex(0x001A_1A1E),
            elevated: hex(0x0011_1113),
            text: hex(0x00E8_E8EA),
            secondary: hex(0x00A9_A9AE),
            tertiary: hex(0x006B_6B72),
            hairline: hex(0x001E_1E22),
            accent: hex(0x008B_7CF6),
            accent_soft: hex(0x008B_7CF6).opacity(0.12),
            on_accent: hex(0x00FF_FFFF),
            control_fill: hex(0x00E8_E8EA).opacity(0.075),
            danger: hex(0x00F8_7171),
            success: hex(0x0034_D399),
            warning: hex(0x00FA_CC15),
            dark: true,
        }
    }

    // The phone apps' other colours and the surfaces of controls, by appearance (methods keep the palette small
    // enough to pass around by value).

    /// Searching and reading.
    #[must_use]
    pub fn blue(&self) -> Hsla {
        if self.dark {
            hex(0x0060_A5FA)
        } else {
            hex(0x0025_63EB)
        }
    }

    /// Writing.
    #[must_use]
    pub fn orange(&self) -> Hsla {
        if self.dark {
            hex(0x00FB_923C)
        } else {
            hex(0x00EA_580C)
        }
    }

    /// git and connections.
    #[must_use]
    pub fn teal(&self) -> Hsla {
        if self.dark {
            hex(0x002D_D4BF)
        } else {
            hex(0x000D_9488)
        }
    }

    /// Rules.
    #[must_use]
    pub fn pink(&self) -> Hsla {
        if self.dark {
            hex(0x00F4_72B6)
        } else {
            hex(0x00DB_2777)
        }
    }

    /// What has no colour of its own.
    #[must_use]
    pub fn gray(&self) -> Hsla {
        if self.dark {
            hex(0x006B_6B72)
        } else {
            hex(0x0086_868E)
        }
    }

    /// The track of a segmented control, a text field and a mono well: a step below the cards.
    #[must_use]
    pub fn well(&self) -> Hsla {
        if self.dark {
            hex(0x0006_0606)
        } else {
            hex(0x00F3_F3F5)
        }
    }

    /// The selected choice of a segmented control.
    #[must_use]
    pub fn raised(&self) -> Hsla {
        if self.dark {
            hex(0x0023_2327)
        } else {
            hex(0x00FF_FFFF)
        }
    }

    /// A logo's square, and the stage's tiles.
    #[must_use]
    pub fn tile(&self) -> Hsla {
        if self.dark {
            hex(0x001C_1C20)
        } else {
            hex(0x00F1_F1F4)
        }
    }

    /// Their border.
    #[must_use]
    pub fn tile_border(&self) -> Hsla {
        if self.dark {
            hex(0x002A_2A2F)
        } else {
            hex(0x00E2_E2E6)
        }
    }

    /// A plain button.
    #[must_use]
    pub fn button(&self) -> Hsla {
        if self.dark {
            hex(0x001F_1F23)
        } else {
            hex(0x0027_272C).opacity(0.075)
        }
    }

    /// The button that goes ahead (near-white on dark, near-black on light).
    #[must_use]
    pub fn primary(&self) -> Hsla {
        if self.dark {
            hex(0x00F2_F2F4)
        } else {
            hex(0x0027_272C)
        }
    }

    /// Its text.
    #[must_use]
    pub fn on_primary(&self) -> Hsla {
        if self.dark {
            hex(0x000A_0A0A)
        } else {
            hex(0x00FF_FFFF)
        }
    }

    /// A popover's background.
    #[must_use]
    pub fn popover(&self) -> Hsla {
        if self.dark {
            hex(0x0018_181B)
        } else {
            hex(0x00FF_FFFF)
        }
    }

    /// Its border.
    #[must_use]
    pub fn popover_border(&self) -> Hsla {
        if self.dark {
            hex(0x002A_2A30)
        } else {
            hex(0x00DC_DCE1)
        }
    }

    /// The colour of `tone`.
    #[must_use]
    pub fn tone(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Violet => self.accent,
            Tone::Green => self.success,
            Tone::Red => self.danger,
            Tone::Yellow => self.warning,
            Tone::Blue => self.blue(),
            Tone::Orange => self.orange(),
            Tone::Teal => self.teal(),
            Tone::Pink => self.pink(),
            Tone::Gray => self.gray(),
        }
    }

    /// What goes on a solid square of `tone` (a selected section's icon, Health checks' yellow).
    #[must_use]
    pub fn on_solid(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Yellow if self.dark => hex(0x0023_1A00),
            _ => hex(0x00FF_FFFF),
        }
    }

    /// The dark text on a red button that is held (`#2a0606`), or white on light.
    #[must_use]
    pub fn on_danger(&self) -> Hsla {
        if self.dark {
            hex(0x002A_0606)
        } else {
            hex(0x00FF_FFFF)
        }
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
