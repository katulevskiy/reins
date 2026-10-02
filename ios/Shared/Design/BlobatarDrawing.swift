import SwiftUI
import UIKit

/// Blobatar figures as SwiftUI paths, built once per name.
enum BlobatarDrawing {
    struct Drawn {
        let path: Path
        let color: Color
    }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var cache: [String: [Drawn]] = [:]

    static func marks(for seed: String) -> [Drawn] {
        let key = seed.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "?" : seed
        if let hit = lock.withLock({ cache[key] }) { return hit }
        let drawn = Blobatar.figure(key).marks.map { mark -> Drawn in
            switch mark {
            case let .path(d, fill): Drawn(path: SVGPath.parse(d), color: color(fill))
            case let .circle(cx, cy, r, fill): Drawn(path: Path(ellipseIn: CGRect(x: cx - r, y: cy - r, width: 2 * r, height: 2 * r)), color: color(fill))
            }
        }
        lock.withLock {
            if cache.count > 256 { cache.removeAll() }
            cache[key] = drawn
        }
        return drawn
    }

    /// "#rrggbb".
    static func color(_ hex: String) -> Color {
        let value = UInt32(hex.dropFirst(), radix: 16) ?? 0
        return Color(uiColor: UIColor(hex: value))
    }
}

/// The subset of SVG path data the blobatar renderer writes: absolute M, L, H, V, C, Q and Z.
enum SVGPath {
    static func parse(_ d: String) -> Path {
        var path = Path()
        var command: Character = "M"
        var numbers: [CGFloat] = []
        var current = CGPoint.zero

        func flush() {
            var n = numbers[...]
            func take() -> CGFloat? { n.isEmpty ? nil : n.removeFirst() }
            func point() -> CGPoint? {
                guard let x = take(), let y = take() else { return nil }
                return CGPoint(x: x, y: y)
            }
            switch command {
            case "M":
                if let p = point() { path.move(to: p); current = p }
                while let p = point() { path.addLine(to: p); current = p }
            case "L":
                while let p = point() { path.addLine(to: p); current = p }
            case "H":
                while let x = take() { current = CGPoint(x: x, y: current.y); path.addLine(to: current) }
            case "V":
                while let y = take() { current = CGPoint(x: current.x, y: y); path.addLine(to: current) }
            case "C":
                while let c1 = point(), let c2 = point(), let p = point() { path.addCurve(to: p, control1: c1, control2: c2); current = p }
            case "Q":
                while let c = point(), let p = point() { path.addQuadCurve(to: p, control: c); current = p }
            case "Z":
                path.closeSubpath()
            default:
                break
            }
            numbers = []
        }

        var token = ""
        func endToken() {
            if !token.isEmpty, let v = Double(token) { numbers.append(CGFloat(v)) }
            token = ""
        }
        for ch in d {
            if ch.isLetter && ch != "e" {
                endToken()
                flush()
                command = ch
            } else if ch == " " || ch == "," {
                endToken()
            } else if ch == "-" && !token.isEmpty && token.last != "e" {
                endToken()
                token = "-"
            } else {
                token.append(ch)
            }
        }
        endToken()
        flush()
        return path
    }
}
