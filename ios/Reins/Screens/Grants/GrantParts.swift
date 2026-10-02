import SwiftUI

// The pieces the grant screens, the resume sheet, "New grant" and a connection's page share.

/// A choice among a few ("24 hours", "Read emails"): soft accent when chosen. Plays the selection feedback.
struct OptionChip: View {
    var title: String
    var selected: Bool
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    init(_ title: String, selected: Bool, action: @escaping () -> Void) {
        self.title = title
        self.selected = selected
        self.action = action
    }

    var body: some View {
        Button {
            action()
            feedback.play(.selection)
        } label: {
            Text(title)
                .font(RFont.sans(14.5, .medium))
                .foregroundStyle(selected ? Palette.accent : Palette.text)
                .lineLimit(1)
                .padding(.horizontal, 15)
                .frame(minHeight: 38)
                .background(selected ? Palette.accentSoft : Palette.controlFill, in: Capsule())
                .overlay(Capsule().strokeBorder(selected ? Palette.accent.opacity(0.6) : .clear, lineWidth: 0.75))
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? [.isSelected] : [])
    }
}

/// Lays chips out in rows, wrapping when a row is full.
struct ChipFlow: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
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
        var rows: [Row] = [Row()]
        for i in subviews.indices {
            let size = subviews[i].sizeThatFits(.unspecified)
            let needed = rows[rows.count - 1].items.isEmpty ? size.width : rows[rows.count - 1].width + spacing + size.width
            if needed > width, !rows[rows.count - 1].items.isEmpty {
                rows.append(Row())
            }
            var row = rows[rows.count - 1]
            row.width = row.items.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.items.append(i)
            rows[rows.count - 1] = row
        }
        return rows.filter { !$0.items.isEmpty }
    }
}

/// A small label tinted with a colour ("Reads", "Ends soon", "Active").
struct TintedTag: View {
    var text: String
    var tint: Color?
    var mono = false

    init(_ text: String, tint: Color? = nil, mono: Bool = false) {
        self.text = text
        self.tint = tint
        self.mono = mono
    }

    var body: some View {
        Text(text)
            .font(mono ? RFont.mono(12, .medium) : RFont.sans(12.5, .medium))
            .foregroundStyle(tint.map { $0.opacity(0.9) } ?? Palette.secondary)
            .lineLimit(1)
            .padding(.horizontal, 10)
            .frame(minHeight: 26)
            .background(tint.map { $0.opacity(0.1) } ?? Palette.controlFill, in: Capsule())
    }
}

/// A heading above a form group in the grant forms ("FOR WHICH AI?").
struct FormLabel: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text.uppercased())
            .font(RFont.sans(12.5, .medium))
            .tracking(0.6)
            .foregroundStyle(Palette.secondary)
            .accessibilityAddTraits(.isHeader)
    }
}

/// A text field in the grant forms and sign-in: a soft rounded well, mono for addresses.
struct FieldWell: ViewModifier {
    var mono = false

    func body(content: Content) -> some View {
        content
            .font(mono ? RFont.mono(15) : RFont.sans(16))
            .foregroundStyle(Palette.text)
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

extension View {
    func fieldWell(mono: Bool = false) -> some View { modifier(FieldWell(mono: mono)) }
}

/// A message under a form: what is wrong, or what went well.
struct FormBanner: View {
    enum Kind { case error, info }
    var text: String
    var kind: Kind = .error

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: kind == .error ? "exclamationmark.circle.fill" : "checkmark.circle.fill")
                .font(.system(size: 16, weight: .semibold))
            Text(text).font(RFont.sans(14.5)).frame(maxWidth: .infinity, alignment: .leading)
        }
        .foregroundStyle(kind == .error ? Palette.danger : Palette.success)
        .padding(12)
        .background((kind == .error ? Palette.danger : Palette.success).opacity(0.1), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .combine)
    }
}

// MARK: Grant rows

/// A running permission: the time left as a pie and along the border, how much it was used, who and what.
struct GrantTile: View {
    var grant: GrantView
    var now: Int64

    var body: some View {
        let clock = grantClock(grant, now: now)
        let kind = ActionKind.of(grant.action)
        HStack(alignment: .center, spacing: 14) {
            ExpiryPie(clock: clock, size: 54)
            VStack(alignment: .leading, spacing: 6) {
                Text(untrusted(grant.summary))
                    .font(RFont.sans(16, .semibold))
                    .foregroundStyle(Palette.text)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                HStack(spacing: 7) {
                    ConnectionIcon(connectionId: grant.connectionId, label: grant.connectionLabel, size: 18)
                    Text(untrusted(grant.connectionLabel))
                        .font(RFont.sans(14, .medium))
                        .foregroundStyle(Palette.secondary)
                        .lineLimit(1)
                    TintedTag(GrantText.verb(grant.action), tint: ActionTile.color(kind))
                        .padding(.leading, 1)
                }
                UsesMeter(uses: Int(grant.uses), maxUses: grant.maxUses.map(Int.init))
                ConnectorTags(service: grant.service, account: grant.account)
                    .padding(.top, 1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if clock.soon {
                TintedTag("Ends soon", tint: Palette.danger)
            }
        }
        .padding(14)
        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(TimeOutline(fraction: clock.fraction, color: clockColor(clock), radius: 20))
        .contentShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        .accessibilityElement(children: .combine)
        .accessibilityHint("Opens the grant")
    }
}

/// A permission that ended, with the ways to start it again or to get rid of it.
struct EndedGrantRow: View {
    var grant: GrantView
    var onOpen: () -> Void
    var onResume: () -> Void
    var onDelete: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: grant.state == "revoked" ? "trash" : "clock")
                .font(.system(size: 18, weight: .medium))
                .foregroundStyle(Palette.tertiary)
                .frame(width: 44, height: 44)
                .background(Palette.controlFill, in: Circle())
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 5) {
                Button(action: onOpen) {
                    VStack(alignment: .leading, spacing: 5) {
                        Text(untrusted(grant.summary))
                            .font(RFont.sans(15.5, .medium))
                            .foregroundStyle(Palette.text)
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                        HStack(spacing: 6) {
                            ConnectionIcon(connectionId: grant.connectionId, label: grant.connectionLabel, size: 18)
                            Text(untrusted(grant.connectionLabel) + " · " + GrantText.endedLine(grant))
                                .font(RFont.sans(13))
                                .foregroundStyle(Palette.secondary)
                                .lineLimit(1)
                        }
                        ConnectorTags(service: grant.service, account: grant.account)
                            .padding(.top, 2)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("ended:\(grant.id)")
                HStack(spacing: 8) {
                    Button(action: onResume) {
                        Label("Resume", systemImage: "arrow.clockwise")
                    }
                    .buttonStyle(SmallCapsuleStyle(kind: .secondary))
                    .accessibilityIdentifier("resume:\(grant.id)")
                    Button(role: .destructive, action: onDelete) {
                        Label("Delete", systemImage: "trash")
                    }
                    .buttonStyle(SmallCapsuleStyle(kind: .danger))
                    .accessibilityIdentifier("delete:\(grant.id)")
                }
                .padding(.top, 6)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
    }
}

/// A compact capsule button on a row ("Resume", "Delete", "New grant").
struct SmallCapsuleStyle: ButtonStyle {
    enum Kind { case primary, secondary, danger }
    var kind: Kind = .secondary

    func makeBody(configuration: Configuration) -> some View {
        let (fg, bg): (Color, Color) = switch kind {
        case .primary: (Palette.background, Palette.text)
        case .secondary: (Palette.text, Palette.controlFill)
        case .danger: (Palette.danger, Palette.danger.opacity(0.12))
        }
        configuration.label
            .font(RFont.sans(14, .semibold))
            .labelStyle(TightLabelStyle())
            .foregroundStyle(fg)
            .padding(.horizontal, 14)
            .frame(minHeight: 36)
            .background(bg, in: Capsule())
            .opacity(configuration.isPressed ? 0.7 : 1)
            .contentShape(Capsule())
    }
}

private struct TightLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 6) {
            configuration.icon.font(.system(size: 13, weight: .semibold))
            configuration.title
        }
    }
}

/// One operation a grant covered: what it was, when, and how it ended.
struct GrantUseRow: View {
    var entry: ActivityEntry

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            ActionTile(kind: ActionKind.of(entry.action), count: Int(entry.count), size: 40)
            VStack(alignment: .leading, spacing: 3) {
                Text(entry.headline)
                    .font(RFont.sans(15.5, .semibold))
                    .foregroundStyle(Palette.text)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                let detail = untrusted(entry.detail)
                if !detail.isEmpty {
                    Text(detail).font(RFont.sans(13.5)).foregroundStyle(Palette.secondary).lineLimit(2)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            VStack(alignment: .trailing, spacing: 6) {
                Text(GrantText.relative(entry.at)).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary).lineLimit(1)
                OutcomePill(text: Self.outcome(entry.outcome), tone: Self.tone(entry.outcome))
            }
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    static func outcome(_ o: String) -> String {
        switch o {
        case "denied": "Denied"
        case "error": "Failed"
        case "granted": "Granted"
        case "sent": "Sent"
        case "released": "Allowed"
        default: o.prefix(1).uppercased() + o.dropFirst()
        }
    }

    static func tone(_ o: String) -> OutcomePill.Tone {
        switch o {
        case "denied", "error", "failed": .bad
        case "granted": .accent
        default: .neutral
        }
    }
}
