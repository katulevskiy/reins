//! Icons and logos, embedded as SVG assets: [Lucide](https://lucide.dev) icons (ISC, `assets/icons/lucide/LICENSE`)
//! and brand marks from [Simple Icons](https://simpleicons.org) (CC0, `assets/icons/brands/LICENSE.md`). GPUI draws
//! an SVG in one colour, the text colour given here: the brand marks get their brand's colour, as on the phone.

use std::borrow::Cow;

use gpui::{AssetSource, Hsla, SharedString, Styled as _, Svg, px, svg};

use crate::theme::Palette;

macro_rules! assets {
    ($kind:ident, $dir:literal, $($variant:ident => $file:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $kind {
            $($variant),+
        }

        impl $kind {
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            /// The asset's path, as [`Assets`] knows it.
            #[must_use]
            pub fn path(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!("icons/", $dir, "/", $file, ".svg")),+
                }
            }

            fn bytes(self) -> &'static [u8] {
                match self {
                    $(Self::$variant => include_bytes!(concat!("../../assets/icons/", $dir, "/", $file, ".svg"))),+
                }
            }
        }
    };
}

assets!(Icon, "lucide",
    Activity => "activity",
    ArrowLeft => "arrow-left",
    ArrowRight => "arrow-right",
    Bell => "bell",
    Cable => "cable",
    Calendar => "calendar",
    Check => "check",
    ChevronRight => "chevron-right",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    CircleHelp => "circle-help",
    CircleX => "circle-x",
    Cloud => "cloud",
    CloudCog => "cloud-cog",
    CreditCard => "credit-card",
    Database => "database",
    Download => "download",
    File => "file",
    FileCode => "file-code",
    FileLock => "file-lock",
    GitBranch => "git-branch",
    Globe => "globe",
    HardDrive => "hard-drive",
    HeartPulse => "heart-pulse",
    Infinity => "infinity",
    KeyRound => "key-round",
    Laptop => "laptop",
    LayoutDashboard => "layout-dashboard",
    ListFilter => "list-filter",
    Lock => "lock",
    LogOut => "log-out",
    Mail => "mail",
    Question => "message-circle-question-mark",
    Package => "package",
    Pause => "pause",
    Play => "play",
    Plus => "plus",
    Power => "power",
    QrCode => "qr-code",
    Rocket => "rocket",
    Send => "send",
    Server => "server",
    Settings => "settings",
    ShieldCheck => "shield-check",
    Smartphone => "smartphone",
    SquareTerminal => "square-terminal",
    Terminal => "terminal",
    Ticket => "ticket",
    Timer => "timer",
    TimerOff => "timer-off",
    Trash => "trash-2",
    TriangleAlert => "triangle-alert",
    Unlink => "unlink",
    Wrench => "wrench",
    X => "x",
);

assets!(Brand, "brands",
    Claude => "claude",
    Gemini => "googlegemini",
    Cursor => "cursor",
    GitHub => "github",
    GitLab => "gitlab",
    Codeberg => "codeberg",
    Bitbucket => "bitbucket",
    Anthropic => "anthropic",
    Gmail => "gmail",
);

impl Brand {
    /// The brand's colour; the black-and-white marks (GitHub, Cursor, Anthropic) take the text colour.
    #[must_use]
    pub fn color(self, pal: Palette) -> Hsla {
        let rgb = match self {
            Self::Claude => 0x00D9_7757,
            Self::Gemini => 0x008E_75B2,
            Self::GitLab => 0x00FC_6D26,
            Self::Codeberg => 0x0021_85D0,
            Self::Bitbucket => 0x0026_84FF,
            Self::Gmail => 0x00EA_4335,
            Self::GitHub | Self::Cursor | Self::Anthropic => return pal.text,
        };
        gpui::rgb(rgb).into()
    }

    /// The mark of a git or SSH host (`github.com`), if it has one.
    #[must_use]
    pub fn of_host(host: &str) -> Option<Self> {
        let host = host.trim().to_ascii_lowercase();
        let host = host.split('/').next().unwrap_or("");
        Some(match host {
            "github.com" | "ssh.github.com" | "gist.github.com" => Self::GitHub,
            "gitlab.com" => Self::GitLab,
            "codeberg.org" => Self::Codeberg,
            "bitbucket.org" => Self::Bitbucket,
            _ => return None,
        })
    }
}

/// An icon, `side` px square, in `color`.
pub fn icon(icon: Icon, side: f32, color: Hsla) -> Svg {
    svg().path(icon.path()).size(px(side)).flex_none().text_color(color)
}

/// A brand's mark, `side` px square, in its colour.
pub fn brand(brand: Brand, side: f32, pal: Palette) -> Svg {
    svg().path(brand.path()).size(px(side)).flex_none().text_color(brand.color(pal))
}

/// Either: what a logo square shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Icon(Icon),
    Brand(Brand),
}

impl Mark {
    /// The mark, `side` px; an icon in `color`, a brand in its own.
    pub fn render(self, side: f32, color: Hsla, pal: Palette) -> Svg {
        match self {
            Self::Icon(i) => icon(i, side, color),
            // The marks fill their square: a little smaller than an icon's strokes look.
            Self::Brand(b) => brand(b, side * 0.85, pal),
        }
    }
}

/// The app's SVGs, for GPUI's `svg().path(…)`.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let found = Icon::ALL
            .iter()
            .find(|i| i.path() == path)
            .map(|i| i.bytes())
            .or_else(|| Brand::ALL.iter().find(|b| b.path() == path).map(|b| b.bytes()));
        Ok(found.map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Icon::ALL
            .iter()
            .map(|i| i.path())
            .chain(Brand::ALL.iter().map(|b| b.path()))
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asset_loads_and_is_an_svg() {
        let paths: Vec<&str> = Icon::ALL.iter().map(|i| i.path()).chain(Brand::ALL.iter().map(|b| b.path())).collect();
        for path in &paths {
            let bytes = Assets.load(path).unwrap().unwrap_or_else(|| panic!("{path} missing"));
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("<svg") && text.contains("viewBox=\"0 0 24 24\""), "{path}");
        }
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len());
        assert!(Assets.load("icons/lucide/nope.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/brands/").unwrap().len(), Brand::ALL.len());
    }

    #[test]
    fn hosts_have_their_marks() {
        assert_eq!(Brand::of_host("github.com"), Some(Brand::GitHub));
        assert_eq!(Brand::of_host("GitLab.com/acme/web"), Some(Brand::GitLab));
        assert_eq!(Brand::of_host("git.example.com"), None);
    }
}
