import SwiftUI
import UIKit

// The pieces Connections, its service pages, pairing and the vault share (the Connections, Connect and Vault boards):
// a section header you add to, the quiet title with its mark, form fields as rows, machine values that wrap, the focal
// card, the quiet danger row, code cells, and the scroll page both templates sit in.

/// A section you add to (the kit's `sec_add`): the header's glyph and title, its count, and a 30 pt round plus. The
/// plus lives on its own section, never on the page title.
struct PlusSectionHeader: View {
    var title: String
    var glyph: Glyph?
    var count: Int?
    var addLabel: String
    var addId: String
    var action: () -> Void

    var body: some View {
        SectionHeader(title, glyph: glyph) {
            HStack(spacing: 4) {
                if let count { Text("\(count)") }
                SmallRoundButton(glyph: .plus, label: addLabel, action: action)
                    .accessibilityIdentifier(addId)
            }
        }
        .padding(.top, -6)
        .padding(.bottom, -4)
    }
}

/// A 30 pt round button on paper (a section's plus): raised paper with a hairline ring. Its touch area is 44 pt.
struct SmallRoundButton: View {
    var glyph: Glyph
    var label: String
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            GlyphIcon(glyph, size: 16)
                .foregroundStyle(Palette.ink)
                .frame(width: 30, height: 30)
                .background(Palette.paperRaised, in: Circle())
                .overlay(Circle().strokeBorder(Palette.rule, lineWidth: 1))
                .frame(width: 44, height: 44)
                .contentShape(Circle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(label)
    }
}

/// A row's quiet icon button (copy, paste, trash, refresh): 40 pt, ink-2, no fill (the kit's `qb`).
struct QuietIconButton: View {
    var glyph: Glyph
    var label: String
    var tint: Color = Palette.ink2
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    init(_ glyph: Glyph, label: String, tint: Color = Palette.ink2, action: @escaping () -> Void) {
        self.glyph = glyph
        self.label = label
        self.tint = tint
        self.action = action
    }

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            GlyphIcon(glyph, size: 18)
                .foregroundStyle(tint)
                .frame(width: 40, height: 40)
                .contentShape(Circle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(label)
    }
}

/// A standard page's title (a tab root, the vault): display 30, 20 pt in, with what sits at its right.
struct StandardTitle<Trailing: View>: View {
    var title: String
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 8) {
            Text(title)
                .textRole(.title)
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityAddTraits(.isHeader)
            trailing
        }
        .padding(.horizontal, 20)
        .frame(minHeight: 44)
    }
}

extension StandardTitle where Trailing == EmptyView {
    init(_ title: String) {
        self.init(title: title) { EmptyView() }
    }
}

/// A quiet page's title (the kit's `page_title(…, 'quiet', lead)`): its mark (a 36 pt logo or avatar), then display 24.
/// A name from elsewhere arrives cleaned and capped; it is the page's own heading, one line.
struct MarkedTitle<Lead: View>: View {
    var title: String
    var titleId: String?
    @ViewBuilder var lead: Lead

    var body: some View {
        HStack(spacing: 12) {
            lead
            Text(title)
                .textRole(.quietTitle)
                .foregroundStyle(Palette.ink)
                .lineLimit(2)
                .minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier(titleId ?? "")
        }
        .padding(.horizontal, 20)
        .padding(.top, -2)
    }
}

extension MarkedTitle where Lead == EmptyView {
    init(_ title: String, titleId: String? = nil) {
        self.init(title: title, titleId: titleId) { EmptyView() }
    }
}

/// A logo or avatar on its round plate, as the boards draw marks beside titles and in rows.
struct MarkPlate<Content: View>: View {
    var size: CGFloat
    @ViewBuilder var content: Content

    var body: some View {
        content
            .frame(width: size, height: size)
            .background(Palette.plate, in: Circle())
            .overlay(Circle().strokeBorder(Palette.plateEdge, lineWidth: 1))
            .clipShape(Circle())
            .accessibilityHidden(true)
    }
}

/// The scroll page every Connections, service and vault page sits in: paper, the content top-aligned, room at the bottom
/// for the floating tab bar or a footer.
struct NextPage<Content: View>: View {
    var bottom: CGFloat = 40
    @ViewBuilder var content: Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                content
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.bottom, bottom)
        }
        .scrollDismissesKeyboard(.interactively)
        .pageBackground()
    }
}

extension View {
    /// A pushed page's bar: the system back button (a glass circle, which keeps the edge swipe back) and its own
    /// round buttons; the title is the page's own heading, so the bar shows none.
    func quietPageBar(_ title: String) -> some View {
        navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .principal) { Color.clear.frame(width: 1, height: 1).accessibilityHidden(true) } }
    }
}

/// A form field as a row (the kit's `inp`): its glyph says what it is, a short placeholder, anything at its right.
struct GlyphInputRow<Field: View, Trailing: View>: View {
    var glyph: Glyph
    @ViewBuilder var field: Field
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 12) {
            GlyphIcon(glyph, size: 18)
                .foregroundStyle(Palette.inkMeta)
                .frame(width: 20)
            field
                .font(RFont.sans(16))
                .foregroundStyle(Palette.ink)
                .frame(maxWidth: .infinity, alignment: .leading)
            trailing
        }
        .frame(minHeight: 54)
    }
}

extension GlyphInputRow where Trailing == EmptyView {
    init(glyph: Glyph, @ViewBuilder field: () -> Field) {
        self.glyph = glyph
        self.field = field()
        self.trailing = EmptyView()
    }
}

/// A group of input rows straight on paper (a quiet form, the kit's `flat` of `inp`): hairlines above, below and
/// between, after the glyph.
struct FlatFields<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        FlatGroup(inset: 32) { content }
    }
}

/// A group of input rows on raised paper (a standard form): 16 pt inside, the hairline after the glyph.
struct RaisedFields<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        RaisedGroup(inset: 48) {
            Group(subviews: content) { rows in
                ForEach(rows) { row in
                    row.padding(.leading, 16).padding(.trailing, 12)
                }
            }
        }
    }
}

/// A long machine value on a flat row that wraps, never cut (the kit's `mfield`): a public key, a vault reference.
struct MachineRow<Buttons: View>: View {
    var glyph: Glyph
    var value: String
    var valueId: String?
    @ViewBuilder var buttons: Buttons

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            GlyphIcon(glyph, size: 18)
                .foregroundStyle(Palette.inkMeta)
                .frame(width: 20)
                .padding(.top, 1)
            Text(verbatim: value)
                .font(RFont.mono(13))
                .lineSpacing(3)
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .environment(\.layoutDirection, .leftToRight)
                .textSelection(.enabled)
                .accessibilityIdentifier(valueId ?? "")
            HStack(spacing: 0) { buttons }
                .padding(.top, -10)
                .padding(.trailing, -8)
        }
        .padding(.vertical, 12)
    }
}

/// A quiet page's one focal point (the kit's `focal_card`): a hairline card, an ink tile with its grain, a display
/// title and a kind line, then glyph facts.
struct FocalCard<Facts: View>: View {
    var glyph: Glyph
    var title: String
    var sub: String
    var dashed = false
    @ViewBuilder var facts: Facts

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 24, style: .continuous)
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 14) {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill(Palette.inverse)
                    .overlay { Grain().clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous)) }
                    .overlay { GlyphIcon(glyph, size: 24).foregroundStyle(Palette.onInverse) }
                    .frame(width: 52, height: 52)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 4) {
                    Text(title)
                        .font(RFont.display(25))
                        .tracking(RFont.displayTracking(25))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                    Text(sub).font(RFont.sans(13.5)).foregroundStyle(Palette.inkMeta)
                }
            }
            .padding(.bottom, 14)
            Group(subviews: facts) { rows in
                ForEach(rows) { row in
                    row.overlay(alignment: .top) { Rectangle().fill(Palette.rule).frame(height: 1) }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 18)
        .padding(.bottom, 6)
        .overlay {
            if dashed {
                shape.strokeBorder(Palette.rule2, style: StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
            } else {
                shape.strokeBorder(Palette.rule2, lineWidth: 1)
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 16)
    }
}

/// A fact inside a focal card (the kit's `kfact`): a glyph, a value (mono for machine values), anything at its right.
struct FocalFact<Trailing: View>: View {
    var glyph: Glyph
    var value: String
    var mono = false
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 12) {
            GlyphIcon(glyph, size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20)
            Text(verbatim: value)
                .font(mono ? RFont.mono(13.5) : RFont.sans(15))
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
                .truncationMode(mono ? .middle : .tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                .environment(\.layoutDirection, .leftToRight)
            trailing
        }
        .frame(minHeight: 48)
    }
}

extension FocalFact where Trailing == EmptyView {
    init(glyph: Glyph, value: String, mono: Bool = false) {
        self.init(glyph: glyph, value: value, mono: mono) { EmptyView() }
    }
}

/// A rare, destructive action at the end of a page (the kit's `quiet_danger`): a raised row with the red tile, its
/// title in red and what happens under it. It is held, never tapped: the whole row is the hold.
struct QuietDangerRow: View {
    var title: String
    var glyph: Glyph = .trash
    var note: String
    var busy = false
    var onConfirm: () -> Void

    var body: some View {
        HoldRow(busy: busy, onConfirm: onConfirm) { progress in
            HStack(spacing: 12) {
                GlyphTile(glyph, danger: true)
                VStack(alignment: .leading, spacing: 1) {
                    Text(progress > 0 ? "Keep holding" : title).textRole(.row).foregroundStyle(Palette.danger)
                    Text(note).textRole(.sub).foregroundStyle(Palette.inkMeta).fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if busy { ProgressView() }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .frame(minHeight: 56)
        }
        .accessibilityLabel(title)
        .padding(.top, 22)
    }
}

/// A raised row you hold for `Motion.holdDuration` to confirm: the red wash sweeps across it while held, then
/// `onConfirm`. VoiceOver's double tap confirms at once (it is the deliberate act). Built like `HoldButton`, shaped as a
/// row.
struct HoldRow<Content: View>: View {
    var busy = false
    var onConfirm: () -> Void
    @ViewBuilder var content: (Double) -> Content
    @Environment(\.feedback) private var feedback
    @Environment(\.colorScheme) private var scheme
    @State private var started: Date?
    @State private var done = false
    @State private var ticker: Task<Void, Never>?

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        TimelineView(.animation(paused: started == nil)) { context in
            let progress = started.map { min(1, context.date.timeIntervalSince($0) / Motion.holdDuration) } ?? 0
            content(progress)
                .frame(maxWidth: .infinity)
                .background {
                    ZStack(alignment: .leading) {
                        Palette.paperRaised
                        GeometryReader { geo in
                            Rectangle().fill(Palette.dangerWash).frame(width: geo.size.width * progress)
                        }
                    }
                }
        }
        .overlay { if scheme == .light { shape.strokeBorder(Palette.rule, lineWidth: 1) } }
        .clipShape(shape)
        .contentShape(shape)
        .onLongPressGesture(minimumDuration: Motion.holdDuration, maximumDistance: 40) {
            confirm()
        } onPressingChanged: { pressing in
            if pressing { begin() } else if !done { end() }
        }
        .allowsHitTesting(!busy && !done)
        .accessibilityElement(children: .ignore)
        .accessibilityHint("Hold to confirm")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { confirm() }
        .padding(.horizontal, 16)
        .onDisappear { ticker?.cancel() }
    }

    private func begin() {
        guard started == nil else { return }
        started = .now
        feedback.play(.holdTick)
    }

    private func end() {
        started = nil
    }

    private func confirm() {
        guard !done, !busy else { return }
        done = true
        started = nil
        feedback.play(.holdConfirmed)
        onConfirm()
        Task {
            try? await Task.sleep(for: .seconds(1))
            done = false
        }
    }
}

/// The green dot of an account or server that works, with its word for VoiceOver.
struct OkDot: View {
    var label = "Connected"

    var body: some View {
        Circle()
            .fill(Palette.success)
            .frame(width: 8, height: 8)
            .background(Circle().fill(Palette.success.opacity(0.2)).frame(width: 14, height: 14))
            .frame(width: 14, height: 14)
            .accessibilityElement()
            .accessibilityLabel(label)
    }
}

/// One cell of a typed code (the kit's `ccell`): raised, a hairline ring (violet on the cell that takes the next
/// character), the character in mono 24.
struct CodeCell: View {
    var character: String
    var next = false
    var height: CGFloat = 56

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)
        Text(character.isEmpty ? " " : character)
            .font(RFont.mono(24, .medium))
            .foregroundStyle(Palette.ink)
            .frame(maxWidth: .infinity, minHeight: height)
            .background(Palette.paperRaised, in: shape)
            .overlay(shape.strokeBorder(next ? Palette.violet : Palette.rule2, lineWidth: next ? 2 : 1))
    }
}

/// A big code on a field, in mono, tabular, spaced (the kit's `bigcode`): what is compared between two screens.
struct BigCode: View {
    var code: String
    var size: CGFloat

    var body: some View {
        Text(verbatim: code)
            .font(RFont.mono(size, .medium))
            .tracking(size * 0.06)
            .monospacedDigit()
            .foregroundStyle(Palette.onField)
            .lineLimit(1)
            .minimumScaleFactor(0.5)
            .environment(\.layoutDirection, .leftToRight)
    }
}

/// Copies an address or a reference (not a secret) and says so for a moment.
@MainActor
func copyPlain(_ text: String, feedback: Feedback) {
    UIPasteboard.general.string = text
    feedback.play(.copied)
}
