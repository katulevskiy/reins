import SwiftUI

// The pieces the grant screens, the resume sheet, "New grant" and a connection's page share.

/// A choice among a few ("24 hours", "Read emails"): the shared `SelectChip`, with an optional small icon before its
/// words ("Specific senders", "All mail"). Plays the selection feedback.
struct OptionChip: View {
    var title: String
    var selected: Bool
    var symbol: String?
    var action: () -> Void

    init(_ title: String, selected: Bool, symbol: String? = nil, action: @escaping () -> Void) {
        self.title = title
        self.selected = selected
        self.symbol = symbol
        self.action = action
    }

    var body: some View {
        if let symbol {
            SelectChip(title: title, selected: selected, leading: {
                Image(systemName: symbol)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(selected ? SelectChipLook.check : Palette.ink2)
            }, action: action)
        } else {
            SelectChip(title: title, selected: selected, action: action)
        }
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

/// A text field in the grant forms and sign-in: a soft rounded well, mono for addresses.
struct FieldWell: ViewModifier {
    var mono = false
    /// What was typed is wrong: the ring turns red (the meta line below says why).
    var error = false

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)
        content
            .font(mono ? RFont.mono(15) : RFont.sans(16))
            .foregroundStyle(Palette.ink)
            .padding(.horizontal, 14)
            .padding(.vertical, 13)
            .background(Palette.paperRaised, in: shape)
            .overlay(shape.strokeBorder(error ? Palette.danger : Palette.rule, lineWidth: error ? 1.5 : 1))
    }
}

extension View {
    func fieldWell(mono: Bool = false, error: Bool = false) -> some View { modifier(FieldWell(mono: mono, error: error)) }
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

// MARK: Ended grants

/// A grant that ended (the Trust board, 05): its dimmed plate and avatar, the summary, who and how it ended, then
/// Resume and a round Delete.
struct EndedGrantRow: View {
    var grant: GrantView
    var onOpen: () -> Void
    var onResume: () -> Void
    var onDelete: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            GrantMini(service: grant.service, symbol: ActionKind.of(grant.action).symbol, connectionId: grant.connectionId,
                      label: grant.connectionLabel, size: 36, dim: true)
            VStack(alignment: .leading, spacing: 0) {
                Button(action: onOpen) {
                    VStack(alignment: .leading, spacing: 2) {
                        AskRunsText(runs: GrantWords.runs(grant.summary), size: TextRole.row.size)
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                        Text(verbatim: EndedGroup.line(grant))
                            .textRole(.sub)
                            .foregroundStyle(Palette.inkMeta)
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("ended:\(grant.id)")
                HStack(spacing: 8) {
                    Button(action: onResume) {
                        Label { Text("Resume") } icon: { GlyphIcon(.refresh, size: 15) }
                    }
                    .buttonStyle(SignalButtonStyle(.soft, size: .small))
                    .fixedSize()
                    .accessibilityIdentifier("resume:\(grant.id)")
                    RoundHeaderButton(glyph: .trash, label: "Delete", size: 42, action: onDelete)
                        .accessibilityIdentifier("delete:\(grant.id)")
                }
                .padding(.top, 10)
            }
        }
        .padding(.vertical, 14)
    }
}
