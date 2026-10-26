import SwiftUI

/// The five modes as Next draws them (the System board, Modes): one glyph, one colour and two or three words each, the
/// same in the header chip, Trust's mode card, the mode sheet, the running field and the widgets. The bolt is only ever
/// Autopilot; Bypass is a shield with a mark; Lockdown a block.
enum ModeKind: String, CaseIterable {
    case manual, auto, overnight, lockdown, bypass

    var name: String {
        switch self {
        case .manual: "Manual"
        case .auto: "Auto"
        case .overnight: "Overnight"
        case .lockdown: "Lockdown"
        case .bypass: "Bypass"
        }
    }

    /// What the mode does, in the card and the sheet.
    var effect: String {
        switch self {
        case .manual: "You decide"
        case .auto: "Easy ones decided"
        case .overnight: "Risky ones wait"
        case .lockdown: "Everything denied"
        case .bypass: "Everything allowed"
        }
    }

    var glyph: Glyph {
        switch self {
        case .manual: .person
        case .auto: .autopilot
        case .overnight: .moon
        case .lockdown: .block
        case .bypass: .shieldAlert
        }
    }

    /// The solid colour: the chip, the card, the running field. Manual's is ink (inverse), and it is drawn on paper.
    var fill: Color {
        switch self {
        case .manual: Palette.inverse
        case .auto: Palette.violetFill
        case .overnight: Palette.modeOvernight
        case .lockdown: Palette.fieldWarn
        case .bypass: Palette.fieldDelete
        }
    }

    /// The mode as a glyph or a word on paper (menus, tags, Trust's tab dot).
    var ink: Color {
        switch self {
        case .manual: Palette.ink
        case .auto: Palette.violet
        case .overnight: Palette.nightText
        case .lockdown: Palette.warning
        case .bypass: Palette.danger
        }
    }

    /// The calm modes sit on paper; the others become their colour field.
    var calm: Bool { self == .manual || self == .auto }

    /// Overnight, Lockdown and Bypass: started "for a while", from the one row on Trust.
    static let timed: [ModeKind] = [.overnight, .lockdown, .bypass]
}

/// A mode's glyph; Autopilot's bolt is filled, as on the boards.
struct ModeGlyph: View {
    var mode: ModeKind
    var size: CGFloat = 18

    init(_ mode: ModeKind, size: CGFloat = 18) {
        self.mode = mode
        self.size = size
    }

    var body: some View {
        if mode == .auto {
            Bolt(size: size)
        } else {
            GlyphIcon(mode.glyph, size: size)
        }
    }
}

/// Autopilot's bolt, filled (the glyph set's `autopilot` path with a fill and a 1 pt stroke on a 24 grid).
struct Bolt: View {
    var size: CGFloat = 18
    @ScaledMetric(relativeTo: .body) private var scale: CGFloat = 1

    var body: some View {
        let side = size * scale
        BoltShape()
            .fill(.foreground)
            .overlay { BoltShape().stroke(.foreground, style: StrokeStyle(lineWidth: side / 24, lineJoin: .round)) }
            .frame(width: side, height: side)
            .accessibilityHidden(true)
    }
}

struct BoltShape: Shape {
    func path(in rect: CGRect) -> Path {
        let u = rect.width / 24
        var p = Path()
        p.move(to: CGPoint(x: 13.5 * u, y: 3 * u))
        p.addLine(to: CGPoint(x: 5.5 * u, y: 13.5 * u))
        p.addLine(to: CGPoint(x: 11.5 * u, y: 13.5 * u))
        p.addLine(to: CGPoint(x: 10.5 * u, y: 21 * u))
        p.addLine(to: CGPoint(x: 18.5 * u, y: 10.5 * u))
        p.addLine(to: CGPoint(x: 12.5 * u, y: 10.5 * u))
        p.closeSubpath()
        return p.offsetBy(dx: rect.minX, dy: rect.minY)
    }
}

/// A timed mode as a tiny tile in its colour with its grain (the "For a while" row, the mode sheet).
struct ModeTile: View {
    var mode: ModeKind
    var side: CGFloat = 24

    var body: some View {
        RoundedRectangle(cornerRadius: side / 3, style: .continuous)
            .fill(mode == .manual ? Palette.paperSunken : mode.fill)
            .overlay { if mode != .manual { Grain().clipShape(RoundedRectangle(cornerRadius: side / 3, style: .continuous)) } }
            .overlay { ModeGlyph(mode, size: side * 0.54).foregroundStyle(mode == .manual ? Palette.ink : .white) }
            .frame(width: side, height: side)
    }
}
