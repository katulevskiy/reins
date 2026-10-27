import CoreGraphics
import SwiftUI
import UIKit

/// Turns an image (a shipped wallpaper or a photo) into a wallpaper, all on the phone: filled to the wallpaper's 780 ×
/// 1000 (390 × 500 pt), aligned to its top, treated with an effect (`WallpaperEffects`, the same pixel code as
/// Android's), and faded into the page's paper at the bottom. Pure: no state, safe off the main thread.
enum WallpaperRender {
    static let size = CGSize(width: 780, height: 1000)
    /// The fade a new photo opens with (a shipped image has its own, and adds none by default).
    static let defaultFade = 0.65

    /// The page's paper the fade runs into, in 0...255.
    static func paper(dark: Bool) -> (r: UInt8, g: UInt8, b: UInt8) { WallpaperEffects.paper(dark: dark) }

    /// `source` with `effect` and `fade`, for a theme; `foot`: where `source` already fades into the page.
    static func render(
        _ source: CGImage, effect: WallpaperEffect, fade: Double, dark: Bool, size: CGSize = size, foot: WallpaperEffects.Foot? = nil
    ) -> CGImage? {
        let w = max(1, Int(size.width)), h = max(1, Int(size.height))
        guard var px = Pixels(fill: source, width: w, height: h) else { return nil }
        WallpaperEffects.apply(effect, to: &px, dark: dark, foot: foot)
        // An image with no foot of its own (your photo) still meets the paper under an effect, which can turn its
        // bottom pure black: the art never ends on a hard line. Original keeps the bottom as chosen.
        let minimumFoot = effect == .original || foot != nil ? 0 : effectFoot
        fadeOut(&px, amount: fade, dark: dark, minimumFoot: minimumFoot)
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

    /// The share of the height at the bottom that an effect fades into the paper on an image with no foot of its own.
    static let effectFoot = 0.2

    /// Where the fade starts, as a share of the height from the top: later with less fade, none at 0.
    static func fadeStart(_ amount: Double) -> Double { 1 - 0.7 * min(1, max(0, amount)) }

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

/// Pixels from and to Core Graphics.
extension Pixels {
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

    func image() -> CGImage? {
        let data = Data(bytes) as CFData
        guard let provider = CGDataProvider(data: data) else { return nil }
        return CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
            provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent
        )
    }
}
