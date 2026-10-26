//! A field: a solid surface of a [`Field`]'s colour, lit and grained so it reads as painted.
//!
//! Next's fields wear the boards' "Aurora" texture (`.tx-aurora .grain`): three soft lights in the field's own box (a
//! white one at the top right, a shade at the bottom left, a warm glow up left) and a grey noise (`grain.png`, 360 px,
//! shown at 180), all blended over the field with `soft-light`. GPUI has no blend modes and no image tiling, so at
//! start [`init`] bakes, for each field colour, what the blend makes of it: the lights as one small picture that is
//! stretched over the field (they are sized in the field's box, so they stretch with it), and the noise as a tile,
//! both see-through where the texture leaves the field alone. A field paints its colour, its lights, then its tile
//! across itself, 180 px a tile, clipped to its corners. Off under Reduce Transparency and with Settings' Grain on
//! fields off (a flat field).

use std::panic::Location;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use gpui::Refineable as _;
use gpui::{
    App, Bounds, Corners, Div, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    ParentElement as _, Pixels, RenderImage, Style, StyleRefinement, Styled, Window, div, point, px, size,
};

use crate::theme::{Field, motion};
use crate::ui::icons::GRAIN_PNG;

/// The size a tile is drawn at, in px (the noise is 360 px: sharp on a 2× screen).
pub const TILE: f32 = 180.0;
/// A field's bottom corners.
pub const FIELD_RADIUS: f32 = 30.0;

/// The baked tiles and lights, one per field (in [`Field::ALL`]'s order).
struct Grain {
    tiles: [Arc<RenderImage>; 7],
    lights: [Arc<RenderImage>; 7],
}

/// The size the lights are baked at (they are smooth: stretched, they stay smooth).
const LIGHTS: (u32, u32) = (192, 96);

static GRAIN: OnceLock<Option<Grain>> = OnceLock::new();

/// Grain on fields, as Settings says (on unless turned off).
static GRAIN_ON: AtomicBool = AtomicBool::new(true);

/// Turns the grain on fields on or off (Settings' Grain on fields).
pub fn set_grain(on: bool) {
    GRAIN_ON.store(on, Ordering::Relaxed);
}

/// Bakes the tiles, once; later calls do nothing. Without them (the noise did not decode) fields are flat.
pub fn init() {
    GRAIN.get_or_init(|| match bake_all() {
        Ok(g) => Some(g),
        Err(e) => {
            log::warn!("no grain on fields: {e}");
            None
        }
    });
}

fn bake_all() -> Result<Grain, String> {
    let noise = crate::tray::decode(GRAIN_PNG)?;
    let image = |w: u32, h: u32, bgra: Vec<u8>| -> Result<Arc<RenderImage>, String> {
        let buffer = image::RgbaImage::from_raw(w, h, bgra).ok_or("the texture's size")?;
        Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
    };
    let tile = |f: Field| image(noise.width, noise.height, bake(f.rgb(), &noise.rgba));
    let light = |f: Field| image(LIGHTS.0, LIGHTS.1, bake_lights(f.rgb(), LIGHTS.0, LIGHTS.1));
    Ok(Grain {
        tiles: [
            tile(Field::Push)?,
            tile(Field::Read)?,
            tile(Field::Delete)?,
            tile(Field::Grant)?,
            tile(Field::Pair)?,
            tile(Field::Accounts)?,
            tile(Field::Warn)?,
        ],
        lights: [
            light(Field::Push)?,
            light(Field::Read)?,
            light(Field::Delete)?,
            light(Field::Grant)?,
            light(Field::Pair)?,
            light(Field::Accounts)?,
            light(Field::Warn)?,
        ],
    })
}

/// One of the Aurora's lights: `radial-gradient(rx% ry% at cx% cy%, rgba(color, alpha), transparent end%)`.
struct Light {
    at: (f32, f32),
    radii: (f32, f32),
    color: [f32; 3],
    alpha: f32,
    end: f32,
}

/// The lights, bottom first (`.tx-aurora .grain`, whose first gradient is on top).
const AURORA: [Light; 3] = [
    Light {
        at: (0.3, 0.2),
        radii: (0.5, 0.6),
        color: [1.0, 220.0 / 255.0, 180.0 / 255.0],
        alpha: 0.35,
        end: 0.7,
    },
    Light {
        at: (0.0, 1.0),
        radii: (0.6, 0.8),
        color: [0.0, 0.0, 0.0],
        alpha: 0.45,
        end: 0.7,
    },
    Light {
        at: (0.92, 0.0),
        radii: (0.7, 0.9),
        color: [1.0, 1.0, 1.0],
        alpha: 0.7,
        end: 0.6,
    },
];

/// The lights blended soft-light over the colour `rgb`, `w` × `h` over the field's box, as BGRA with straight alpha:
/// each pixel what the blend makes of the field there, as opaque as the lights are.
#[must_use]
pub fn bake_lights(rgb: u32, width: u32, height: u32) -> Vec<u8> {
    let field = channels(rgb);
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for col in 0..width {
            #[allow(clippy::cast_precision_loss, reason = "a small picture")]
            let (across, down) = ((col as f32 + 0.5) / width as f32, (row as f32 + 0.5) / height as f32);
            // The lights composited over nothing (source-over, premultiplied).
            let (mut c, mut a) = ([0.0f32; 3], 0.0f32);
            for l in &AURORA {
                let t = ((across - l.at.0) / l.radii.0).hypot((down - l.at.1) / l.radii.1);
                let la = l.alpha * (1.0 - t / l.end).clamp(0.0, 1.0);
                for (ch, lc) in c.iter_mut().zip(l.color) {
                    *ch = lc.mul_add(la, *ch * (1.0 - la));
                }
                a = la + a * (1.0 - la);
            }
            let mut px = [0u8; 4];
            for i in 0..3 {
                let cs = if a > 0.0 {
                    c[i] / a
                } else {
                    0.5
                };
                px[2 - i] = byte(soft_light(field[i], cs));
            }
            px[3] = byte(a);
            out.extend_from_slice(&px);
        }
    }
    out
}

/// `rgb`'s three channels, 0…1.
fn channels(rgb: u32) -> [f32; 3] {
    let channel = |shift: u32| f32::from(u8::try_from((rgb >> shift) & 0xFF).unwrap_or(0)) / 255.0;
    [channel(16), channel(8), channel(0)]
}

/// 0…1 as a byte.
fn byte(v: f32) -> u8 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "0…255 after the clamp")]
    let b = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    b
}

/// W3C `soft-light`: the backdrop `cb` lit by the source `cs` (both 0…1).
#[must_use]
pub fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        ((2.0f32.mul_add(cs, -1.0)) * cb).mul_add(1.0 - cb, cb)
    } else {
        let d = if cb <= 0.25 {
            16.0f32.mul_add(cb, -12.0).mul_add(cb, 4.0) * cb
        } else {
            cb.sqrt()
        };
        2.0f32.mul_add(cs, -1.0).mul_add(d - cb, cb)
    }
}

/// The noise (`rgba`, straight alpha) blended soft-light over the colour `rgb`, as BGRA (GPUI's order) with the
/// noise's own alpha: painted over the field, it gives what the blend gives.
#[must_use]
pub fn bake(rgb: u32, rgba: &[u8]) -> Vec<u8> {
    let field = channels(rgb);
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        let mut rgb_out = [0u8; 3];
        for (i, cb) in field.iter().copied().enumerate() {
            rgb_out[i] = byte(soft_light(cb, f32::from(px[i]) / 255.0));
        }
        out.extend_from_slice(&[rgb_out[2], rgb_out[1], rgb_out[0], px[3]]);
    }
    out
}

fn tile_of(field: Field) -> Option<Arc<RenderImage>> {
    let grain = GRAIN.get()?.as_ref()?;
    let i = Field::ALL.iter().position(|f| *f == field)?;
    Some(Arc::clone(&grain.tiles[i]))
}

fn lights_of(field: Field) -> Option<Arc<RenderImage>> {
    let grain = GRAIN.get()?.as_ref()?;
    let i = Field::ALL.iter().position(|f| *f == field)?;
    Some(Arc::clone(&grain.lights[i]))
}

/// A field's surface: its colour, its grain, its corners (`top` and `bottom` radii; a field that runs into the top of
/// the window has `top` 0 and `bottom` [`FIELD_RADIUS`]). Children go on top, in `on-field` colours.
pub fn field_surface(field: Field, top: f32, bottom: f32) -> Div {
    div()
        .relative()
        .overflow_hidden()
        .bg(field.field())
        .rounded_tl(px(top))
        .rounded_tr(px(top))
        .rounded_bl(px(bottom))
        .rounded_br(px(bottom))
        .child(grain(field, top, bottom))
}

/// The grain alone, filling its (positioned) parent.
pub fn grain(field: Field, top: f32, bottom: f32) -> GrainLayer {
    let mut layer = GrainLayer {
        field,
        corners: Corners {
            top_left: px(top),
            top_right: px(top),
            bottom_right: px(bottom),
            bottom_left: px(bottom),
        },
        style: StyleRefinement::default(),
    };
    layer = layer.absolute().top_0().left_0().size_full();
    layer
}

/// The element that paints a field's grain: no state, no allocation as it paints (the baked tile is shared).
pub struct GrainLayer {
    field: Field,
    corners: Corners<Pixels>,
    style: StyleRefinement,
}

impl Styled for GrainLayer {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for GrainLayer {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for GrainLayer {
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
        if motion::reduce_transparency() || !GRAIN_ON.load(Ordering::Relaxed) {
            return;
        }
        let Some(tile) = tile_of(self.field) else {
            return;
        };
        if let Some(lights) = lights_of(self.field)
            && let Err(e) = window.paint_image(bounds, self.corners, lights, 0, false)
        {
            log::debug!("lights: {e}");
        }
        let (cols, x0) = grid(bounds.size.width.as_f32());
        let (rows, y0) = grid(bounds.size.height.as_f32());
        let (left, top) = (bounds.origin.x.as_f32(), bounds.origin.y.as_f32());
        let (right, bottom) = (left + bounds.size.width.as_f32(), top + bounds.size.height.as_f32());
        for row in 0..rows {
            for col in 0..cols {
                // The tile's box on the grid, and the part of it inside the field.
                #[allow(clippy::cast_precision_loss, reason = "a few tiles")]
                let (fx, fy) = (left + x0 + col as f32 * TILE, top + y0 + row as f32 * TILE);
                let (vx, vy) = (fx.max(left), fy.max(top));
                let (vr, vb) = ((fx + TILE).min(right), (fy + TILE).min(bottom));
                if vr <= vx || vb <= vy {
                    continue;
                }
                let fitted = Bounds::new(point(px(fx), px(fy)), size(px(TILE), px(TILE)));
                let visible = Bounds::new(point(px(vx), px(vy)), size(px(vr - vx), px(vb - vy)));
                // A tile at a corner of the field takes that corner's rounding.
                let (first_row, last_row, first_col, last_col) = (row == 0, row + 1 == rows, col == 0, col + 1 == cols);
                let at = |yes: bool, r: Pixels| {
                    if yes {
                        r
                    } else {
                        px(0.0)
                    }
                };
                let corners = Corners {
                    top_left: at(first_row && first_col, self.corners.top_left),
                    top_right: at(first_row && last_col, self.corners.top_right),
                    bottom_right: at(last_row && last_col, self.corners.bottom_right),
                    bottom_left: at(last_row && first_col, self.corners.bottom_left),
                };
                if let Err(e) = window.paint_image_fitted(visible, fitted, corners, Arc::clone(&tile), 0, false) {
                    log::debug!("grain: {e}");
                    return;
                }
            }
        }
    }
}

/// How many tiles cross `extent`, and where the first starts: centred, so the two edge tiles are cut alike and are
/// never thinner than half a tile (a corner's rounding always fits in its tile).
fn grid(extent: f32) -> (usize, f32) {
    if extent <= 0.0 {
        return (0, 0.0);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a positive count of tiles")]
    let n = (extent / TILE).ceil().max(1.0) as usize;
    #[allow(clippy::cast_precision_loss, reason = "a few tiles")]
    let over = n as f32 * TILE - extent;
    (n, -over / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_light_keeps_grey_neutral_and_lights_or_darkens() {
        for cb in [0.1, 0.3, 0.5, 0.9] {
            assert!((soft_light(cb, 0.5) - cb).abs() < 1e-6);
            assert!(soft_light(cb, 1.0) >= cb && soft_light(cb, 0.0) <= cb);
        }
    }

    /// The grain bakes: a 360 px opaque tile for every field, close to the flat colour (contrast within the design's
    /// 0.1) but not flat.
    #[test]
    fn the_grain_bakes() {
        let noise = crate::tray::decode(GRAIN_PNG).unwrap();
        assert_eq!((noise.width, noise.height), (360, 360));
        for f in Field::ALL {
            let bgra = bake(f.rgb(), &noise.rgba);
            assert_eq!(bgra.len(), 360 * 360 * 4);
            // The tile keeps the noise's alpha, and painted over the field stays close to its colour.
            assert!(bgra.as_chunks::<4>().0.iter().zip(noise.rgba.as_chunks::<4>().0).all(|(p, n)| p[3] == n[3]));
            let over = |c: usize, shift: u32| {
                let flat = f64::from((f.rgb() >> shift) & 0xFF);
                #[allow(clippy::cast_precision_loss, reason = "a mean of bytes")]
                let m = bgra
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| {
                        let a = f64::from(p[3]) / 255.0;
                        f64::from(p[c]).mul_add(a, flat * (1.0 - a))
                    })
                    .sum::<f64>()
                    / (360.0 * 360.0);
                (m, flat)
            };
            for (c, shift) in [(2, 16), (1, 8), (0, 0)] {
                let (m, flat) = over(c, shift);
                assert!((m - flat).abs() < 0.1 * 255.0, "{f:?} drifts from its colour");
            }
            let first = &bgra[..4];
            assert!(bgra.as_chunks::<4>().0.iter().any(|p| p[..] != *first), "{f:?} is flat");
        }
        init();
        assert!(Field::ALL.iter().all(|f| tile_of(*f).is_some() && lights_of(*f).is_some()));
    }

    /// The Aurora lights the top right, shades the bottom left, and leaves the far side of the field alone.
    #[test]
    fn the_lights_light_the_top_right() {
        let (w, h) = (64, 32);
        let px = bake_lights(Field::Push.rgb(), w, h);
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            (px[i + 2], px[i + 1], px[i], px[i + 3])
        };
        let (r_lit, _, _, a_lit) = at(w - 6, 0);
        let (r_dark, _, _, a_dark) = at(0, h - 1);
        let field_r = u8::try_from((Field::Push.rgb() >> 16) & 0xFF).unwrap();
        assert!(a_lit > 100 && r_lit > field_r, "the top right is lit");
        assert!(a_dark > 60 && r_dark < field_r, "the bottom left is shaded");
        assert_eq!(at(w / 2 + 6, h - 1).3, 0, "the bottom middle is the field's own colour");
    }

    #[test]
    fn tiles_centre_and_never_get_thin() {
        assert_eq!(grid(0.0), (0, 0.0));
        assert_eq!(grid(180.0), (1, 0.0));
        let (n, x0) = grid(181.0);
        assert_eq!(n, 2);
        assert!(x0 + TILE >= 90.0);
        for extent in [30.0, 200.0, 361.0, 700.0, 1279.0] {
            let (n, x0) = grid(extent);
            #[allow(clippy::cast_precision_loss, reason = "a few tiles")]
            let covered = n as f32 * TILE;
            assert!(x0 <= 0.0 && x0 + covered >= extent - 0.01);
            if n > 1 {
                assert!(x0 + TILE >= TILE / 2.0 - 0.01, "{extent}");
            }
        }
    }
}
