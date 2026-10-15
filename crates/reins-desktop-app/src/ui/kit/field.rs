//! A field: a solid surface of a [`Field`]'s colour, with the grain over it so it reads as painted.
//!
//! The design blends a grey noise (`grain.png`, 360 px, shown at 180) over the field with `soft-light`. GPUI has no
//! blend modes and no image tiling, so at start [`init`] decodes the noise once and bakes one tile per field colour
//! with the soft-light formula; a field then paints its baked tile across itself, 180 px a tile, clipped to its
//! corners. Off under Reduce Transparency (a flat field).

use std::panic::Location;
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

/// The baked tiles, one per field (in [`Field::ALL`]'s order).
struct Grain {
    tiles: [Arc<RenderImage>; 7],
}

static GRAIN: OnceLock<Option<Grain>> = OnceLock::new();

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
    let tile = |f: Field| -> Result<Arc<RenderImage>, String> {
        let bgra = bake(f.rgb(), &noise.rgba);
        let buffer = image::RgbaImage::from_raw(noise.width, noise.height, bgra).ok_or("the grain's size")?;
        Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
    };
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
    })
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

/// The noise (`rgba`, straight alpha) blended soft-light over the colour `rgb`, opaque, as BGRA (GPUI's order).
#[must_use]
pub fn bake(rgb: u32, rgba: &[u8]) -> Vec<u8> {
    let channel = |shift: u32| f32::from(u8::try_from((rgb >> shift) & 0xFF).unwrap_or(0)) / 255.0;
    let field = [channel(16), channel(8), channel(0)];
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        let alpha = f32::from(px[3]) / 255.0;
        let mut rgb_out = [0u8; 3];
        for (i, cb) in field.iter().copied().enumerate() {
            let cs = f32::from(px[i]) / 255.0;
            let blended = (soft_light(cb, cs) - cb).mul_add(alpha, cb);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "0…255 after the clamp")]
            let byte = (blended.clamp(0.0, 1.0) * 255.0).round() as u8;
            rgb_out[i] = byte;
        }
        out.extend_from_slice(&[rgb_out[2], rgb_out[1], rgb_out[0], 255]);
    }
    out
}

fn tile_of(field: Field) -> Option<Arc<RenderImage>> {
    let grain = GRAIN.get()?.as_ref()?;
    let i = Field::ALL.iter().position(|f| *f == field)?;
    Some(Arc::clone(&grain.tiles[i]))
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
        if motion::reduce_transparency() {
            return;
        }
        let Some(tile) = tile_of(self.field) else {
            return;
        };
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
            assert!(bgra.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
            let mean = |c: usize| {
                #[allow(clippy::cast_precision_loss, reason = "a mean of bytes")]
                let m = bgra.as_chunks::<4>().0.iter().map(|p| f64::from(p[c])).sum::<f64>() / (360.0 * 360.0);
                m
            };
            let flat = |shift: u32| f64::from((f.rgb() >> shift) & 0xFF);
            for (c, shift) in [(2, 16), (1, 8), (0, 0)] {
                assert!((mean(c) - flat(shift)).abs() < 0.1 * 255.0, "{f:?} drifts from its colour");
            }
            let first = &bgra[..4];
            assert!(bgra.as_chunks::<4>().0.iter().any(|p| p[..] != *first), "{f:?} is flat");
        }
        init();
        assert!(Field::ALL.iter().all(|f| tile_of(*f).is_some()));
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
