import CoreGraphics
import CoreImage
import SwiftUI
import UIKit

/// The treatments any wallpaper can take, a shipped one or your photo (the Settings board, 04c), as the shipped
/// wallpapers were treated. Original leaves the image as it is.
enum WallpaperEffect: String, CaseIterable, Identifiable {
    case original, dither, scanlines, halftone, colour

    var id: String { rawValue }

    var title: String {
        switch self {
        case .original: "Original"
        case .dither: "Dither"
        case .scanlines: "Scanlines"
        case .halftone: "Halftone"
        case .colour: "Colour"
        }
    }
}

/// Turns an image (a shipped wallpaper or a photo) into a wallpaper, all on the phone: filled to the wallpaper's 780 × 1000 (390 × 500 pt), aligned to
/// its top, treated with an effect (plain pixel code, and Core Image for the colour one), and faded into the page's
/// paper at the bottom (the fade is baked, as in the shipped art). Pure: no state, safe off the main thread.
enum WallpaperRender {
    static let size = CGSize(width: 780, height: 1000)
    /// The fade a new photo opens with (a shipped image has its own, and adds none by default).
    static let defaultFade = 0.65

    /// The page's paper the fade runs into, in 0...255.
    static func paper(dark: Bool) -> (r: UInt8, g: UInt8, b: UInt8) { dark ? (0x12, 0x12, 0x15) : (0xF6, 0xF6, 0xF8) }

    static func render(_ source: CGImage, effect: WallpaperEffect, fade: Double, dark: Bool, size: CGSize = size) -> CGImage? {
        let w = max(1, Int(size.width)), h = max(1, Int(size.height))
        guard var px = Pixels(fill: source, width: w, height: h) else { return nil }
        switch effect {
        case .original: break
        case .dither: dither(&px, dark: dark)
        case .scanlines: scanlines(&px)
        case .halftone: halftone(&px, dark: dark)
        case .colour: colour(&px)
        }
        // An effect redraws the whole image, a shipped one's baked fade too (Dither turns it pure black): its foot
        // always runs into the paper, so the art never ends on a hard line. Original keeps the bottom as chosen.
        fadeOut(&px, amount: fade, dark: dark, minimumFoot: effect == .original ? 0 : effectFoot)
        return px.image()
    }

    /// A photo the right way up and no bigger than `maxSide`, as the effects expect it: what is kept of your photo.
    static func upright(_ image: UIImage, maxSide: CGFloat) -> CGImage? {
        let scale = min(1, maxSide / max(image.size.width, image.size.height))
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }.cgImage
    }

    /// The share of the height at the bottom that an effect always fades into the paper, whatever the added fade.
    static let effectFoot = 0.2

    /// Where the fade starts, as a share of the height from the top: later with less fade, none at 0.
    static func fadeStart(_ amount: Double) -> Double { 1 - 0.7 * min(1, max(0, amount)) }

    // MARK: Effects

    /// Ordered (Bayer 4 × 4) dither on 3 px cells: light dots on black for dark, ink dots on paper for light.
    static func dither(_ px: inout Pixels, dark: Bool) {
        let bayer: [Double] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map { ($0 + 0.5) / 16 }
        let cell = max(2, px.width / 260)
        let on: (UInt8, UInt8, UInt8) = dark ? (0xE4, 0xE8, 0xFF) : (0x2A, 0x2A, 0x33)
        let off: (UInt8, UInt8, UInt8) = dark ? (0, 0, 0) : paper(dark: false)
        for cy in stride(from: 0, to: px.height, by: cell) {
            for cx in stride(from: 0, to: px.width, by: cell) {
                var lum = px.averageLuminance(x: cx, y: cy, w: cell, h: cell)
                // A little more contrast, as the shipped dithers have.
                lum = min(1, max(0, (pow(lum, 0.8) - 0.5) * 1.15 + 0.5))
                let t = bayer[((cy / cell) % 4) * 4 + (cx / cell) % 4]
                let lit = dark ? lum > t : lum < t
                px.fill(x: cx, y: cy, w: cell, h: cell, lit ? on : off)
            }
        }
    }

    /// The photo with every other 2 px band darkened, a little more contrast.
    static func scanlines(_ px: inout Pixels) {
        for y in 0..<px.height {
            let dim = (y / 2) % 2 == 1
            for x in 0..<px.width {
                let i = px.index(x, y)
                for c in 0..<3 {
                    let v = (Double(px.bytes[i + c]) / 255 - 0.5) * 1.15 + 0.5
                    px.bytes[i + c] = UInt8(clamping: Int((min(1, max(0, v)) * (dim ? 0.38 : 1)) * 255))
                }
            }
        }
    }

    /// Round dots on a 10 px grid, as big as the cell is light (dark) or dark (light), in the photo's colour warmed.
    static func halftone(_ px: inout Pixels, dark: Bool) {
        let cell = 10
        var cells: [(x: Int, y: Int, lum: Double, rgb: (Double, Double, Double))] = []
        for cy in stride(from: 0, to: px.height, by: cell) {
            for cx in stride(from: 0, to: px.width, by: cell) {
                cells.append((cx, cy, px.averageLuminance(x: cx, y: cy, w: cell, h: cell), px.averageColour(x: cx, y: cy, w: cell, h: cell)))
            }
        }
        let back: (UInt8, UInt8, UInt8) = dark ? (0x0B, 0x0A, 0x09) : paper(dark: false)
        px.fill(x: 0, y: 0, w: px.width, h: px.height, back)
        for c in cells {
            let amount = dark ? c.lum : 1 - c.lum
            let r = Double(cell) * 0.62 * amount.squareRoot()
            guard r > 0.4 else { continue }
            // Warm: towards a cream for dark, towards a sepia ink for light.
            let tone: (Double, Double, Double) = dark ? (1, 0.92, 0.78) : (0.32, 0.24, 0.16)
            let mix = 0.55
            let colour = (
                UInt8(clamping: Int((c.rgb.0 * (1 - mix) + tone.0 * mix) * 255)),
                UInt8(clamping: Int((c.rgb.1 * (1 - mix) + tone.1 * mix) * 255)),
                UInt8(clamping: Int((c.rgb.2 * (1 - mix) + tone.2 * mix) * 255))
            )
            px.disc(cx: Double(c.x) + Double(cell) / 2, cy: Double(c.y) + Double(cell) / 2, r: r, colour)
        }
    }

    /// Richer colour in fewer steps: Core Image's colour controls, then a posterize.
    static func colour(_ px: inout Pixels) {
        guard let image = px.image() else { return }
        var ci = CIImage(cgImage: image)
        if let controls = CIFilter(name: "CIColorControls") {
            controls.setValue(ci, forKey: kCIInputImageKey)
            controls.setValue(1.35, forKey: kCIInputSaturationKey)
            controls.setValue(1.05, forKey: kCIInputContrastKey)
            ci = controls.outputImage ?? ci
        }
        if let poster = CIFilter(name: "CIColorPosterize") {
            poster.setValue(ci, forKey: kCIInputImageKey)
            poster.setValue(12, forKey: "inputLevels")
            ci = poster.outputImage ?? ci
        }
        let context = CIContext(options: [.cacheIntermediates: false])
        guard let out = context.createCGImage(ci, from: CGRect(x: 0, y: 0, width: px.width, height: px.height)),
              let treated = Pixels(fill: out, width: px.width, height: px.height) else { return }
        px = treated
    }

    /// Blends the bottom into the page's paper, from `fadeStart(amount)` down (and at least the bottom `minimumFoot`),
    /// eased.
    static func fadeOut(_ px: inout Pixels, amount: Double, dark: Bool, minimumFoot: Double = 0) {
        let start = min(fadeStart(amount), 1 - min(1, max(0, minimumFoot))) * Double(px.height)
        let end = Double(px.height)
        let p = paper(dark: dark)
        let target = [Double(p.r), Double(p.g), Double(p.b)]
        for y in Int(start)..<px.height {
            let t = min(1, max(0, (Double(y) - start) / max(1, end - start)))
            let k = t * t * (3 - 2 * t)
            for x in 0..<px.width {
                let i = px.index(x, y)
                for c in 0..<3 {
                    px.bytes[i + c] = UInt8(clamping: Int(Double(px.bytes[i + c]) * (1 - k) + target[c] * k))
                }
            }
        }
    }
}

/// An RGBA image as bytes (opaque, 8 bits a channel), for the effects' pixel code.
struct Pixels {
    var width: Int
    var height: Int
    var bytes: [UInt8]

    init(width: Int, height: Int, bytes: [UInt8]) {
        self.width = width
        self.height = height
        self.bytes = bytes
    }

    /// `source` drawn to fill `width` × `height`, centred across and aligned to the top (a portrait's face stays).
    init?(fill source: CGImage, width: Int, height: Int) {
        guard width > 0, height > 0, source.width > 0, source.height > 0 else { return nil }
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        let scale = max(Double(width) / Double(source.width), Double(height) / Double(source.height))
        let dw = Double(source.width) * scale, dh = Double(source.height) * scale
        let drawn = bytes.withUnsafeMutableBytes { raw -> Bool in
            guard let context = CGContext(
                data: raw.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
            ) else { return false }
            context.interpolationQuality = .high
            // Core Graphics counts from the bottom: the top of the photo at the top of the frame.
            context.draw(source, in: CGRect(x: (Double(width) - dw) / 2, y: Double(height) - dh, width: dw, height: dh))
            return true
        }
        guard drawn else { return nil }
        self.init(width: width, height: height, bytes: bytes)
    }

    func index(_ x: Int, _ y: Int) -> Int { (y * width + x) * 4 }

    func image() -> CGImage? {
        let data = Data(bytes) as CFData
        guard let provider = CGDataProvider(data: data) else { return nil }
        return CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
            provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent
        )
    }

    func averageColour(x: Int, y: Int, w: Int, h: Int) -> (Double, Double, Double) {
        var r = 0.0, g = 0.0, b = 0.0, n = 0.0
        for yy in y..<min(height, y + h) {
            for xx in x..<min(width, x + w) {
                let i = index(xx, yy)
                r += Double(bytes[i]); g += Double(bytes[i + 1]); b += Double(bytes[i + 2]); n += 1
            }
        }
        guard n > 0 else { return (0, 0, 0) }
        return (r / n / 255, g / n / 255, b / n / 255)
    }

    /// Rec. 709 luminance, 0...1.
    func averageLuminance(x: Int, y: Int, w: Int, h: Int) -> Double {
        let c = averageColour(x: x, y: y, w: w, h: h)
        return 0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2
    }

    mutating func fill(x: Int, y: Int, w: Int, h: Int, _ c: (UInt8, UInt8, UInt8)) {
        for yy in max(0, y)..<min(height, y + h) {
            for xx in max(0, x)..<min(width, x + w) {
                let i = index(xx, yy)
                bytes[i] = c.0; bytes[i + 1] = c.1; bytes[i + 2] = c.2; bytes[i + 3] = 255
            }
        }
    }

    /// A filled disc, its edge softened over a pixel.
    mutating func disc(cx: Double, cy: Double, r: Double, _ c: (UInt8, UInt8, UInt8)) {
        let x0 = max(0, Int(cx - r - 1)), x1 = min(width - 1, Int(cx + r + 1))
        let y0 = max(0, Int(cy - r - 1)), y1 = min(height - 1, Int(cy + r + 1))
        guard x0 <= x1, y0 <= y1 else { return }
        for yy in y0...y1 {
            for xx in x0...x1 {
                let dx = Double(xx) + 0.5 - cx, dy = Double(yy) + 0.5 - cy
                let cover = min(1, max(0, r - (dx * dx + dy * dy).squareRoot() + 0.5))
                guard cover > 0 else { continue }
                let i = index(xx, yy)
                bytes[i] = UInt8(Double(bytes[i]) * (1 - cover) + Double(c.0) * cover)
                bytes[i + 1] = UInt8(Double(bytes[i + 1]) * (1 - cover) + Double(c.1) * cover)
                bytes[i + 2] = UInt8(Double(bytes[i + 2]) * (1 - cover) + Double(c.2) * cover)
            }
        }
    }
}
