import Foundation

/// The looks a wallpaper can take (the Settings board, 04c): any image, shipped or your photo (a treated image's
/// effects are drawn on its untreated original). Dither, Scanlines and Halftone keep the image's colours; Mono is the
/// boards' two-tone dither in the theme's light and dark. Original leaves the image as it is.
enum WallpaperEffect: String, CaseIterable, Identifiable {
    case original, dither, mono, scanlines, halftone

    var id: String { rawValue }

    var title: String {
        switch self {
        case .original: "Original"
        case .dither: "Dither"
        case .mono: "Mono"
        case .scanlines: "Scanlines"
        case .halftone: "Halftone"
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

    func index(_ x: Int, _ y: Int) -> Int { (y * width + x) * 4 }
}

/// The effects as pixel code, the Next boards' (design/next, `source/art/fx.py`). Android draws them with the same
/// steps and numbers (`PhotoEffects.kt`), so a wallpaper looks the same on both. Sizes are for the 780-wide art and
/// scale with the width, so a small preview looks like the wallpaper. Pure: safe off the main thread.
enum WallpaperEffects {
    /// The art's width the sizes are for.
    static let reference = 780.0

    /// The page's paper, in 0...255.
    static func paper(dark: Bool) -> (r: UInt8, g: UInt8, b: UInt8) { dark ? (0x12, 0x12, 0x15) : (0xF6, 0xF6, 0xF8) }

    /// Where a shipped image already fades into the page, as a share of its height from the top (the boards'
    /// `fade(start, end)`). An effect fades back into the image over it, so the foot stays smooth.
    struct Foot: Hashable, Sendable {
        var start: Double
        var end: Double
    }

    /// `effect` on `px`, in place; `foot`: where `px` already fades into the page.
    static func apply(_ effect: WallpaperEffect, to px: inout Pixels, dark: Bool, foot: Foot? = nil) {
        let plain = foot != nil && effect != .original ? px : nil
        switch effect {
        case .original: break
        case .dither: dither(&px)
        case .mono: mono(&px, dark: dark)
        case .scanlines: scanlines(&px)
        case .halftone: halftone(&px, dark: dark)
        }
        if let plain, let foot { footOut(&px, plain: plain, foot: foot) }
    }

    /// Blends `px` back into `plain` over the `foot`, eased, so the effect ends where the image fades out.
    static func footOut(_ px: inout Pixels, plain: Pixels, foot: Foot) {
        for y in 0..<px.height {
            let t = min(1, max(0, (Double(y) / Double(max(1, px.height - 1)) - foot.start) / (foot.end - foot.start)))
            guard t > 0 else { continue }
            let k = t * t * (3 - 2 * t)
            for x in 0..<px.width {
                let p = px.index(x, y)
                for c in 0..<3 { px.bytes[p + c] = byte(Double(px.bytes[p + c]) * (1 - k) + Double(plain.bytes[p + c]) * k) }
            }
        }
    }

    /// The boards' 8 × 8 Bayer matrix, in 64ths.
    private static let bayer: [Int] = [
        0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22,
        3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
    ]

    private static func threshold(_ col: Int, _ row: Int) -> Double { Double(bayer[(row % 8) * 8 + col % 8]) / 64 }

    /// A size on the 780-wide art, at this width: rounded, at least `least`.
    private static func scaled(_ n: Double, _ width: Int, least: Int = 1) -> Int {
        max(least, Int((n * Double(width) / reference + 0.5).rounded(.down)))
    }

    private static func byte(_ v: Double) -> UInt8 { UInt8(min(255, max(0, (v + 0.5).rounded(.down)))) }

    /// The image in square cells of `size` px: each cell's mean colour, 0...255 a channel (cells at the edges are cut).
    private struct Cells {
        var size: Int, cols: Int, rows: Int
        var r: [Double], g: [Double], b: [Double]

        /// Rec. 601 luma, 0...1, as the boards' greyscale.
        func luma(_ i: Int) -> Double { (0.299 * r[i] + 0.587 * g[i] + 0.114 * b[i]) / 255 }

        init(_ px: Pixels, size: Int) {
            self.size = size
            cols = (px.width + size - 1) / size
            rows = (px.height + size - 1) / size
            let count = cols * rows
            r = [Double](repeating: 0, count: count)
            g = r
            b = r
            var n = [Int](repeating: 0, count: count)
            for y in 0..<px.height {
                let row = (y / size) * cols
                for x in 0..<px.width {
                    let i = row + x / size, p = px.index(x, y)
                    r[i] += Double(px.bytes[p]); g[i] += Double(px.bytes[p + 1]); b[i] += Double(px.bytes[p + 2]); n[i] += 1
                }
            }
            for i in 0..<count {
                r[i] /= Double(n[i]); g[i] /= Double(n[i]); b[i] /= Double(n[i])
            }
        }
    }

    /// Paints each cell its colour from `colour` (by cell index).
    private static func paint(_ px: inout Pixels, _ cells: Cells, _ colour: (Int) -> (UInt8, UInt8, UInt8)) {
        let out = (0..<(cells.cols * cells.rows)).map(colour)
        for y in 0..<px.height {
            let row = (y / cells.size) * cells.cols
            for x in 0..<px.width {
                let c = out[row + x / cells.size], p = px.index(x, y)
                px.bytes[p] = c.0; px.bytes[p + 1] = c.1; px.bytes[p + 2] = c.2; px.bytes[p + 3] = 255
            }
        }
    }

    /// Ordered dither in colour (the boards' Ruins): 3 px cells, each channel stretched to its 1st...99th percentile,
    /// lifted 10 %, then dithered on its own to three levels (0, ½, 1) by the Bayer matrix: 27 colours.
    static func dither(_ px: inout Pixels) {
        var cells = Cells(px, size: scaled(3, px.width))
        autocontrast(&cells.r); autocontrast(&cells.g); autocontrast(&cells.b)
        func level(_ v: Double, _ t: Double) -> UInt8 {
            let q = min(1, ((min(1, max(0, v * 1.1 / 255)) * 2 + t).rounded(.down)) / 2)
            return byte(q * 255)
        }
        let c = cells
        paint(&px, c) { i in
            let t = threshold(i % c.cols, i / c.cols)
            return (level(c.r[i], t), level(c.g[i], t), level(c.b[i], t))
        }
    }

    /// Stretches a channel so its darkest and lightest 1 % reach the ends (PIL's `autocontrast(cutoff=1)`).
    private static func autocontrast(_ ch: inout [Double]) {
        var hist = [Int](repeating: 0, count: 256)
        for v in ch { hist[min(255, max(0, Int((v + 0.5).rounded(.down))))] += 1 }
        let cut = Double(ch.count) * 0.01
        var lo = 0, sum = 0
        while lo < 255 {
            sum += hist[lo]
            if Double(sum) > cut { break }
            lo += 1
        }
        var hi = 255
        sum = 0
        while hi > 0 {
            sum += hist[hi]
            if Double(sum) > cut { break }
            hi -= 1
        }
        guard hi > lo else { return }
        for i in ch.indices { ch[i] = (ch[i] - Double(lo)) * 255 / Double(hi - lo) }
    }

    /// The boards' two-tone dither (Halo, Torii dither): luma with more contrast about its mean, dithered by the Bayer
    /// matrix to the theme's light and dark. Dark: 3 px cells, 1.2 contrast, 0.9 gamma, blue-white on black. Light:
    /// 2 px cells, 1.4 contrast, the paper on a slate grey.
    static func mono(_ px: inout Pixels, dark: Bool) {
        let cells = Cells(px, size: dark ? scaled(3, px.width) : scaled(2, px.width))
        let contrast = dark ? 1.2 : 1.4, gamma = dark ? 0.9 : 1.0
        let on: (UInt8, UInt8, UInt8) = dark ? (232, 236, 255) : (246, 246, 248)
        let off: (UInt8, UInt8, UInt8) = dark ? (0, 0, 0) : (70, 70, 78)
        let n = cells.cols * cells.rows
        let mean = (0..<n).reduce(0.0) { $0 + cells.luma($1) } / Double(n)
        paint(&px, cells) { i in
            let g = pow(min(1, max(0, mean + (cells.luma(i) - mean) * contrast)), gamma)
            return g > threshold(i % cells.cols, i / cells.cols) ? on : off
        }
    }

    /// CRT lines over the image in its colours (the boards' Eyes): every 3rd row of 3 as it is, the other two at 60 %,
    /// red moved 2 px right and blue 2 px left.
    static func scanlines(_ px: inout Pixels) {
        let period = scaled(3, px.width, least: 2)
        let lit = max(1, period / 2)
        let shift = Int((2 * Double(px.width) / reference + 0.5).rounded(.down))
        let w = px.width
        var row = [UInt8](repeating: 0, count: w * 4)
        for y in 0..<px.height {
            let start = px.index(0, y)
            for i in 0..<(w * 4) { row[i] = px.bytes[start + i] }
            let m = y % period < lit ? 1.0 : 0.6
            for x in 0..<w {
                let p = start + x * 4
                px.bytes[p] = byte(Double(row[min(w - 1, max(0, x - shift)) * 4]) * m)
                px.bytes[p + 1] = byte(Double(row[x * 4 + 1]) * m)
                px.bytes[p + 2] = byte(Double(row[min(w - 1, max(0, x + shift)) * 4 + 2]) * m)
                px.bytes[p + 3] = 255
            }
        }
    }

    /// Round dots on 9 px cells, on the theme's paper, in the image's colours (the boards' Figure, coloured). A dot is
    /// as big as its cell is light (dark theme) or dark (light theme), after 1.4 contrast about the mean; in the dark
    /// its colour is the cell's brought up to full brightness, in the light the cell's a little deeper.
    static func halftone(_ px: inout Pixels, dark: Bool) {
        let cells = Cells(px, size: scaled(9, px.width, least: 3))
        let size = Double(cells.size)
        let n = cells.cols * cells.rows
        let mean = (0..<n).reduce(0.0) { $0 + cells.luma($1) } / Double(n)
        let page = paper(dark: dark)
        for i in stride(from: 0, to: px.bytes.count, by: 4) {
            px.bytes[i] = page.r; px.bytes[i + 1] = page.g; px.bytes[i + 2] = page.b; px.bytes[i + 3] = 255
        }
        for i in 0..<n {
            let g = min(1, max(0, mean + (cells.luma(i) - mean) * 1.4))
            let amount = dark ? g : 1 - g
            let radius = size * 0.52 * pow(amount, 0.8)
            guard radius > 0.4 else { continue }
            let r = cells.r[i] / 255, gr = cells.g[i] / 255, b = cells.b[i] / 255
            let k = dark ? 1 / max(max(r, max(gr, b)), 0.25) : 0.85
            let dot = (byte(min(1, r * k) * 255), byte(min(1, gr * k) * 255), byte(min(1, b * k) * 255))
            disc(&px, cx: Double(i % cells.cols) * size + size / 2, cy: Double(i / cells.cols) * size + size / 2, r: radius, dot)
        }
    }

    /// A filled disc, its edge softened over a pixel.
    private static func disc(_ px: inout Pixels, cx: Double, cy: Double, r: Double, _ c: (UInt8, UInt8, UInt8)) {
        let x0 = max(0, Int(cx - r - 1)), x1 = min(px.width - 1, Int(cx + r + 1))
        let y0 = max(0, Int(cy - r - 1)), y1 = min(px.height - 1, Int(cy + r + 1))
        guard x0 <= x1, y0 <= y1 else { return }
        for y in y0...y1 {
            for x in x0...x1 {
                let dx = Double(x) + 0.5 - cx, dy = Double(y) + 0.5 - cy
                let cover = min(1, max(0, r - (dx * dx + dy * dy).squareRoot() + 0.5))
                guard cover > 0 else { continue }
                let p = px.index(x, y)
                px.bytes[p] = byte(Double(px.bytes[p]) * (1 - cover) + Double(c.0) * cover)
                px.bytes[p + 1] = byte(Double(px.bytes[p + 1]) * (1 - cover) + Double(c.1) * cover)
                px.bytes[p + 2] = byte(Double(px.bytes[p + 2]) * (1 - cover) + Double(c.2) * cover)
            }
        }
    }
}
