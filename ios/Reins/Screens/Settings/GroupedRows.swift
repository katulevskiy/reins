import SwiftUI

// Inset-grouped lists in Reins's look (Settings, a grant's page, a connection's page): the page background behind,
// rows on elevated cards, Geist type, small uppercase headers.

extension View {
    /// An inset-grouped list on the page background.
    func reinsGrouped() -> some View {
        listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .listSectionSpacing(22)
            // The first group sits close under the navigation bar, not a whole section gap below it.
            .contentMargins(.top, 0, for: .scrollContent)
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

/// A section footer: the longer explanation of the group, behind a (?) until asked for. Every group's footer,
/// header'd or not, goes through this one, so no page opens on a paragraph.
struct GroupFooter: View {
    var text: String
    @State private var open = false
    @Environment(\.feedback) private var feedback

    init(_ text: String) { self.text = text }

    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
            Button {
                let next = !open
                withAnimation(.smooth(duration: 0.25)) { open = next }
                feedback.play(.expand(next))
            } label: {
                Text("?")
                    .font(RFont.sans(12.5, .semibold))
                    .foregroundStyle(open ? Palette.accent : Palette.secondary)
                    .frame(width: 22, height: 22)
                    .background(open ? Palette.accentSoft : Palette.controlFill, in: Circle())
                    .frame(width: 44, height: 30, alignment: .trailing)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(open ? "Hide help" : "Help")
            .accessibilityIdentifier("help")
            if open {
                Text(text)
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .transition(.opacity)
            }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .textCase(nil)
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
