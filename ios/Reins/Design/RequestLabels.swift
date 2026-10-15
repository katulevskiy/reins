import SwiftUI

// What a request is, drawn the same in every Activity row, waiting card and entry (the Android app's
// ui/common/RequestLabels.kt): the thing it touches as the tile (a terminal, a file, a key, a brand's logo) with what it
// does as a badge on its corner, and the request itself as the title ("Run `git push --force origin main`"). The core
// decides all of it (`RequestLabel`); this only draws it. Which symbol and colour each provider and action gets is said
// here and nowhere else, so every screen agrees.

enum RequestLabelLook {
    /// The title as one line of plain text (copying, VoiceOver): the lead, the subject, then the tail.
    static func plain(_ label: RequestLabel) -> String {
        let lead = untrusted(label.lead)
        let subject = untrusted(label.subject)
        let gap = !lead.isEmpty && !subject.isEmpty ? " " : ""
        return lead + gap + subject + untrustedLine(label.tail)
    }

    /// The provider's own symbol when it has no logo: a terminal, a file, a key, a computer.
    static func providerSymbol(_ provider: String) -> String? {
        switch provider {
        case "terminal": "apple.terminal"
        case "file": "doc.text"
        case "secrets": "key.fill"
        case "ssh": "terminal"
        case "desktop": "desktopcomputer"
        default: nil
        }
    }

    /// The logo's asset key (`service-<key>`): "mcp" for any MCP server, else the integration's own; nil for the
    /// providers drawn with a symbol.
    static func providerAsset(_ provider: String) -> String? {
        if providerSymbol(provider) != nil || provider.trimmingCharacters(in: .whitespaces).isEmpty { return nil }
        return provider.hasPrefix(mcpPrefix) ? "mcp" : provider
    }

    /// The tile's mark colour: orange for the vault's secrets, teal for SSH, the text colour for the plain symbols,
    /// else the brand's own.
    static func providerTint(_ provider: String) -> Color {
        switch provider {
        case "secrets": Palette.kindPush
        case "ssh": Palette.kindRead
        case "terminal", "file", "desktop": Palette.ink
        default: serviceColor(providerAsset(provider) ?? provider)
        }
    }

    /// What the badge on the tile's corner shows.
    static func actionSymbol(_ action: String) -> String {
        switch action {
        case "run": "play.fill"
        case "read": "eye"
        case "send": "paperplane"
        case "delete": "trash"
        case "secrets": "key"
        case "push": "arrow.up"
        case "search": "magnifyingglass"
        case "grant": "checkmark.shield"
        case "ask": "questionmark"
        default: "pencil"
        }
    }

    /// The badge's colour: orange for what changes or runs, teal for reading, red for deleting, yellow for secrets,
    /// violet for a push, blue for a search, the accent for a permission, grey for a question.
    static func actionTint(_ action: String) -> Color {
        switch action {
        case "read": Palette.kindRead
        case "delete": Palette.danger
        case "secrets": Palette.warning
        case "push": violet
        case "grant": Palette.violet
        case "search": Palette.kindRead
        case "ask": Palette.ink2
        default: Palette.kindPush
        }
    }

    /// A lighter violet than the accent on dark, so a push's arrow reads on the small badge.
    static let violet = Palette.dynamic(0x5B43E8, 0xA99BFA)

    /// "Terminal", "SSH", "GitHub", "Linear" (an MCP server's own name).
    static func providerName(_ provider: String) -> String {
        switch provider {
        case "terminal": "Terminal"
        case "file": "File"
        case "secrets": "Secrets"
        case "ssh": "SSH"
        case "desktop": "Computer"
        default: serviceName(provider)
        }
    }

    /// "Run, Terminal", "Push, GitHub": the tile for VoiceOver.
    static func tileWords(_ label: RequestLabel) -> String {
        let action = label.action.prefix(1).uppercased() + label.action.dropFirst()
        return "\(action), \(providerName(label.provider))"
    }

    /// The tile's plate and edge (the same as a service's tile).
    static let plate = Palette.dynamic(0xFFFFFF, 0x1C1C20)
    static let edge = Palette.dynamic(0xE2E2E6, 0x2A2A2F)
}

/// The tile: the provider on a rounded plate, the action on a small badge over its bottom-right corner, ringed in `ring`
/// (the colour behind the tile) so the two read as one.
struct ProviderTile: View {
    var label: RequestLabel
    var size: CGFloat = 40
    var ring: Color = Palette.paper

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
        let mark = size * 0.5
        let tint = RequestLabelLook.providerTint(label.provider)
        ZStack {
            if let key = RequestLabelLook.providerAsset(label.provider), UIImage(named: "service-\(key)") != nil {
                Image("service-\(key)")
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .foregroundStyle(tint)
                    .frame(width: mark, height: mark)
            } else {
                Image(systemName: RequestLabelLook.providerSymbol(label.provider) ?? "link")
                    .font(.system(size: size * 0.42, weight: .medium))
                    .foregroundStyle(tint)
            }
        }
        .frame(width: size, height: size)
        .background(RequestLabelLook.plate, in: shape)
        .overlay(shape.strokeBorder(RequestLabelLook.edge, lineWidth: 1))
        .overlay(alignment: .bottomTrailing) {
            badge.offset(x: size * 0.12, y: size * 0.12)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(RequestLabelLook.tileWords(label))
        .accessibilityIdentifier("providerTile:\(label.provider):\(label.action)")
    }

    /// 20 pt for a 40 pt tile, its corners 7 pt.
    private var badge: some View {
        let side = size * 0.5
        let radius = side * 0.35
        let tint = RequestLabelLook.actionTint(label.action)
        return Image(systemName: RequestLabelLook.actionSymbol(label.action))
            .font(.system(size: side * 0.5, weight: .bold))
            .foregroundStyle(tint)
            .frame(width: side, height: side)
            .background(tint.mix(with: RequestLabelLook.plate, by: 0.78), in: RoundedRectangle(cornerRadius: radius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: radius + 2.5, style: .continuous)
                    .strokeBorder(ring, lineWidth: 2.5)
                    .padding(-2.5)
            )
            .accessibilityHidden(true)
    }
}

/// The title: the lead in the text colour, the subject set apart (a command or a path in mono, a name in semibold),
/// then the tail. Everything in it came from the AI, so it goes through `untrusted`.
struct LabelText: View {
    var label: RequestLabel
    var size: CGFloat = 15
    var lines: Int? = 2
    var color: Color = Palette.ink
    var terms: [String] = []

    var body: some View {
        Text(Self.attributed(label, size: size, color: color, terms: terms))
            .lineSpacing(size * 0.18)
            .lineLimit(lines)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityLabel(RequestLabelLook.plain(label))
    }

    /// `terms`: the words a search found, set apart in the cleaned title where a word starts with one.
    static func attributed(_ label: RequestLabel, size: CGFloat, color: Color, terms: [String] = []) -> AttributedString {
        let lead = untrusted(label.lead)
        let subject = untrusted(label.subject)
        var out = AttributedString(lead + (!lead.isEmpty && !subject.isEmpty ? " " : ""))
        out.font = RFont.sans(size, .medium)
        if !subject.isEmpty {
            var run = AttributedString(subject)
            run.font = label.code ? RFont.mono(size * 0.92, .medium) : RFont.sans(size, .semibold)
            out += run
        }
        var tail = AttributedString(untrustedLine(label.tail))
        tail.font = RFont.sans(size, .medium)
        out += tail
        out.foregroundColor = color
        TitleHighlight.apply(&out, terms: terms)
        return out
    }
}

/// What a waiting request touches with what it does on its corner, and who asks on the other (the AI's avatar over
/// the bottom-left corner, as the service tile had it).
struct PendingTile: View {
    var item: PendingItem
    var label: RequestLabel
    var size: CGFloat = 44

    var body: some View {
        let avatar = size / 2
        ProviderTile(label: label, size: size)
            .overlay(alignment: .bottomLeading) {
                ConnectionIcon(connectionId: item.connectionId, label: item.connectionLabel, size: avatar)
                    .overlay(Circle().strokeBorder(Palette.paper, lineWidth: 2.5).padding(-2.5))
                    .offset(x: -size * 0.14, y: size * 0.14)
            }
            .padding(.leading, size * 0.14)
            .padding(.bottom, size * 0.14)
            .accessibilityHidden(true)
    }
}
