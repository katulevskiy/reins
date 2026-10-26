import SwiftUI

// The drawn pieces of arriving (the Arrival and Tour boards, the kit's `onode`, `ophone`, `olink`, `sky`, `progress`,
// `hero_text`, `status_chip`): round nodes and little phones joined by dots, soft colour tiles with their grain, the
// onboarding progress, and the field every account page opens with. Decoration is hidden from VoiceOver; the words
// beside it say what it means.

/// A round node of an illustrated chain: a glyph on a disc (white on a field, or a wash), with a soft shadow.
struct ArtNode: View {
    var glyph: Glyph
    var size: CGFloat
    var fill: Color = .white
    var ink: Color = Palette.fieldAccounts
    var rotation: Double = 0
    var shadow = true

    var body: some View {
        Circle()
            .fill(fill)
            .frame(width: size, height: size)
            .overlay { GlyphIcon(glyph, size: (size * 0.42).rounded()).foregroundStyle(ink).rotationEffect(.degrees(rotation)) }
            .shadow(color: .black.opacity(shadow ? 0.3 : 0), radius: 15, y: 14)
            .accessibilityHidden(true)
    }
}

/// A little phone drawn as a shape (a rounded slab with its speaker), a glyph on its screen.
struct ArtPhone: View {
    var glyph: Glyph
    var width: CGFloat
    var height: CGFloat
    var fill: Color = .white
    var ink: Color = Palette.fieldAccounts
    var shadow = false

    var body: some View {
        RoundedRectangle(cornerRadius: 20, style: .continuous)
            .fill(fill)
            .frame(width: width, height: height)
            .overlay(alignment: .top) {
                Capsule().fill(ink.opacity(0.25)).frame(width: width * 0.3, height: 6).padding(.top, 8)
            }
            .overlay { GlyphIcon(glyph, size: (width * 0.44).rounded()).foregroundStyle(ink) }
            .shadow(color: .black.opacity(shadow ? 0.35 : 0), radius: 20, y: 22)
            .accessibilityHidden(true)
    }
}

/// The dotted joint between two nodes of a chain.
struct ArtDots: View {
    var count = 3
    var color: Color = .white

    var body: some View {
        HStack(spacing: 5) {
            ForEach(0..<count, id: \.self) { _ in
                Circle().fill(color.opacity(0.55)).frame(width: 5, height: 5)
            }
        }
        .accessibilityHidden(true)
    }
}

/// A soft colour tile with its grain that holds an onboarding picture (the kit's `sky`): the colour mixed into paper,
/// radius 32.
struct SkyTile<Content: View>: View {
    var tint: Color
    var mix: Double = 0.16
    var height: CGFloat
    /// A solid gradient instead of the mix (the lock screen's sky).
    var gradient: LinearGradient?
    @ViewBuilder var content: Content

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 32, style: .continuous)
        ZStack {
            if let gradient {
                gradient
            } else {
                Palette.paper
                tint.opacity(mix)
            }
            Grain()
            content
        }
        .frame(maxWidth: .infinity)
        .frame(height: height)
        .clipShape(shape)
    }
}

/// The onboarding progress (the kit's `progress`): a 4 pt bar per page, filled as you arrive, and Skip.
struct TourProgress: View {
    var page: Int
    var count: Int
    var onSkip: (() -> Void)?

    var body: some View {
        HStack(spacing: 14) {
            HStack(spacing: 5) {
                ForEach(0..<count, id: \.self) { i in
                    Capsule().fill(i <= page ? Palette.ink : Palette.rule2).frame(height: 4)
                }
            }
            .animation(.easeOut(duration: 0.3), value: page)
            .accessibilityElement()
            .accessibilityLabel("Step \(page + 1) of \(count)")
            .accessibilityIdentifier("setupProgress")
            if let onSkip {
                Button("Skip", action: onSkip)
                    .font(RFont.sans(16, .semibold))
                    .foregroundStyle(Palette.ink2)
                    .frame(minHeight: 44)
                    .accessibilityLabel("Skip setup")
                    .accessibilityIdentifier("setupSkip")
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 4)
        .frame(minHeight: 44)
    }
}

/// An onboarding page's title (display 41) and at most one short line under it.
struct HeroText: View {
    var title: String
    var line: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title)
                .textRole(.hero)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
            if let line {
                Text(line)
                    .font(RFont.sans(17))
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
    }
}

/// A small chip with a glyph, a word and a green check, or a minus where something is missing (All set).
struct SetupChip: View {
    var glyph: Glyph
    var text: String
    var ok: Bool
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 8) {
            GlyphIcon(glyph, size: 18).foregroundStyle(Palette.ink2)
            Text(text).font(RFont.sans(15, .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
            GlyphIcon(ok ? .check : .minus, size: 15).foregroundStyle(ok ? Palette.success : Palette.inkMeta)
        }
        .padding(.leading, 12)
        .padding(.trailing, 14)
        .frame(minHeight: 44)
        .background(Palette.paperRaised, in: Capsule())
        .overlay { if scheme == .light { Capsule().strokeBorder(Palette.rule, lineWidth: 1) } }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(text), \(ok ? "ready" : "not yet")")
    }
}

/// Lays chips out in centred rows, wrapping when one is full.
struct CenteredFlow: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? 360, subviews: subviews)
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        return CGSize(width: proposal.width ?? rows.map(\.width).max() ?? 0, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.midX - row.width / 2
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: y + (row.height - size.height) / 2), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var items: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for (i, subview) in subviews.enumerated() {
            let size = subview.sizeThatFits(.unspecified)
            let needed = row.items.isEmpty ? size.width : row.width + spacing + size.width
            if needed > width, !row.items.isEmpty {
                rows.append(row)
                row = Row()
            }
            row.width = row.items.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.items.append(i)
        }
        if !row.items.isEmpty { rows.append(row) }
        return rows
    }
}

/// The bottom buttons of an arriving page (the kit's `foot`): on paper, 16 pt in, above the home indicator.
struct ArriveFooter<Content: View>: View {
    var background: Color = Palette.paper
    @ViewBuilder var content: Content

    var body: some View {
        VStack(spacing: 8) { content }
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
            .background(background.ignoresSafeArea())
    }
}
