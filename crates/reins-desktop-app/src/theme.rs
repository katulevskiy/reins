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
            on_accent: hex(0x0006_0606),
            control_fill: hex(0x00E8_E8EA).opacity(0.075),
            danger: hex(0x00F8_7171),
            success: hex(0x0034_D399),
            warning: hex(0x00FA_CC15),
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
