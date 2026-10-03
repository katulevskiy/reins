//! A QR code as a GPUI element: dark modules painted as quads, one per horizontal run, on whole pixels so the edges
//! stay sharp. A quiet zone of two modules on every side.

use gpui::{Bounds, Hsla, IntoElement, Styled as _, canvas, fill, point, px, size};
use qrcode::{Color, EcLevel, QrCode};

const QUIET: usize = 2;

/// The modules of `data` (true is dark), row by row, and the side length; `None` when it is too long for a QR code.
#[must_use]
pub fn modules(data: &str) -> Option<(Vec<bool>, usize)> {
    let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::M).ok()?;
    let width = code.width();
    Some((code.to_colors().into_iter().map(|c| c == Color::Dark).collect(), width))
}

/// `data` as a QR code `side` pixels wide, dark on `light`.
pub fn element(data: &str, side: f32, dark: Hsla, light: Hsla) -> impl IntoElement {
    let code = modules(data);
    canvas(
        move |_, _, _| code,
        move |bounds, code, window, _| {
            window.paint_quad(fill(bounds, light));
            let Some((dark_modules, width)) = code else {
                return;
            };
            let n = width + 2 * QUIET;
            // Whole pixels per module, the code centred in what is left.
            let unit = (f32::from(bounds.size.width) / f32_of(n)).floor().max(1.0);
            let offset = (f32::from(bounds.size.width) - unit * f32_of(n)) / 2.0;
            let origin = bounds.origin + point(px(offset.floor()), px(offset.floor()));
            for (y, row) in dark_modules.chunks(width).enumerate() {
                let mut x = 0;
                while x < width {
                    if !row[x] {
                        x += 1;
                        continue;
                    }
                    let start = x;
                    while x < width && row[x] {
                        x += 1;
                    }
                    let quad = Bounds::new(
                        origin + point(px(unit * f32_of(start + QUIET)), px(unit * f32_of(y + QUIET))),
                        size(px(unit * f32_of(x - start)), px(unit)),
                    );
                    window.paint_quad(fill(quad, dark));
                }
            }
        },
    )
    .size(px(side))
}

/// A module count as `f32` (QR codes are at most 177 modules wide).
fn f32_of(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pairing_link_fits_and_has_finder_patterns() {
        let (dark, width) = modules("https://app.reins2fa.com/device?user_code=WDJB-MJHT").unwrap();
        assert_eq!(dark.len(), width * width);
        assert!(width >= 21);
        // The top-left finder pattern: a dark 7x7 ring.
        assert!((0..7).all(|i| dark[i] && dark[6 * width + i] && dark[i * width] && dark[i * width + 6]));
        assert!(!dark[width + 1]);
    }
}
