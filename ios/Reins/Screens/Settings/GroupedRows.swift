import SwiftUI

// Inset-grouped lists in Reins's look (Settings, a grant's page, a connection's page): the page background behind,
// rows on elevated cards, Geist type, small uppercase headers.

extension View {
    /// An inset-grouped list on the page background.
    func reinsGrouped() -> some View {
        listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .listSectionSpacing(22)
            .pageBackground()
    }

    /// A row on the group's card.
    func cardRow() -> some View {
        listRowBackground(Palette.elevated)
            .listRowSeparatorTint(Palette.hairline)
    }
}

/// A section header: small, uppercase, spaced.
struct GroupHeader: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text.uppercased())
            .font(RFont.sans(12.5, .medium))
            .tracking(0.6)
            .foregroundStyle(Palette.secondary)
            .textCase(nil)
            .accessibilityAddTraits(.isHeader)
    }
}

/// A section footer: what the group is about, in a sentence.
struct GroupFooter: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .font(RFont.sans(13))
            .foregroundStyle(Palette.tertiary)
    }
}

/// A title with an optional line under it and an optional leading symbol on a tinted tile.
struct InfoRow<Trailing: View>: View {
    var title: String
    var subtitle: String?
    var symbol: String?
    var tint: Color = Palette.secondary
    /// Addresses and URLs read left to right whatever the language.
    var ltrSubtitle = false
    @ViewBuilder var trailing: Trailing
    @Environment(\.layoutDirection) private var direction

    init(
        title: String, subtitle: String? = nil, symbol: String? = nil, tint: Color = Palette.secondary, ltrSubtitle: Bool = false,
        @ViewBuilder trailing: () -> Trailing
    ) {
        self.title = title
        self.subtitle = subtitle
        self.symbol = symbol
        self.tint = tint
        self.ltrSubtitle = ltrSubtitle
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: 14) {
            if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 16, weight: .medium))
                    .foregroundStyle(tint)
                    .frame(width: 24)
                    .accessibilityHidden(true)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(RFont.sans(16, .medium)).foregroundStyle(Palette.text)
                if let subtitle {
                    Text(subtitle)
                        .font(RFont.sans(14))
                        .foregroundStyle(Palette.secondary)
                        .environment(\.layoutDirection, ltrSubtitle ? .leftToRight : direction)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trailing
        }
        .padding(.vertical, 3)
        .accessibilityElement(children: .combine)
    }
}

extension InfoRow where Trailing == EmptyView {
    init(_ title: String, subtitle: String? = nil, symbol: String? = nil, tint: Color = Palette.secondary, ltrSubtitle: Bool = false) {
        self.init(title: title, subtitle: subtitle, symbol: symbol, tint: tint, ltrSubtitle: ltrSubtitle) { EmptyView() }
    }
}
