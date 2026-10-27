//! The wallpapers (`design/next/desktop/_blob`, the boards' art): dithered pictures behind the title of Home and
//! Activity, faded into the page, and filling the window on the welcome. One for dark paper (Halo by default) and one
//! for light (Torii dither), picked in Settings.
//!
//! GPUI's `img` can only centre a cover fit, and the boards place the art higher (`object-position: 50% 30%`): each
//! picture is decoded into a [`RenderImage`] and painted by [`Art`], covering its box at the position asked.
//!
//! The phones' art is 780 × 1000; a wide window covered with it is blurred. The desktop ships its own, landscape and
//! 3200 × 1800, made from the owner's full-size art with each picture's look (`scripts/brand/desktop-wallpapers.py`).
//! GPUI samples a picture without mipmaps, so one drawn much smaller than it is shimmers: [`Art`] paints the halving
//! of the picture just above the size it is drawn at ([`level`]), each made once, when first drawn.

use std::panic::Location;
use std::sync::{Arc, OnceLock};

use gpui::Refineable as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Bounds, Corners, Div, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    ParentElement as _, Pixels, RenderImage, Style, StyleRefinement, Styled, Window, div, linear_color_stop,
    linear_gradient, point, px, size,
};

use crate::theme::{Palette, rgba};

/// A wallpaper of the set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Wallpaper {
    #[default]
    Halo,
    Crescent,
    Figure,
    Hands,
    Ruins,
    Eyes,
    Torii,
    ToriiDither,
}

impl Wallpaper {
    pub const ALL: [Self; 8] = [
        Self::Halo,
        Self::Crescent,
        Self::Figure,
        Self::Hands,
        Self::Ruins,
        Self::Eyes,
        Self::Torii,
        Self::ToriiDither,
    ];
    /// The default for dark paper.
    pub const DARK: Self = Self::Halo;
    /// The default for light paper.
    pub const LIGHT: Self = Self::ToriiDither;

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Halo => "Halo",
            Self::Crescent => "Crescent",
            Self::Figure => "Figure",
            Self::Hands => "Hands",
            Self::Ruins => "Ruins",
            Self::Eyes => "Eyes",
            Self::Torii => "Torii",
            Self::ToriiDither => "Torii dither",
        }
    }

    /// As `REINS_DEMO_WALLPAPER` and the settings file name it.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Halo => "halo",
            Self::Crescent => "crescent",
            Self::Figure => "figure",
            Self::Hands => "hands",
            Self::Ruins => "ruins",
            Self::Eyes => "eyes",
            Self::Torii => "torii",
            Self::ToriiDither => "torii-dither",
        }
    }

    /// The file as shipped, and its format: the dithered art is PNG (exact, and smaller), the photos JPEG.
    fn file(self) -> (&'static [u8], image::ImageFormat) {
        use image::ImageFormat::{Jpeg, Png};
        match self {
            Self::Halo => (include_bytes!("../../../assets/wallpapers/halo.png"), Png),
            Self::Crescent => (include_bytes!("../../../assets/wallpapers/crescent.jpg"), Jpeg),
            Self::Figure => (include_bytes!("../../../assets/wallpapers/figure.png"), Png),
            Self::Hands => (include_bytes!("../../../assets/wallpapers/hands.png"), Png),
            Self::Ruins => (include_bytes!("../../../assets/wallpapers/ruins.png"), Png),
            Self::Eyes => (include_bytes!("../../../assets/wallpapers/eyes.jpg"), Jpeg),
            Self::Torii => (include_bytes!("../../../assets/wallpapers/torii.jpg"), Jpeg),
            Self::ToriiDither => (include_bytes!("../../../assets/wallpapers/torii-dither.png"), Png),
        }
    }

    /// The picture's size as shipped, read once from its header.
    fn size(self) -> Option<(u32, u32)> {
        static SIZES: [OnceLock<Option<(u32, u32)>>; Wallpaper::ALL.len()] =
            [const { OnceLock::new() }; Wallpaper::ALL.len()];
        let i = Self::ALL.iter().position(|w| *w == self)?;
        *SIZES[i].get_or_init(|| {
            let (bytes, format) = self.file();
            image::ImageReader::with_format(std::io::Cursor::new(bytes), format).into_dimensions().ok()
        })
    }

    /// The picture at `level` (0: as shipped, each level half the one before), decoded once, as BGRA (GPUI's order).
    fn image(self, level: usize) -> Option<Arc<RenderImage>> {
        static DECODED: [[OnceLock<Option<Arc<RenderImage>>>; LEVELS]; Wallpaper::ALL.len()] =
            [const { [const { OnceLock::new() }; LEVELS] }; Wallpaper::ALL.len()];
        let i = Self::ALL.iter().position(|w| *w == self)?;
        let level = level.min(LEVELS - 1);
        DECODED[i][level]
            .get_or_init(|| match decode(self.file(), level) {
                Ok(img) => Some(img),
                Err(e) => {
                    log::warn!("cannot decode the {} wallpaper: {e}", self.label());
                    None
                }
            })
            .clone()
    }
}

/// How many sizes of a picture there can be: as shipped, then halved down to 1/32 (100 px wide, the smallest
/// thumbnail's).
const LEVELS: usize = 6;

/// The level to draw a `width` px picture at, `drawn` device pixels wide: the smallest halving still at least that
/// wide, so a picture is never shrunk to less than half when drawn, nor enlarged unless drawn bigger than it is.
fn level(width: u32, drawn: f32) -> usize {
    let mut level = 0;
    #[allow(clippy::cast_precision_loss, reason = "picture sizes")]
    while level + 1 < LEVELS && (width >> (level + 1)) as f32 >= drawn {
        level += 1;
    }
    level
}

fn decode((bytes, format): (&[u8], image::ImageFormat), level: usize) -> Result<Arc<RenderImage>, String> {
    let mut rgba = image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?.into_rgba8();
    if level > 0 {
        let (w, h) = ((rgba.width() >> level).max(1), (rgba.height() >> level).max(1));
        rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Triangle);
    }
    for p in rgba.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

/// The picture, covering its box, its point at `(x, y)` (fractions, as CSS's `object-position`) kept in view.
pub fn art(which: Wallpaper, x: f32, y: f32) -> Art {
    let mut a = Art {
        which,
        at: (x, y),
        radius: 0.0,
        style: StyleRefinement::default(),
    };
    a = a.size_full();
    a
}

/// The element that paints a wallpaper.
pub struct Art {
    which: Wallpaper,
    at: (f32, f32),
    radius: f32,
    style: StyleRefinement,
}

impl Art {
    /// Rounds the picture's corners.
    #[must_use]
    pub fn corners(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

impl Styled for Art {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for Art {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Art {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.refine(&self.style);
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        let Some((w, h)) = self.which.size() else {
            return;
        };
        #[allow(clippy::cast_possible_wrap, reason = "picture sizes")]
        let Some(fitted) = cover(bounds, w as i32, h as i32, self.at) else {
            return;
        };
        let Some(image) = self.which.image(level(w, fitted.size.width.as_f32() * window.scale_factor())) else {
            return;
        };
        if let Err(e) = window.paint_image_fitted(bounds, fitted, Corners::all(px(self.radius)), image, 0, false) {
            log::debug!("wallpaper: {e}");
        }
    }
}

/// The box an `iw` × `ih` picture covers `bounds` in, its point `at` (fractions) placed as CSS's `object-position`.
fn cover(bounds: Bounds<Pixels>, iw: i32, ih: i32, at: (f32, f32)) -> Option<Bounds<Pixels>> {
    let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
    #[allow(clippy::cast_precision_loss, reason = "picture sizes")]
    let (iw, ih) = (iw as f32, ih as f32);
    if w <= 0.0 || h <= 0.0 || iw <= 0.0 || ih <= 0.0 {
        return None;
    }
    let scale = (w / iw).max(h / ih);
    let (dw, dh) = (iw * scale, ih * scale);
    let x = bounds.origin.x.as_f32() + (w - dw) * at.0.clamp(0.0, 1.0);
    let y = bounds.origin.y.as_f32() + (h - dh) * at.1.clamp(0.0, 1.0);
    Some(Bounds::new(point(px(x), px(y)), size(px(dw), px(dh))))
}

/// The wallpaper behind a page's title (`.dwal`): `height` px of art across the top, faded into the paper from 35%
/// down, darkened (dark paper) or lightened (light paper) under the title so it reads. It takes no room: a child of
/// the (relative) page, under its content.
pub fn band(which: Wallpaper, height: f32, pal: Palette) -> Div {
    let paper = pal.paper();
    let shade = if pal.dark {
        div().absolute().top_0().left_0().right_0().h(px(170.0)).bg(linear_gradient(
            180.0,
            linear_color_stop(rgba(0, 0.6), 0.0),
            linear_color_stop(rgba(0, 0.0), 1.0),
        ))
    } else {
        div().absolute().top_0().left_0().right_0().h(px(140.0)).bg(linear_gradient(
            180.0,
            linear_color_stop(rgba(0x00F6_F6F8, 0.8), 0.0),
            linear_color_stop(rgba(0x00F6_F6F8, 0.0), 1.0),
        ))
    };
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(height))
        .overflow_hidden()
        .child(art(which, 0.5, 0.3))
        .child(div().absolute().inset_0().bg(linear_gradient(
            180.0,
            linear_color_stop(paper.opacity(0.0), 0.35),
            linear_color_stop(paper, 1.0),
        )))
        .child(shade)
}

/// The wallpaper filling a box (the welcome): the art, and the paper rising over its foot (`fade` px).
pub fn fill(which: Wallpaper, fade: f32, pal: Palette) -> Div {
    let paper = pal.paper();
    div().absolute().inset_0().overflow_hidden().child(art(which, 0.5, 0.35)).when(fade > 0.0, |d| {
        d.child(div().absolute().left_0().right_0().bottom_0().h(px(fade)).bg(linear_gradient(
            180.0,
            linear_color_stop(paper.opacity(0.0), 0.0),
            linear_color_stop(paper, 1.0),
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wallpaper_decodes_landscape_and_is_named_once() {
        for w in Wallpaper::ALL {
            let img = w.image(0).unwrap_or_else(|| panic!("{w:?} does not decode"));
            assert_eq!((img.size(0).width.0, img.size(0).height.0), (3200, 1800), "{w:?}");
            assert_eq!(w.size(), Some((3200, 1800)), "{w:?}");
            assert_eq!(serde_json::to_string(&w).unwrap(), format!("\"{}\"", w.id()));
        }
        let half = Wallpaper::Halo.image(1).unwrap();
        assert_eq!((half.size(0).width.0, half.size(0).height.0), (1600, 900));
        assert_eq!((Wallpaper::DARK, Wallpaper::LIGHT), (Wallpaper::Halo, Wallpaper::ToriiDither));
    }

    /// A picture is drawn from its smallest halving at least as wide as it is drawn: never shrunk to less than half.
    #[test]
    fn the_level_is_the_halving_just_above_the_drawn_size() {
        assert_eq!(level(3200, 5120.0), 0);
        assert_eq!(level(3200, 2088.0), 0);
        assert_eq!(level(3200, 1600.0), 1);
        assert_eq!(level(3200, 1044.0), 1);
        assert_eq!(level(3200, 764.0), 2);
        assert_eq!(level(3200, 356.0), 3);
        assert_eq!(level(3200, 52.0), LEVELS - 1);
        assert_eq!(level(780, 1044.0), 0);
    }

    /// The picture covers its box and keeps its point in view: a wide box crops the top and bottom, from 30% down.
    #[test]
    fn the_cover_keeps_the_point_asked() {
        let b = Bounds::new(point(px(0.0), px(0.0)), size(px(1040.0), px(360.0)));
        let f = cover(b, 3200, 1800, (0.5, 0.3)).unwrap();
        assert!((f.size.width.as_f32() - 1040.0).abs() < 0.01);
        assert!(f.size.height.as_f32() > 360.0);
        let over = f.size.height.as_f32() - 360.0;
        assert!((f.origin.y.as_f32() + over * 0.3).abs() < 0.01);
        assert!(cover(b, 0, 10, (0.5, 0.5)).is_none());
    }
}
