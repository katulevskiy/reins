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

/// A section header: Signal's section label, mono capitals tracked 0.16 em in ink-meta.
struct GroupHeader: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .textRole(.label)
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

// MARK: Settings rows

/// A row's icon in the iOS Settings look: a 32-point rounded square in the row's colour with a white symbol. A
/// destructive row's is a red wash with a red symbol (`washed`).
struct SettingsTile: View {
    var symbol: String
    var tint: Color
    var washed = false

    var body: some View {
        RoundedRectangle(cornerRadius: 8, style: .continuous)
            .fill(washed ? tint.opacity(0.14) : tint)
            .frame(width: 32, height: 32)
            .overlay {
                Image(systemName: symbol)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(washed ? tint : Color.white)
            }
            .accessibilityHidden(true)
    }
}

/// What a settings row shows: its tile (or any leading view), the title and an optional line under it, then what
/// goes on the right (a value, a spinner) and the disclosure mark when it opens a page.
struct SettingsRowLabel<Leading: View, Trailing: View>: View {
    var title: String
    var subtitle: String?
    var titleColor: Color
    var chevron: Bool
    var leading: Leading
    var trailing: Trailing

    init(
        title: String, subtitle: String? = nil, titleColor: Color = Palette.text, chevron: Bool = true,
        @ViewBuilder leading: () -> Leading, @ViewBuilder trailing: () -> Trailing
    ) {
        self.title = title
        self.subtitle = subtitle
        self.titleColor = titleColor
        self.chevron = chevron
        self.leading = leading()
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: 12) {
            leading
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(RFont.sans(16, .medium))
                    .foregroundStyle(titleColor)
                    .lineLimit(1)
                if let subtitle {
                    Text(subtitle)
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.tertiary)
                        .lineLimit(1)
                        .environment(\.layoutDirection, .leftToRight)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trailing
            if chevron { Chevron() }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
    }
}

/// A row's value on the right, in the secondary colour (mono for versions and codes).
struct RowValue: View {
    var text: String
    var mono = false

    init(_ text: String, mono: Bool = false) {
        self.text = text
        self.mono = mono
    }

    var body: some View {
        Text(text)
            .font(mono ? RFont.mono(14) : RFont.sans(15))
            .foregroundStyle(Palette.secondary)
            .lineLimit(1)
    }
}

/// A row that opens a page: a coloured tile, a title, the value on the right, and the disclosure mark. VoiceOver
/// reads the title, then `accessibilityValue` (the value spelled out when it is only a number).
struct SettingsLinkRow: View {
    var title: String
    var subtitle: String?
    var value: String?
    var symbol: String
    var tint: Color
    var id: String
    var spokenValue: String?
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    init(
        title: String, subtitle: String? = nil, value: String? = nil, symbol: String, tint: Color, id: String,
        spokenValue: String? = nil, action: @escaping () -> Void
    ) {
        self.title = title
        self.subtitle = subtitle
        self.value = value
        self.symbol = symbol
        self.tint = tint
        self.id = id
        self.spokenValue = spokenValue
        self.action = action
    }

    var body: some View {
        Button {
            action()
            // The page or sheet it opens has the sound.
            feedback.defaultTap()
        } label: {
            SettingsRowLabel(title: title, subtitle: subtitle) {
                SettingsTile(symbol: symbol, tint: tint)
            } trailing: {
                if let value { RowValue(value) }
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(subtitle.map { "\(title), \($0)" } ?? title)
        .accessibilityValue(spokenValue ?? value ?? "")
        .accessibilityIdentifier(id)
        .cardRow()
    }
}

/// A red row (Sign out, Delete account, Disconnect): a red-tinted tile, a red title, no chevron.
struct DestructiveRow: View {
    var title: String
    var symbol: String
    var id: String
    var disabled = false
    var action: () -> Void

    var body: some View {
        Button(role: .destructive, action: action) {
            SettingsRowLabel(title: title, titleColor: Palette.danger, chevron: false) {
                SettingsTile(symbol: symbol, tint: Palette.danger, washed: true)
            } trailing: {
                EmptyView()
            }
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .accessibilityLabel(title)
        .accessibilityIdentifier(id)
        .cardRow()
    }
}

/// The disclosure mark at the end of a row that opens a page.
struct Chevron: View {
    var body: some View {
        Image(systemName: "chevron.right")
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(Palette.tertiary)
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
                        .foregroundStyle(open ? Palette.accent : Palette.tertiary)
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
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .textCase(nil)
                    .transition(.opacity)
                    .accessibilityIdentifier(id.map { "\($0)Help" } ?? "helpText")
            }
        }
    }
}
