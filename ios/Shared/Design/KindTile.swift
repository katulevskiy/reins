import SwiftUI

/// A kind on paper: its tile (the kind's ink at 13 %, 14 % in dark) under its own ink. 38 pt tiles round 11; larger ones
/// scale the corner. An operation with no kind is ink-2 on a sunken tile.
struct KindTile: View {
    private var field: FieldKind?
    private var content: Content
    var size: CGFloat

    private enum Content {
        case symbol(String)
        case glyph(Glyph)
    }

    /// An operation's kind with its SF Symbol (the History and Activity rows until they move to glyphs).
    init(kind: ActionKind, danger: Bool = false, size: CGFloat = 36) {
        field = FieldKind.of(kind, danger: danger)
        content = .symbol(kind.symbol)
        self.size = size
    }

    /// A field's kind with a Signal glyph (its own by default).
    init(_ field: FieldKind, glyph: Glyph? = nil, size: CGFloat = 38) {
        self.field = field
        content = .glyph(glyph ?? field.glyph)
        self.size = size
    }

    var body: some View {
        let ink = field?.ink ?? Palette.ink2
        Group {
            switch content {
            case .symbol(let name):
                Image(systemName: name).font(.system(size: size * 0.45, weight: .semibold))
            case .glyph(let glyph):
                Image(glyph: glyph).resizable().scaledToFit().frame(width: size * 0.53, height: size * 0.53)
            }
        }
        .foregroundStyle(ink)
        .frame(width: size, height: size)
        .background(field?.tile ?? Palette.paperSunken, in: RoundedRectangle(cornerRadius: size * 11 / 38, style: .continuous))
        .accessibilityHidden(true)
    }
}
