import SwiftUI
import UIKit

/// The seven kinds of decision, each with a solid field of colour (the same in both themes), its ink on paper (lighter
/// in dark) and its kind tile (the ink at 13 %, 14 % in dark). A field appears only where something needs a decision;
/// everywhere else a kind is its ink or its tile.
enum FieldKind: String, CaseIterable {
    /// Writes or sends.
    case push
    /// Reads or searches.
    case read
    /// Can't be undone; pay; bypass.
    case delete
    /// Power already handed over (and Overnight while it runs).
    case grant
    /// Connects something new.
    case pair
    /// Changes an account.
    case accounts
    /// Reins is broken and needs you.
    case warn

    /// The solid field.
    var fill: Color {
        switch self {
        case .push: Palette.fieldPush
        case .read: Palette.fieldRead
        case .delete: Palette.fieldDelete
        case .grant: Palette.fieldGrant
        case .pair: Palette.fieldPair
        case .accounts: Palette.fieldAccounts
        case .warn: Palette.fieldWarn
        }
    }

    /// The kind's ink on paper: tile glyphs, row marks.
    var ink: Color {
        switch self {
        case .push: Palette.kindPush
        case .read: Palette.kindRead
        case .delete: Palette.kindDelete
        case .grant: Palette.kindGrant
        case .pair: Palette.kindPair
        case .accounts: Palette.kindAccounts
        case .warn: Palette.kindWarn
        }
    }

    /// The kind tile's fill: its ink at 13 % on light paper, 14 % on dark.
    var tile: Color {
        let (light, dark) = inkHex
        return Palette.dual(UIColor(hex: light, alpha: 0.13), UIColor(hex: dark, alpha: 0.14))
    }

    private var inkHex: (UInt32, UInt32) {
        switch self {
        case .push: (0xC2410C, 0xFB923C)
        case .read: (0x2563EB, 0x60A5FA)
        case .delete: (0xB91C1C, 0xF87171)
        case .grant: (0x5B43E8, 0xA196FF)
        case .pair: (0x0E7490, 0x22D3EE)
        case .accounts: (0xBE185D, 0xF472B6)
        case .warn: (0xA16207, 0xFACC15)
        }
    }

    /// The word for it, as on the Colour board ("PUSH"); never let the colour say it alone.
    var title: String { rawValue.capitalized }

    /// The glyph its tile carries when nothing more particular is known.
    var glyph: Glyph {
        switch self {
        case .push: .send
        case .read: .search
        case .delete: .trash
        case .grant: .key
        case .pair: .link
        case .accounts: .person
        case .warn: .warning
        }
    }

    /// What an operation is, as a field: reads and searches are `read`, writes and sends `push`; a request that always
    /// asks because it can't be undone is `delete`. Nil for an operation with no kind (it stays ink-2 on paper).
    static func of(_ kind: ActionKind, danger: Bool = false) -> FieldKind? {
        if danger { return .delete }
        switch kind {
        case .search, .list, .read, .headlines, .upload: return .read
        case .write, .send: return .push
        case .grant: return .grant
        case .accounts: return .accounts
        case .pair, .join: return .pair
        case .other: return nil
        }
    }
}

/// A field: the solid colour of a decision with its grain, running into the top of the screen behind the status bar
/// and the Dynamic Island. Only its bottom corners round (30). Its content starts below the safe area: the field
/// extends under it by itself, so no status-bar height is ever hard-coded. `band` makes it the thin band a drill-in
/// keeps at the top (14 pt below its content), so the ask never leaves the screen.
struct SignalField<Content: View>: View {
    var kind: FieldKind
    var band = false
    @ViewBuilder var content: Content

    static var radius: CGFloat { 30 }

    var body: some View {
        content
            .foregroundStyle(Palette.onField)
            .tint(Palette.onField)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.bottom, band ? 14 : 0)
            .background {
                let shape = UnevenRoundedRectangle(bottomLeadingRadius: Self.radius, bottomTrailingRadius: Self.radius, style: .continuous)
                FieldSurface(kind: kind)
                    .clipShape(shape)
                    .ignoresSafeArea(edges: .top)
            }
    }
}

/// A field's colour with its grain, to put behind anything shaped like a field (a waiting card, a swatch):
/// `.background(FieldSurface(kind: .push), in: RoundedRectangle(cornerRadius: 26))`.
struct FieldSurface: View {
    var kind: FieldKind

    var body: some View {
        ZStack {
            kind.fill
            Grain()
        }
        .animation(.easeInOut(duration: Motion.kindChange), value: kind)
    }
}

extension View {
    /// Fills `shape` with a field and its grain, behind this view, and sets on-field ink.
    func fieldBackground<S: Shape>(_ kind: FieldKind, in shape: S) -> some View {
        foregroundStyle(Palette.onField)
            .background(FieldSurface(kind: kind).clipShape(shape))
    }
}
