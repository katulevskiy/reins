import SwiftUI

// Grouped lists (Settings, a connection's page, the vault): rows on paper with rule hairlines between them, never
// cards; monochrome tiles, since nothing on these pages is asking anything. Section headers use `TextRole.section`
// (Sora, not capitals). Colour is the Signal tokens in Palette.swift.

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

    /// A row of a group: on paper, a rule hairline between it and the next.
    func cardRow() -> some View {
        listRowBackground(Color.clear)
            .listRowSeparatorTint(Palette.rule)
    }
}

/// A section header: `TextRole.section` (Sora 13 semibold, +0.015 em, not capitals) in ink-meta.
struct GroupHeader: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .textRole(.section)
            .foregroundStyle(Palette.inkMeta)
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
                    .foregroundStyle(open ? Palette.onInverse : Palette.ink2)
                    .frame(width: 22, height: 22)
                    .background(open ? Palette.inverse : Palette.paperSunken, in: Circle())
                    .frame(width: 44, height: 30, alignment: .trailing)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(open ? "Hide help" : "Help")
            .accessibilityIdentifier("help")
            if open {
                Text(text)
                    .textRole(.meta)
                    .foregroundStyle(Palette.ink2)
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
    var tint: Color = Palette.ink2
    /// Addresses and URLs read left to right whatever the language.
    var ltrSubtitle = false
    @ViewBuilder var trailing: Trailing
    @Environment(\.layoutDirection) private var direction

    init(
        title: String, subtitle: String? = nil, symbol: String? = nil, tint: Color = Palette.ink2, ltrSubtitle: Bool = false,
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
                Text(title).font(TextRole.row.font()).foregroundStyle(Palette.ink)
                if let subtitle {
                    Text(subtitle)
                        .textRole(.meta)
                        .foregroundStyle(Palette.inkMeta)
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
    init(_ title: String, subtitle: String? = nil, symbol: String? = nil, tint: Color = Palette.ink2, ltrSubtitle: Bool = false) {
        self.init(title: title, subtitle: subtitle, symbol: symbol, tint: tint, ltrSubtitle: ltrSubtitle) { EmptyView() }
    }
}

// MARK: Settings rows

/// The disclosure mark at the end of a row that opens a page.
struct Chevron: View {
    var body: some View {
        Image(systemName: "chevron.right")
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(Palette.inkMeta)
            .accessibilityHidden(true)
    }
}

/// A section header with a (?) right beside its words (not at the far end, where it reads as belonging to something
/// else) that shows the longer text under it. `id` names the help text for UI tests (`<id>Help`).
struct HelpHeader: View {
    var title: String
    var help: String
    var id: String? = nil
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 0) {
                GroupHeader(title)
                Button {
                    let next = !open
                    withAnimation(.smooth(duration: 0.25)) { open = next }
                    feedback.play(.expand(next))
                } label: {
                    Image(systemName: "questionmark.circle")
                        .font(.system(size: 16, weight: .regular))
                        .foregroundStyle(open ? Palette.violet : Palette.inkMeta)
                        .frame(width: 32, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(open ? "Hide help" : "Help")
                .accessibilityIdentifier("help")
                Spacer(minLength: 0)
            }
            if open {
                Text(help)
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .textCase(nil)
                    .transition(.opacity)
                    .accessibilityIdentifier(id.map { "\($0)Help" } ?? "helpText")
            }
        }
    }
}
