import Foundation

/// Blobatar 2 (gen2), https://github.com/Alain00/blobatar, MIT licence, (c) 2026 Alain.
///
/// A line-for-line port of the Android app's port (`design/blobatar/Blobatar.kt`) of the library's static renderer
/// (`hash`, `traits`, `color`, `shape`, `styles/shapes`, `styles/compose`, `render`). It adds no geometry and changes
/// no constant; `BlobatarGoldenTests` renders the library's own 1000-seed golden fixture and requires byte-identical
/// SVG markup. Rounding follows JavaScript's `Math.round` (ties toward +infinity), lengths count UTF-16 units like
/// JavaScript strings, and the trig goes through one seam (`JSMath`), all for the same reason: the same name must draw
/// the same figure on every platform.
///
/// Expressions, motion and trait overrides of the library are not ported: the app draws the resting figure.
enum Blobatar {
    /// One drawn primitive; the fill rides on every mark.
    enum Mark: Equatable {
        case path(d: String, fill: String)
        case circle(cx: Double, cy: Double, r: Double, fill: String)

        var fill: String {
            switch self {
            case let .path(_, fill), let .circle(_, _, _, fill): fill
            }
        }
    }

    /// The figure: the backdrop plate (if any) followed by the marks, in draw order, in a 100 x 100 box.
    struct Figure: Equatable {
        var backdrop: Mark?
        var marks: [Mark]
    }

    enum Backdrop { case none, square, circle, squircle }

    static func figure(_ name: String, backdrop: Backdrop = .none) -> Figure {
        let t = Traits(name)
        let colors = palette(hue: t.num("hue", 0, 360), enforce: true, tone: t.unit("tone"))
        let shape = layout(t)
        let plate: Mark? = switch backdrop {
        case .none: nil
        case .square: .path(d: "M0 0H100V100H0Z", fill: colors.bg)
        case .circle: .path(d: superellipse(Ellipse(cx: 50, cy: 50, rx: 50, ry: 50), n: 2, rot: 0), fill: colors.bg)
        case .squircle: .path(d: superellipse(Ellipse(cx: 50, cy: 50, rx: 50, ry: 50), n: 6, rot: 0), fill: colors.bg)
        }
        return Figure(backdrop: plate, marks: marks(shape, colors))
    }

    /// The library's `blobatar(name, { background })` markup, used to check this port against the golden fixture.
    static func svg(_ name: String, backdrop: Backdrop = .none) -> String {
        let figure = figure(name, backdrop: backdrop)
        guard let head = figure.marks.first?.fill, let eye = figure.marks.last?.fill else { return "" }
        var body = ""
        if case let .path(d, fill)? = figure.backdrop { body += "<path d=\"\(d)\" fill=\"\(fill)\"/>" }
        body += "<g fill=\"\(head)\">"
        let eyes = 2
        for m in figure.marks.dropLast(eyes) {
            switch m {
            case let .circle(cx, cy, r, _): body += "<circle cx=\"\(num(cx))\" cy=\"\(num(cy))\" r=\"\(num(r))\"/>"
            case let .path(d, _): body += "<path d=\"\(d)\"/>"
            }
        }
        body += "</g><g fill=\"\(eye)\">"
        for m in figure.marks.suffix(eyes) {
            if case let .path(d, _) = m { body += "<path d=\"\(d)\"/>" }
        }
        body += "</g>"
        return "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">\(body)</svg>"
    }

    // MARK: Hash

    private static let sep: UInt8 = 0xff

    private static func feed(_ state: UInt32, _ bytes: some Sequence<UInt8>) -> UInt32 {
        var h = state
        for b in bytes {
            h = (h ^ UInt32(b)) &* 3_432_918_353
            h = (h << 13) | (h >> 19)
        }
        return h
    }

    private static func finalize(_ state: UInt32) -> UInt32 {
        var h = state
        h = (h ^ (h >> 16)) &* 2_246_822_507
        h = (h ^ (h >> 13)) &* 3_266_489_909
        return h ^ (h >> 16)
    }

    /// JavaScript's `String.prototype.trim` whitespace set (white space and line terminators, incl. U+FEFF).
    private static func isJsSpace(_ c: Unicode.Scalar) -> Bool {
        if c == "\u{FEFF}" { return true }
        if c == "\u{200B}" { return false }
        if (0x09...0x0D).contains(c.value) { return true }
        switch c.properties.generalCategory {
        case .spaceSeparator, .lineSeparator, .paragraphSeparator: return true
        default: return false
        }
    }

    static func normalizeSeed(_ seed: String) -> String {
        let scalars = Array(seed.precomposedStringWithCanonicalMapping.unicodeScalars)
        var lo = 0
        var hi = scalars.count
        while lo < hi, isJsSpace(scalars[lo]) { lo += 1 }
        while hi > lo, isJsSpace(scalars[hi - 1]) { hi -= 1 }
        var view = String.UnicodeScalarView()
        view.append(contentsOf: scalars[lo..<hi])
        return String(view).lowercased()
    }

    private struct Traits {
        let state: UInt32

        init(_ seed: String) {
            let s = normalizeSeed(seed)
            // JavaScript's `length`: UTF-16 code units.
            state = Blobatar.feed(1_779_033_703 ^ UInt32(truncatingIfNeeded: s.utf16.count), s.utf8)
        }

        /// Uniform double in [0, 1).
        func unit(_ key: String) -> Double {
            Double(Blobatar.finalize(Blobatar.feed(Blobatar.feed(state, [Blobatar.sep]), key.utf8))) / 4_294_967_296.0
        }

        func num(_ key: String, _ min: Double, _ max: Double) -> Double { min + unit(key) * (max - min) }

        func int(_ key: String, _ min: Int, _ max: Int) -> Int { min + Int((unit(key) * Double(max - min + 1)).rounded(.down)) }

        func jitter(_ key: String, _ amount: Double) -> Double { (unit(key) * 2 - 1) * amount }
    }

    // MARK: Colour

    private struct Oklch {
        var l: Double
        var c: Double
        var h: Double
    }

    private struct Palette {
        let bg: String
        let head: String
        let eye: String
    }

    private static func toLinear(_ color: Oklch) -> [Double] {
        let r = color.h * Double.pi / 180
        let a = color.c * JSMath.cos(r)
        let b = color.c * JSMath.sin(r)
        let l1 = color.l + 0.3963377774 * a + 0.2158037573 * b
        let m1 = color.l - 0.1055613458 * a - 0.0638541728 * b
        let s1 = color.l - 0.0894841775 * a - 1.291485548 * b
        let l = l1 * l1 * l1
        let m = m1 * m1 * m1
        let s = s1 * s1 * s1
        return [
            4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
        ]
    }

    private static func inGamut(_ rgb: [Double]) -> Bool { rgb.allSatisfy { $0 >= -1e-4 && $0 <= 1 + 1e-4 } }

    private static func resolve(_ color: Oklch) -> [Double] {
        var rgb = toLinear(color)
        if !inGamut(rgb) {
            var lo = 0.0
            var hi = color.c
            for _ in 0..<12 {
                let mid = (lo + hi) / 2
                var probe = color
                probe.c = mid
                if inGamut(toLinear(probe)) { lo = mid } else { hi = mid }
            }
            var fit = color
            fit.c = lo
            rgb = toLinear(fit)
        }
        return rgb.map { Swift.min(1.0, Swift.max(0.0, $0)) }
    }

    private static func luminance(_ color: Oklch) -> Double {
        let rgb = resolve(color)
        return 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
    }

    private static func contrast(_ a: Oklch, _ b: Oklch) -> Double {
        let x = luminance(a)
        let y = luminance(b)
        return (Swift.max(x, y) + 0.05) / (Swift.min(x, y) + 0.05)
    }

    private static func ensureContrast(_ fg: Oklch, _ bg: Oklch, _ min: Double) -> Oklch {
        if contrast(fg, bg) >= min { return fg }
        let lean = fg.l >= bg.l ? 1 : -1
        for dir in [lean, -lean] {
            var probe = fg
            for _ in 0..<60 {
                probe.l = Swift.min(1.0, Swift.max(0.0, probe.l + Double(dir) * 0.02))
                if contrast(probe, bg) >= min { return probe }
                if probe.l == 0.0 || probe.l == 1.0 { break }
            }
        }
        let black = Oklch(l: 0, c: 0, h: fg.h)
        let white = Oklch(l: 1, c: 0, h: fg.h)
        return contrast(black, bg) >= contrast(white, bg) ? black : white
    }

    private static func toHex(_ color: Oklch) -> String {
        var out = "#"
        for v in resolve(color) {
            let s = v <= 0.0031308 ? 12.92 * v : 1.055 * JSMath.pow(v, 1 / 2.4) - 0.055
            let byte = jsRound(s * 255)
            let hex = String(byte, radix: 16)
            out += hex.count < 2 ? "0" + hex : hex
        }
        return out
    }

    private static let tones: [(Double, Oklch)] = [
        (0.2, Oklch(l: 0.86, c: 0.085, h: 0)),
        (0.36, Oklch(l: 0.9, c: 0.028, h: 0)),
        (0.62, Oklch(l: 0.73, c: 0.135, h: 0)),
        (0.8, Oklch(l: 0.62, c: 0.165, h: 0)),
        (0.93, Oklch(l: 0.87, c: 0.16, h: 0)),
        (1.0, Oklch(l: 0.34, c: 0.035, h: 0)),
    ]

    private static let darkSurface = Oklch(l: 0.145, c: 0, h: 0)
    private static let surfaceFloor = 1.5

    private static func palette(hue: Double, enforce: Bool, tone: Double) -> Palette {
        let t = (tones.first { tone < $0.0 } ?? tones[0]).1
        let head0 = ensureContrast(Oklch(l: t.l, c: t.c, h: hue), darkSurface, surfaceFloor)
        let bg = Oklch(l: 0.965, c: 0.01, h: hue)
        var head = head0
        var eye = head0.l >= 0.5 ? Oklch(l: 0.17, c: 0.02, h: hue) : Oklch(l: 0.97, c: 0.012, h: hue)
        if enforce {
            head = ensureContrast(head, bg, 1.25)
            eye = ensureContrast(eye, head, 4.5)
        }
        return Palette(bg: toHex(bg), head: toHex(head), eye: toHex(eye))
    }

    // MARK: Shape primitives

    /// JavaScript's `Math.round`: the nearest integer, ties toward +infinity.
    static func jsRound(_ v: Double) -> Int64 {
        let f = v.rounded(.down)
        return Int64(v - f >= 0.5 ? f + 1 : f)
    }

    /// JavaScript's `String(Math.round(v * 100) / 100)` (with -0 as "0").
    static func num(_ v: Double) -> String {
        let scaled = jsRound(v * 100)
        if scaled == 0 { return "0" }
        let abs = Swift.abs(scaled)
        let whole = abs / 100
        let frac = Int(abs % 100)
        let sign = scaled < 0 ? "-" : ""
        if frac == 0 { return "\(sign)\(whole)" }
        if frac % 10 == 0 { return "\(sign)\(whole).\(frac / 10)" }
        return "\(sign)\(whole).\(frac < 10 ? "0" : "")\(frac)"
    }

    private static func r2(_ v: Double) -> Double { Double(jsRound(v * 100)) / 100.0 }

    private struct Ellipse {
        var cx: Double
        var cy: Double
        var rx: Double
        var ry: Double
    }

    private static func superellipse(_ e: Ellipse, n: Double = 4, rot: Double = 0) -> String {
        let k = Swift.min(1.0, (8 * JSMath.pow(2.0, -1 / n) - 4) / 3)
        let a = e.rx
        let b = e.ry
        let ak = a * k
        let bk = b * k
        let pts: [(Double, Double)] = [
            (a, 0.0), (a, bk), (ak, b), (0.0, b), (-ak, b), (-a, bk), (-a, 0.0),
            (-a, -bk), (-ak, -b), (0.0, -b), (ak, -b), (a, -bk), (a, 0.0),
        ]
        let t = rot * Double.pi / 180
        let cos = JSMath.cos(t)
        let sin = JSMath.sin(t)
        func at(_ i: Int) -> String {
            let (x, y) = pts[i]
            return "\(num(e.cx + x * cos - y * sin)) \(num(e.cy + x * sin + y * cos))"
        }
        var d = "M\(at(0))"
        var i = 1
        while i < 13 {
            d += "C\(at(i)) \(at(i + 1)) \(at(i + 2))"
            i += 3
        }
        return d + "Z"
    }

    private static func blobPath(cx: Double, cy: Double, rx: Double, ry: Double, radii: [Double], rot: Double) -> String {
        let n = radii.count
        let t0 = rot * Double.pi / 180
        let p: [(Double, Double)] = radii.enumerated().map { i, m in
            let a = t0 + 2 * Double.pi * Double(i) / Double(n)
            return (cx + rx * m * JSMath.cos(a), cy + ry * m * JSMath.sin(a))
        }
        func at(_ i: Int) -> (Double, Double) { p[((i % n) + n) % n] }
        var d = "M\(num(at(0).0)) \(num(at(0).1))"
        for i in 0..<n {
            let (x0, y0) = at(i - 1)
            let (x1, y1) = at(i)
            let (x2, y2) = at(i + 1)
            let (x3, y3) = at(i + 2)
            d += "C\(num(x1 + (x2 - x0) / 6)) \(num(y1 + (y2 - y0) / 6))"
            d += " \(num(x2 - (x3 - x1) / 6)) \(num(y2 - (y3 - y1) / 6))"
            d += " \(num(x2)) \(num(y2))"
        }
        return d + "Z"
    }

    private static func polygon(_ b: Body) -> String {
        let sides = b.sides
        let round = b.round
        let k = round > 0 ? (round < 1 ? round / 2 : 0.5) : 0.0
        let t0 = b.rot * Double.pi / 180 - Double.pi / 2
        let v: [(Double, Double)] = (0..<sides).map { i in
            let a = t0 + 2 * Double.pi * Double(i) / Double(sides)
            return (b.cx + b.rx * JSMath.cos(a), b.cy + b.ry * JSMath.sin(a))
        }
        func at(_ i: Int) -> (Double, Double) { v[((i % sides) + sides) % sides] }
        func cut(_ i: Int, _ j: Int) -> String {
            let (x0, y0) = at(i)
            let (x1, y1) = at(j)
            return "\(num(x0 + (x1 - x0) * k)) \(num(y0 + (y1 - y0) * k))"
        }
        var d = "M\(cut(0, -1))"
        for i in 0..<sides {
            let (x, y) = at(i)
            d += "Q\(num(x)) \(num(y)) \(cut(i, i + 1))"
            if k < 0.5 { d += "L\(cut(i + 1, i))" }
        }
        return d + "Z"
    }

    private static func box(cx: Double, cy: Double, rx: Double, ry: Double) -> String {
        let l = num(cx - rx)
        let r = num(cx + rx)
        return "M\(l) \(num(cy - ry))H\(r)V\(num(cy + ry))H\(l)Z"
    }

    private static func taper(cx: Double, cy: Double, rx: Double, ry: Double, tip: Double) -> String {
        let t = Swift.max(1.05, tip)
        let tx = rx * (1 - 1 / (t * t)).squareRoot()
        let ty = cy - ry / t
        let apex = cy - t * ry
        let px = tx * 0.14
        let py = ty + 0.86 * (apex - ty)
        return "M\(num(cx - tx)) \(num(ty))"
            + "L\(num(cx - px)) \(num(py))"
            + "Q\(num(cx)) \(num(apex)) \(num(cx + px)) \(num(py))"
            + "L\(num(cx + tx)) \(num(ty))Z"
    }

    // MARK: Silhouettes and layout

    private final class Body {
        var cx: Double
        var cy: Double
        var rx: Double
        var ry: Double
        var n: Double
        var rot: Double
        let radii: [Double]
        var sides = 0
        var round = 0.0

        init(cx: Double, cy: Double, rx: Double, ry: Double, n: Double, rot: Double, radii: [Double]) {
            self.cx = cx
            self.cy = cy
            self.rx = rx
            self.ry = ry
            self.n = n
            self.rot = rot
            self.radii = radii
        }

        var ellipse: Ellipse { Ellipse(cx: cx, cy: cy, rx: rx, ry: ry) }
    }

    private struct Petal {
        let cx: Double
        let cy: Double
        let r: Double
    }

    private final class Deco {
        var petals: [Petal] = []
        var extra: [String] = []
    }

    private struct Shape {
        let name: String
        let core: Double
        var body: ((Traits, Body) -> Void)? = nil
        var face: ((Body) -> Ellipse)? = nil
        var decorate: ((Traits, Body, Deco) -> Void)? = nil
        var path: ((Body) -> String)? = nil
    }

    private static let spline: (Body) -> String = { blobPath(cx: $0.cx, cy: $0.cy, rx: $0.rx, ry: $0.ry, radii: $0.radii, rot: $0.rot) }

    private static func shrunk(_ k: Double) -> (Body) -> Ellipse {
        { Ellipse(cx: $0.cx, cy: $0.cy, rx: $0.rx * k, ry: $0.ry * k) }
    }

    private static let splineFace: (Body) -> Ellipse = { b in shrunk((b.radii.min() ?? 1) * 0.95)(b) }
    private static let polyFace: (Body) -> Ellipse = shrunk(0.84)

    private static let round = Shape(name: "round", core: 1.0)
    private static let organic = Shape(name: "organic", core: 0.98, face: splineFace, path: spline)
    private static let boxy = Shape(name: "boxy", core: 0.86, body: { t, b in
        b.n = t.num("body.n", 3.4, 6.0)
        b.rot = t.num("body.rot", -20.0, 20.0)
    })
    private static let capsule = Shape(
        name: "capsule", core: 1.02,
        body: { t, b in b.ry *= t.num("capsule.squat", 0.55, 0.68) },
        face: shrunk(0.94),
        decorate: { _, b, out in
            for s in [-1.0, 1.0] { out.petals.append(Petal(cx: b.cx + s * (b.rx - b.ry), cy: b.cy, r: b.ry)) }
        },
        path: { box(cx: $0.cx, cy: $0.cy, rx: $0.rx - $0.ry, ry: $0.ry) }
    )
    private static let nub = Shape(name: "nub", core: 0.88, decorate: { t, b, out in
        let count = t.int("nub.n", 1, 2)
        for i in 0..<count {
            let a = t.num("nub.a\(i)", 0.0, 2 * Double.pi)
            out.petals.append(Petal(
                cx: b.cx + JSMath.cos(a) * b.rx * 0.88,
                cy: b.cy + JSMath.sin(a) * b.rx * 0.88,
                r: b.rx * t.num("nub.r\(i)", 0.24, 0.4)
            ))
        }
    })
    private static let cloud = Shape(
        name: "cloud", core: 0.78, face: splineFace,
        decorate: { t, b, out in
            let count = t.int("cloud.n", 4, 6)
            for i in 0..<count {
                let a = Double.pi + (Double.pi * (Double(i) + 0.5)) / Double(count)
                out.petals.append(Petal(
                    cx: b.cx + JSMath.cos(a) * b.rx * 0.8,
                    cy: b.cy + JSMath.sin(a) * b.rx * 0.5,
                    r: b.rx * t.num("cloud.r\(i)", 0.44, 0.62)
                ))
            }
        },
        path: spline
    )
    private static let droplet = Shape(
        name: "droplet", core: 0.78,
        body: { _, b in
            b.cy += 0.22 * b.ry
            b.n = 2.0
        },
        face: { Ellipse(cx: $0.cx, cy: $0.cy + $0.ry * 0.05, rx: $0.rx * 0.88, ry: $0.ry * 0.88) },
        decorate: { t, b, out in out.extra.append(taper(cx: b.cx, cy: b.cy, rx: b.rx, ry: b.ry, tip: t.num("droplet.tip", 1.4, 1.65))) }
    )
    private static let hexagon = Shape(
        name: "hexagon", core: 1.05,
        body: { t, b in
            b.sides = 6
            b.rot = t.num("body.rot", -12.0, 12.0)
            b.round = t.num("poly.round", 0.24, 0.5)
        },
        face: polyFace,
        path: polygon
    )
    private static let sun = Shape(name: "sun", core: 0.7, decorate: { t, b, out in
        let count = t.int("sun.n", 6, 9)
        let dist = b.rx * t.num("sun.dist", 1.0, 1.08)
        let pr = b.rx * t.num("sun.r", 0.2, 0.26)
        let off = t.num("sun.rot", 0.0, 2 * Double.pi)
        for i in 0..<count {
            let a = off + (2 * Double.pi * Double(i)) / Double(count)
            out.petals.append(Petal(cx: b.cx + JSMath.cos(a) * dist, cy: b.cy + JSMath.sin(a) * dist, r: pr))
        }
    })
    private static let triangle = Shape(
        name: "triangle", core: 1.15,
        body: { t, b in
            b.sides = 3
            b.rot = t.num("body.rot", -5.0, 5.0)
            b.round = t.num("poly.round", 0.24, 0.5)
        },
        face: { Ellipse(cx: $0.cx, cy: $0.cy + $0.ry * 0.1, rx: $0.rx * 0.54, ry: $0.ry * 0.36) },
        path: polygon
    )

    /// The gen2 band table: `(shape, upper edge of its band in [0, 1))`.
    private static let bands: [(Shape, Double)] = [
        (round, 0.22), (organic, 0.48), (boxy, 0.6), (capsule, 0.7), (nub, 0.79),
        (cloud, 0.86), (droplet, 0.915), (hexagon, 0.95), (sun, 0.98), (triangle, 1.0),
    ]

    private struct Eye {
        let cx: Double
        let cy: Double
        let rx: Double
        let ry: Double
        let n: Double
        let rot: Double
    }

    private struct Layout {
        let shape: Shape
        let body: Body
        let petals: [Petal]
        let extra: [String]
        let eyes: [Eye]
    }

    private static func faceFit(_ t: Traits, _ b: Body, _ face: Ellipse) -> [Eye] {
        let rx = b.rx
        let er0 = t.num("eye.rx", 0.075, 0.105) * rx
        let ratio = t.num("eye.ratio", 1.9, 3.2)
        let scale = t.num("eye.scale", 0.78, 1.24)
        let stretch = t.num("eye.stretch", 0.85, 1.18)
        let clearance = t.num("eye.gap", 0.1, 0.24) * rx
        let wide = er0 * Swift.max(1.0, scale)
        let tall = er0 * ratio * Swift.max(1.0, scale * stretch)
        let gap0 = wide + rx * 0.03 + clearance

        let gx = t.jitter("gaze.x", 0.09) * face.rx
        let gy = t.num("gaze.y", -0.2, 0.08) * face.ry
        let dy = t.jitter("eye.dy", 0.04) * face.ry
        let reach = JSMath.hypot(wide, tall)
        let need = JSMath.hypot(
            (Swift.abs(gx) + gap0 + reach) / face.rx,
            (Swift.abs(gy) + Swift.abs(dy) + reach) / face.ry
        )
        let fit = need > 0.9 ? 0.9 / need : 1.0

        let er = er0 * fit
        let eyeRy = er * ratio
        let gap = gap0 * fit
        let room = Swift.max(0.0, Swift.min(1.0, clearance / tall))
        let bound = Swift.min(12.0, (JSMath.asin(room) * 180) / Double.pi)
        let lean = t.num("eye.lean", -1.0, 1.0) * bound
        let lean2 = Swift.max(-12.0, Swift.min(12.0, lean + t.jitter("eye.lean2", 3.5)))

        let cx = face.cx + gx * fit
        let cy = face.cy + gy * fit
        return [
            Eye(cx: cx - gap, cy: cy, rx: er, ry: eyeRy, n: t.num("eye.n", 3.5, 6.0), rot: lean),
            Eye(cx: cx + gap, cy: cy + dy * fit, rx: er * scale, ry: eyeRy * scale * stretch, n: t.num("eye.n", 3.5, 6.0), rot: lean2),
        ]
    }

    private static func layout(_ t: Traits) -> Layout {
        let v = t.unit("shape")
        let shape = (bands.first { v < $0.1 } ?? bands[bands.count - 1]).0
        let r = t.num("body.r", 31.0, 38.0) * shape.core
        let body = Body(
            cx: 50 + t.jitter("body.x", 1.5),
            cy: 50 + t.jitter("body.y", 1.5),
            rx: r,
            ry: r * t.num("body.ratio", 0.92, 1.08),
            n: t.num("body.n", 1.9, 2.5),
            rot: 0.0,
            radii: (0..<t.int("body.pts", 6, 8)).map { i in 1 + t.jitter("body.r\(i)", 0.16) }
        )
        shape.body?(t, body)
        let face = shape.face?(body) ?? body.ellipse
        let deco = Deco()
        shape.decorate?(t, body, deco)
        return Layout(shape: shape, body: body, petals: deco.petals, extra: deco.extra, eyes: faceFit(t, body, face))
    }

    private static func marks(_ l: Layout, _ p: Palette) -> [Mark] {
        var out: [Mark] = []
        for d in l.petals { out.append(.circle(cx: r2(d.cx), cy: r2(d.cy), r: r2(d.r), fill: p.head)) }
        for d in l.extra { out.append(.path(d: d, fill: p.head)) }
        out.append(.path(d: l.shape.path?(l.body) ?? superellipse(l.body.ellipse, n: l.body.n, rot: l.body.rot), fill: p.head))
        for e in l.eyes {
            out.append(.path(d: superellipse(Ellipse(cx: e.cx, cy: e.cy, rx: e.rx, ry: e.ry), n: e.n, rot: e.rot), fill: p.eye))
        }
        return out
    }
}

/// The math the renderer needs, behind one name. Darwin's libm reproduces the whole golden corpus as it stands (the
/// library's numbers are rounded to two decimals, far above any last-bit difference from V8's fdlibm); should a future
/// fixture disagree, bit-exact fdlibm routines go here and nowhere else.
private enum JSMath {
    static func sin(_ x: Double) -> Double { Foundation.sin(x) }
    static func cos(_ x: Double) -> Double { Foundation.cos(x) }
    static func asin(_ x: Double) -> Double { Foundation.asin(x) }
    static func pow(_ x: Double, _ y: Double) -> Double { Foundation.pow(x, y) }
    static func hypot(_ x: Double, _ y: Double) -> Double { Foundation.hypot(x, y) }
}
