import SwiftUI

// The pieces the Settings pages share (the Settings board, on the kit): the quiet page every pushed page is, its title,
// rows with a switch, small status tags, the round icon buttons at a row's end, form fields as rows.

/// A quiet page (the System board's third template): the system back button above, the title in `quietTitle`, then
/// quiet section headers and flat groups. No raised cards, no colour except status.
struct QuietPage<Content: View, Footer: View>: View {
    var title: String
    var lead: AnyView?
    @ViewBuilder var content: Content
    @ViewBuilder var footer: Footer

    init(_ title: String, lead: AnyView? = nil, @ViewBuilder content: () -> Content, @ViewBuilder footer: () -> Footer) {
        self.title = title
        self.lead = lead
        self.content = content()
        self.footer = footer()
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                QuietTitle(title, lead: lead)
                content
            }
            .padding(.bottom, 32)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollBounceBehavior(.basedOnSize)
        .safeAreaInset(edge: .bottom, spacing: 0) { footer }
        .pageBackground()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
    }
}

extension QuietPage where Footer == EmptyView {
    init(_ title: String, lead: AnyView? = nil, @ViewBuilder content: () -> Content) {
        self.init(title, lead: lead, content: content) { EmptyView() }
    }
}

/// A quiet page's title: 24 pt display, 20 pt in, just under the back button; an optional mark before it.
struct QuietTitle: View {
    var title: String
    var lead: AnyView?

    init(_ title: String, lead: AnyView? = nil) {
        self.title = title
        self.lead = lead
    }

    var body: some View {
        HStack(spacing: 12) {
            lead
            Text(title)
                .textRole(.quietTitle)
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
        }
        .padding(.horizontal, 20)
        .padding(.top, -4)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The buttons at the foot of a quiet form (the boards' `foot`): on paper, full width, above the home indicator.
struct QuietFooter<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        VStack(spacing: 8) { content }
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
            .frame(maxWidth: .infinity)
            .background(Palette.paper.ignoresSafeArea(edges: .bottom))
    }
}

extension View {
    /// A `TileRow` inside a `FlatGroup`: flush with the group's edges (flat rows have no side padding of their own).
    func flatRow() -> some View { padding(.horizontal, -14) }
}

/// A row with a glyph tile, its title and the switch at the end; the whole row is the switch for VoiceOver and taps.
/// Greyed with the switch when a switch above turns it off.
struct SwitchTileRow: View {
    var title: String
    var glyph: Glyph
    var isOn: Bool
    var enabled = true
    var id: String
    var onChange: (Bool) -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Toggle(isOn: Binding(get: { isOn }, set: { on in
            onChange(on)
            // After the change: turning the master off is silent, turning it on answers.
            feedback.play(.toggle(on))
        })) {
            HStack(spacing: 12) {
                GlyphTile(glyph)
                Text(title).textRole(.row).foregroundStyle(Palette.ink)
            }
        }
        .toggleStyle(SignalSwitchStyle())
        .frame(minHeight: 56)
        .disabled(!enabled)
        .accessibilityIdentifier(id)
    }
}

/// A small status tag: a glyph and one word in its colour (the boards' `stag`: Approves, Alerts).
struct StatusTag: View {
    var text: String
    var glyph: Glyph
    var tint: Color = Palette.success

    var body: some View {
        HStack(spacing: 4) {
            GlyphIcon(glyph, size: 14).accessibilityHidden(true)
            Text(text).font(RFont.sans(12.5, .semibold)).lineLimit(1)
        }
        .foregroundStyle(tint)
        .accessibilityElement(children: .combine)
    }
}

/// A round 40 pt glyph button with no plate, at a row's end (the boards' `qb`: copy, paste, remove).
struct RowIconButton: View {
    var glyph: Glyph
    var label: String
    var tint: Color = Palette.ink2
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            GlyphIcon(glyph, size: 18)
                .foregroundStyle(tint)
                .frame(width: 44, height: 44)
                .contentShape(Circle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(label)
    }
}

/// A form field as a row (the boards' `in`): its glyph says what it is, then the field.
struct InputRow<Field: View, Trailing: View>: View {
    var glyph: Glyph
    @ViewBuilder var field: Field
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 12) {
            GlyphIcon(glyph, size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20).accessibilityHidden(true)
            field
                .font(RFont.sans(16))
                .foregroundStyle(Palette.ink)
                .frame(maxWidth: .infinity, alignment: .leading)
            trailing
        }
        .frame(minHeight: 54)
    }
}

/// A quiet section header with its help behind a (?) at the right (after a count, when there is one); the help opens
/// under the header, above the rows.
struct QuietHelpHeader: View {
    var title: String
    var help: String
    var count: String? = nil
    @State private var open = false
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader(title, quiet: true) {
                HStack(spacing: 6) {
                    if let count { Text(count) }
                    Button {
                        let next = !open
                        withAnimation(reduceMotion ? .easeInOut(duration: 0.15) : .smooth(duration: 0.25)) { open = next }
                        feedback.play(.expand(next))
                    } label: {
                        GlyphIcon(.help, size: 15)
                            .foregroundStyle(open ? Palette.violet : Palette.inkMeta)
                            .frame(width: 44, height: 30)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .padding(.trailing, -14)
                    .accessibilityLabel(open ? "Hide help" : "Help")
                    .accessibilityIdentifier("help")
                }
            }
            if open {
                Text(help)
                    .textRole(.meta)
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 20)
                    .padding(.bottom, 8)
                    .transition(.opacity)
            }
        }
    }
}

/// A message under a group: the old `FormBanner`, 20 pt in like the flat rows.
struct QuietBanner: View {
    var text: String
    var kind: FormBanner.Kind = .error

    var body: some View {
        FormBanner(text: text, kind: kind)
            .padding(.horizontal, 20)
            .padding(.top, 12)
    }
}
