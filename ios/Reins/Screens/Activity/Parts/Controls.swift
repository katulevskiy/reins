import SwiftUI

// Building blocks the activity, approval, pairing and upload screens share (the Android app's design/Components.kt
// pieces they use: Banner, Group, Hairline, CheckRow, SelectChip, Tag, ListRow).

/// An inline message on a wash of its colour: a notice, a warning or an error.
struct Banner: View {
    enum Kind { case notice, warning, error }
    var text: String
    var kind: Kind = .notice

    init(_ text: String, kind: Kind = .notice) {
        self.text = text
        self.kind = kind
    }

    var body: some View {
        let tone: Color = switch kind {
        case .notice: Palette.accent
        case .warning: Palette.warning
        case .error: Palette.danger
        }
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: kind == .notice ? "info.circle.fill" : "exclamationmark.triangle.fill")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(tone)
            Text(text)
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.text)
                .lineSpacing(2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(14)
        .background(tone.opacity(0.1), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .accessibilityElement(children: .combine)
    }
}

/// A grouped card with an optional small-caps header and footer.
struct GroupCard<Content: View>: View {
    var header: String?
    var footer: String?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let header {
                SectionHeader(header)
                    .padding(.leading, 12)
                    .padding(.top, 22)
                    .padding(.bottom, 8)
            }
            VStack(alignment: .leading, spacing: 0) { content }
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            if let footer {
                Text(footer)
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .padding(.horizontal, 16)
                    .padding(.top, 8)
            }
        }
        .padding(.horizontal, 16)
    }
}

/// A thin line between rows, inset from the leading edge.
struct Hairline: View {
    var inset: CGFloat = 16

    var body: some View {
        Rectangle()
            .fill(Palette.hairline)
            .frame(height: 0.75)
            .padding(.leading, inset)
            .accessibilityHidden(true)
    }
}

/// A round tick.
struct CheckMark: View {
    var checked: Bool
    var enabled: Bool = true

    var body: some View {
        ZStack {
            Circle()
                .strokeBorder(checked ? Color.clear : Palette.tertiary, lineWidth: 1.75)
                .background(Circle().fill(checked ? Palette.accent : Color.clear))
            if checked {
                Image(systemName: "checkmark")
                    .font(.system(size: 11.5, weight: .bold))
                    .foregroundStyle(.white)
            }
        }
        .frame(width: 24, height: 24)
        .opacity(enabled ? 1 : 0.45)
        .animation(.spring(duration: 0.22), value: checked)
    }
}

/// A whole-row tick target: the tick, then the label.
struct CheckRow<Label: View>: View {
    var checked: Bool
    var enabled: Bool = true
    var onChange: (Bool) -> Void
    @ViewBuilder var label: Label
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            let next = !checked
            onChange(next)
            feedback.play(.toggle(next))
        } label: {
            HStack(spacing: 12) {
                CheckMark(checked: checked, enabled: enabled)
                label.frame(maxWidth: .infinity, alignment: .leading)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityAddTraits(checked ? .isSelected : [])
        .accessibilityValue(checked ? "Ticked" : "Not ticked")
    }
}

/// One choice among a few ("1 hour", "24 hours"): filled in the accent when picked.
struct SelectChip: View {
    var title: String
    var selected: Bool
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            action()
            feedback.play(.selection)
        } label: {
            Text(title)
                .font(RFont.sans(14.5, .medium))
                .foregroundStyle(selected ? .white : Palette.text)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(selected ? Palette.accent : Palette.controlFill, in: Capsule())
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .animation(.smooth(duration: 0.2), value: selected)
    }
}

/// A small label on a tinted capsule ("Read only", "Looks like a code or a password").
struct TintTag: View {
    var text: String
    var tint: Color?
    var mono = false

    var body: some View {
        Text(text)
            .font(mono ? RFont.mono(12.5) : RFont.sans(12.5, .medium))
            .foregroundStyle(tint ?? Palette.secondary)
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(tint.map { $0.opacity(0.13) } ?? Palette.controlFill, in: Capsule())
            .lineLimit(1)
    }
}

/// A settings-style row inside a grouped card.
struct ListRow: View {
    var title: String
    var subtitle: String?
    var symbol: String?
    var chevron = false
    var monoSubtitle = false
    var action: (() -> Void)?

    var body: some View {
        if let action {
            Button(action: action) { content.contentShape(Rectangle()) }
                .buttonStyle(RowButtonStyle())
        } else {
            content
        }
    }

    private var content: some View {
        HStack(spacing: 14) {
            if let symbol {
                Image(systemName: symbol)
                    .font(.system(size: 17, weight: .medium))
                    .foregroundStyle(Palette.secondary)
                    .frame(width: 22)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(RFont.sans(16, .medium)).foregroundStyle(Palette.text).lineLimit(2)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(monoSubtitle ? RFont.mono(13) : RFont.sans(13))
                        .foregroundStyle(Palette.secondary)
                        .lineLimit(4)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if chevron {
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Palette.tertiary)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .frame(minHeight: 52)
        .textSelection(.enabled)
    }
}

/// A row that highlights while pressed, as list rows do.
struct RowButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(configuration.isPressed ? Palette.controlFill : Color.clear)
    }
}

/// A small caption above a value.
struct Caption: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text).font(RFont.sans(12, .medium)).foregroundStyle(Palette.tertiary)
    }
}

/// Text shown exactly as it is (arguments, a command): monospaced, on a tinted plate, selectable.
struct MonoBox: View {
    var text: String
    var lineLimit: Int?

    var body: some View {
        Text(text)
            .font(RFont.mono(13))
            .foregroundStyle(Palette.text)
            .lineLimit(lineLimit)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .environment(\.layoutDirection, .leftToRight)
            .textSelection(.enabled)
    }
}

/// Lays its children out in rows, wrapping onto the next row when one is full (chips, tags).
struct FlowRow: Layout {
    var spacing: CGFloat = 8
    var lineSpacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let height = rows.last.map { $0.y + $0.height } ?? 0
        let width = rows.map(\.width).max() ?? 0
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for index in row.items {
                let size = subviews[index].sizeThatFits(.unspecified)
                let fitted = CGSize(width: min(size.width, bounds.width), height: size.height)
                subviews[index].place(
                    at: CGPoint(x: x, y: bounds.minY + row.y + (row.height - fitted.height) / 2),
                    proposal: ProposedViewSize(fitted)
                )
                x += fitted.width + spacing
            }
        }
    }

    private struct Row {
        var items: [Int] = []
        var y: CGFloat = 0
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var current = Row()
        for (i, view) in subviews.enumerated() {
            let size = view.sizeThatFits(.unspecified)
            let w = min(size.width, width)
            if !current.items.isEmpty && current.width + spacing + w > width {
                rows.append(current)
                current = Row(y: current.y + current.height + lineSpacing)
            }
            current.width += (current.items.isEmpty ? 0 : spacing) + w
            current.height = max(current.height, size.height)
            current.items.append(i)
        }
        if !current.items.isEmpty { rows.append(current) }
        return rows
    }
}

extension View {
    /// Names a group for UI tests without hiding the names of what is inside it (an identifier on a plain container
    /// would be given to every element in it).
    func accessibilityContainer(_ identifier: String) -> some View {
        accessibilityElement(children: .contain).accessibilityIdentifier(identifier)
    }
}
