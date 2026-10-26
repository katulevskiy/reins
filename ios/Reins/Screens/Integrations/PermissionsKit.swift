import SwiftUI

// The Permissions pages' pieces (the Permissions board; quiet template): a quiet title with an optional leading mark
// and pill, quiet section headers whose explanations sit behind a (?), flat rows, amber warning lines, and the two
// buttons at the foot of a page.

/// A quiet page's title (24), with a leading mark (a service's logo) and a trailing pill ("Active").
struct PermTitle<Lead: View, Trail: View>: View {
    var title: String
    var mono = false
    @ViewBuilder var lead: Lead
    @ViewBuilder var trail: Trail

    var body: some View {
        HStack(spacing: 12) {
            lead
            Text(verbatim: title)
                .font(mono ? RFont.display(24) : TextRole.quietTitle.font())
                .tracking(TextRole.quietTitle.trackingEm * TextRole.quietTitle.size)
                .foregroundStyle(Palette.ink)
                .lineLimit(2)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 8)
            trail
        }
        .padding(.horizontal, 20)
        .padding(.top, 4)
    }
}

extension PermTitle where Lead == EmptyView, Trail == EmptyView {
    init(_ title: String) {
        self.init(title: title) { EmptyView() } trail: { EmptyView() }
    }
}

/// A quiet section header (the boards' `qsec`): glyph, title, a (?) when it has an explanation, a count at the right.
struct PermHeader: View {
    var title: String
    var glyph: Glyph?
    var help: String?
    var count: String?

    var body: some View {
        HStack(spacing: 6) {
            if let glyph { GlyphIcon(glyph, size: 14) }
            Text(title)
                .font(RFont.sans(12.5, .semibold))
                .tracking(12.5 * TextRole.section.trackingEm)
                .lineLimit(2)
                .accessibilityAddTraits(.isHeader)
            if let help { HelpButton(text: help) }
            Spacer(minLength: 8)
            if let count { Text(count).font(RFont.sans(13, .semibold)).monospacedDigit() }
        }
        .foregroundStyle(Palette.inkMeta)
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .padding(.bottom, 6)
        .frame(minHeight: 30)
    }
}

/// A flat row (the boards' `lrow` in `flat`): a leading mark, a title (mono for accounts and addresses), a value in
/// ink-2, a trailing control, a chevron when it opens something.
struct PermRow<Lead: View, Trail: View>: View {
    var title: String
    var mono = false
    var value: String?
    var titleColor: Color = Palette.ink
    var chevron = false
    @ViewBuilder var lead: Lead
    @ViewBuilder var trail: Trail

    var body: some View {
        HStack(spacing: 12) {
            lead
            Text(verbatim: title)
                .font(mono ? RFont.mono(14.5) : TextRole.row.font())
                .foregroundStyle(titleColor)
                .lineLimit(2)
                .environment(\.layoutDirection, mono ? .leftToRight : .leftToRight)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let value {
                Text(verbatim: value).font(RFont.sans(15)).foregroundStyle(Palette.ink2).lineLimit(1).monospacedDigit()
            }
            trail
            if chevron { RowChevron() }
        }
        .padding(.vertical, 8)
        .frame(minHeight: 56)
        .contentShape(Rectangle())
    }
}

extension PermRow where Trail == EmptyView {
    init(title: String, mono: Bool = false, value: String? = nil, titleColor: Color = Palette.ink, chevron: Bool = false, @ViewBuilder lead: () -> Lead) {
        self.init(title: title, mono: mono, value: value, titleColor: titleColor, chevron: chevron, lead: lead) { EmptyView() }
    }
}

/// An amber line that must stay in sight: a restriction, history that cannot be checked, what still asks.
struct PermWarning: View {
    var text: String
    var tint: Color = Palette.warning

    init(_ text: String, tint: Color = Palette.warning) {
        self.text = text
        self.tint = tint
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            GlyphIcon(.warning, size: 15).alignmentGuide(.firstTextBaseline) { $0[.bottom] - 2 }
            Text(verbatim: text).font(RFont.sans(14, .medium)).fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(tint)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

/// A line under a row or a title in ink-2: what is in force, why.
struct PermLine: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(verbatim: text)
            .textRole(.sub)
            .foregroundStyle(Palette.ink2)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Two equal buttons at the foot of a quiet page ("Temporary" · "Review", "Edit" · "Save", "Pause" · "Revoke").
struct PermFoot<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: 10) { content }
            .frame(height: 54)
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
            .background(Palette.paper.opacity(0.94).ignoresSafeArea(edges: .bottom))
    }
}

/// A glyph and a word as a button label.
struct PermGlyphLabel: View {
    var title: String
    var glyph: Glyph?

    init(_ title: String, glyph: Glyph? = nil) {
        self.title = title
        self.glyph = glyph
    }

    var body: some View {
        if let glyph {
            Label { Text(title) } icon: { GlyphIcon(glyph, size: 18) }
        } else {
            Text(title)
        }
    }
}

/// What the permission screens call the operations and modes, with their glyphs.
enum PermissionLook {
    static func glyph(_ c: PermissionCapability) -> Glyph {
        switch c {
        case .mailSearch: .search
        case .mailRead, .integrationRead: .eye
        case .mailSend: .send
        case .integrationWrite: .pencil
        }
    }

    /// The short word in a row's value: "Review", "Allow", "Block".
    static func word(_ m: PermissionMode) -> String {
        switch m {
        case .ask: "Review"
        case .auto: "Allow"
        case .block: "Block"
        }
    }

    static func glyph(_ m: PermissionMode) -> Glyph {
        switch m {
        case .ask: .person
        case .auto: .check
        case .block: .close
        }
    }

    static func glyph(_ p: AccountPermissionPreset) -> Glyph {
        switch p {
        case .reviewEveryRequest: .person
        case .readAssist: .eye
        case .custom: .settings
        }
    }
}

extension View {
    /// A row of a quiet page's list: no cell background or separators, the page's side margins.
    func permRow(top: CGFloat = 0, bottom: CGFloat = 0, sides: CGFloat = 20) -> some View {
        listRowInsets(EdgeInsets(top: top, leading: sides, bottom: bottom, trailing: sides))
            .listRowSeparator(.hidden)
            .listRowBackground(Color.clear)
    }
}
