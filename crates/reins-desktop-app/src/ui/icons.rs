//! Icons, logos and drawings, embedded as SVG assets. Icons are the Reins Signal glyphs (`assets/icons/glyphs/`, the
//! one set of all three platforms, copied from `design/signal/assets/icons`): a 24 grid, a 1.6 stroke, round caps,
//! drawn by GPUI in one colour, the text colour given. Logos (`assets/icons/logos/`) are in their own colours, drawn
//! as images in a white circle. The pond and the kite (`assets/illustrations/`) are ink drawings for empty states and
//! arrivals, one for light paper and one for dark.

use std::borrow::Cow;

use gpui::{AnyElement, AssetSource, Div, Hsla, Img, IntoElement, ParentElement as _, SharedString, Styled as _, Svg};
use gpui::{div, img, px, relative, svg};

use crate::theme::Palette;

macro_rules! glyphs {
    ($($variant:ident => $file:literal),+ $(,)?) => {
        /// A Signal glyph.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Glyph {
            $($variant),+
        }

        impl Glyph {
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            /// Its name in the set (`index.json`).
            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $file),+
                }
            }

            /// The asset's path, as [`Assets`] knows it.
            #[must_use]
            pub fn path(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!("icons/glyphs/", $file, ".svg")),+
                }
            }

            fn bytes(self) -> &'static [u8] {
                match self {
                    $(Self::$variant => include_bytes!(concat!("../../assets/icons/glyphs/", $file, ".svg"))),+
                }
            }
        }
    };
}

glyphs!(
    Activity => "activity",
    Appearance => "appearance",
    ArrowBack => "arrow-back",
    ArrowForward => "arrow-forward",
    AutopilotOff => "autopilot-off",
    Autopilot => "autopilot",
    Bell => "bell",
    Block => "block",
    Briefcase => "briefcase",
    Calendar => "calendar",
    Camera => "camera",
    Card => "card",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    Chip => "chip",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    CircleClose => "circle-close",
    Clipboard => "clipboard",
    Close => "close",
    CloudCog => "cloud-cog",
    Cloud => "cloud",
    CloudUpload => "cloud-upload",
    Copy => "copy",
    Database => "database",
    Devices => "devices",
    Download => "download",
    Eye => "eye",
    FaceId => "face-id",
    FileCode => "file-code",
    FileLock => "file-lock",
    File => "file",
    FileText => "file-text",
    Filter => "filter",
    Fingerprint => "fingerprint",
    Flask => "flask",
    FolderSearch => "folder-search",
    GitBranch => "git-branch",
    GitCommit => "git-commit",
    Globe => "globe",
    Haptics => "haptics",
    HardDrive => "hard-drive",
    Heart => "heart",
    Help => "help",
    Home => "home",
    IdCard => "id-card",
    Infinity => "infinity",
    Key => "key",
    Laptop => "laptop",
    Lifebuoy => "lifebuoy",
    Link => "link",
    LockOpen => "lock-open",
    Lock => "lock",
    Mail => "mail",
    Minus => "minus",
    Moon => "moon",
    More => "more",
    Package => "package",
    Pause => "pause",
    Pencil => "pencil",
    Person => "person",
    Phone => "phone",
    Play => "play",
    Plus => "plus",
    Power => "power",
    QrCode => "qr-code",
    Quit => "quit",
    Refresh => "refresh",
    Rocket => "rocket",
    ScanSearch => "scan-search",
    Search => "search",
    Send => "send",
    Server => "server",
    Settings => "settings",
    Share => "share",
    ShieldAlert => "shield-alert",
    ShieldCheckFilled => "shield-check-filled",
    ShieldCheck => "shield-check",
    ShieldPaused => "shield-paused",
    Shield => "shield",
    ShieldWaiting => "shield-waiting",
    Speaker => "speaker",
    Star => "star",
    Terminal => "terminal",
    TimerOff => "timer-off",
    Timer => "timer",
    Token => "token",
    Trash => "trash",
    Tray => "tray",
    Undo => "undo",
    Unlink => "unlink",
    Vault => "vault",
    Warning => "warning",
);

macro_rules! files {
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
                    $(Self::$variant => concat!($dir, "/", $file, ".svg")),+
                }
            }

            fn bytes(self) -> &'static [u8] {
                match self {
                    $(Self::$variant => include_bytes!(concat!("../../assets/", $dir, "/", $file, ".svg"))),+
                }
            }
        }
    };
}

files!(Brand, "icons/logos",
    Claude => "claude",
    Gemini => "gemini",
    Cursor => "cursor",
    GitHub => "github",
    GitLab => "gitlab",
    Codeberg => "codeberg",
    Bitbucket => "bitbucket",
    Anthropic => "anthropic",
    OpenAI => "openai",
    Gmail => "gmail",
    Reins => "reins",
);

files!(Drawing, "illustrations",
    PondLight => "pond-light",
    PondDark => "pond-dark",
    KiteLight => "kite-light",
    KiteDark => "kite-dark",
);

impl Brand {
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

/// The ink drawings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Illustration {
    /// Empty, all quiet (320 × 170).
    Pond,
    /// Arrival (320 × 240).
    Kite,
}

impl Illustration {
    /// The drawing for this palette's paper.
    #[must_use]
    pub fn drawing(self, pal: Palette) -> Drawing {
        match (self, pal.dark) {
            (Self::Pond, false) => Drawing::PondLight,
            (Self::Pond, true) => Drawing::PondDark,
            (Self::Kite, false) => Drawing::KiteLight,
            (Self::Kite, true) => Drawing::KiteDark,
        }
    }

    /// Height over width.
    #[must_use]
    pub fn aspect(self) -> f32 {
        match self {
            Self::Pond => 170.0 / 320.0,
            Self::Kite => 240.0 / 320.0,
        }
    }
}

/// An ink drawing, `width` px wide, for the paper of `pal`.
pub fn illustration(which: Illustration, width: f32, pal: Palette) -> Img {
    img(which.drawing(pal).path()).w(px(width)).h(px(width * which.aspect())).flex_none()
}

/// A glyph, `side` px square, in `color`.
pub fn glyph(g: Glyph, side: f32, color: Hsla) -> Svg {
    svg().path(g.path()).size(px(side)).flex_none().text_color(color)
}

/// A brand's mark in its own colours, `side` px square.
pub fn brand(brand: Brand, side: f32) -> Img {
    img(brand.path()).size(px(side)).flex_none()
}

/// A logo as Signal shows one: in a white circle, `side` px (28 in rows), the mark at 62% of it, a hairline around.
pub fn avatar(b: Brand, side: f32, pal: Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded_full()
        .bg(gpui::white())
        .border_1()
        .border_color(pal.rule())
        .child(img(b.path()).size(relative(0.62)).flex_none())
}

/// Either: what a logo square shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Glyph(Glyph),
    Brand(Brand),
}

impl Mark {
    /// The mark, `side` px: a glyph in `color`, a brand in its own colours.
    pub fn render(self, side: f32, color: Hsla) -> AnyElement {
        match self {
            Self::Glyph(g) => glyph(g, side, color).into_any_element(),
            // The marks fill their square: a little smaller than an icon's strokes look.
            Self::Brand(b) => brand(b, side * 0.85).into_any_element(),
        }
    }
}

/// The grain over colour fields (`design/signal/assets/textures/grain.png`), as PNG bytes.
pub const GRAIN_PNG: &[u8] = include_bytes!("../../assets/textures/grain.png");

/// The app's SVGs, for GPUI's `svg().path(…)` and `img(…)`.
pub struct Assets;

impl Assets {
    fn all() -> impl Iterator<Item = (&'static str, &'static [u8])> {
        Glyph::ALL
            .iter()
            .map(|g| (g.path(), g.bytes()))
            .chain(Brand::ALL.iter().map(|b| (b.path(), b.bytes())))
            .chain(Drawing::ALL.iter().map(|d| (d.path(), d.bytes())))
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::all().find(|(p, _)| *p == path).map(|(_, bytes)| Cow::Borrowed(bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Self::all().map(|(p, _)| p).filter(|p| p.starts_with(path)).map(SharedString::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn text(path: &str) -> String {
        let bytes = Assets.load(path).unwrap().unwrap_or_else(|| panic!("{path} missing"));
        String::from_utf8(bytes.into_owned()).unwrap()
    }

    #[test]
    fn every_glyph_is_in_the_set_and_loads() {
        for g in Glyph::ALL {
            let svg = text(g.path());
            assert!(svg.contains("viewBox=\"0 0 24 24\"") && svg.contains("currentColor"), "{g:?}");
        }
        let names: HashSet<_> = Glyph::ALL.iter().map(|g| g.name()).collect();
        assert_eq!(names.len(), Glyph::ALL.len());
        // The set here is the design's, all of it: every glyph in `index.json` is here, and nothing else is.
        let index: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../../design/signal/assets/icons/index.json")).unwrap();
        let listed: HashSet<&str> = index.iter().filter_map(|e| e["name"].as_str()).collect();
        assert_eq!(listed, names);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons/glyphs");
        let files: HashSet<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().trim_end_matches(".svg").to_owned())
            .collect();
        assert_eq!(files, names.iter().map(|n| (*n).to_owned()).collect());
    }

    #[test]
    fn logos_and_drawings_load_in_colour() {
        for b in Brand::ALL {
            let svg = text(b.path());
            assert!(svg.contains("<svg") && svg.contains("fill=\"#"), "{b:?} has no colour of its own");
        }
        for d in Drawing::ALL {
            assert!(text(d.path()).contains("<svg"), "{d:?}");
        }
        assert!(Assets.load("icons/glyphs/nope.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/logos/").unwrap().len(), Brand::ALL.len());
        assert_eq!(Assets.list("illustrations/").unwrap().len(), 4);
        assert_eq!(&GRAIN_PNG[1..4], b"PNG");
    }

    #[test]
    fn hosts_have_their_marks() {
        assert_eq!(Brand::of_host("github.com"), Some(Brand::GitHub));
        assert_eq!(Brand::of_host("GitLab.com/acme/web"), Some(Brand::GitLab));
        assert_eq!(Brand::of_host("git.example.com"), None);
    }
}
