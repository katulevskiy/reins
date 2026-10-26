import SwiftUI

// The shared building blocks, in Next: paper, ink and hairlines; raised groups and plates with a hairline ring; one
// violet; the type roles of Typography.swift; the three page templates of the System board (hero, standard, quiet).

/// A section's header over its group (the System board's `sec`): an optional glyph, the title in `section` (13
/// semibold, never capitals) in ink-2, and counts or a control at the right. `quiet` is the smaller one of pushed pages
/// (12.5, ink-meta, closer to its rows).
struct SectionHeader<Trailing: View>: View {
    var title: String
    var glyph: Glyph?
    var quiet = false
    var trailing: Trailing

    init(_ title: String, glyph: Glyph? = nil, quiet: Bool = false, @ViewBuilder trailing: () -> Trailing) {
        self.title = title
        self.glyph = glyph
        self.quiet = quiet
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: 6) {
            if let glyph { GlyphIcon(glyph, size: quiet ? 14 : 15) }
            Text(title)
                .font(RFont.sans(quiet ? 12.5 : 13, .semibold))
                .tracking((quiet ? 12.5 : 13) * TextRole.section.trackingEm)
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 8)
            trailing
                .font(RFont.sans(13, .semibold))
                .monospacedDigit()
        }
        .foregroundStyle(quiet ? Palette.inkMeta : Palette.ink2)
        .padding(.horizontal, quiet ? 20 : 24)
        .padding(.top, quiet ? 20 : 22)
        .padding(.bottom, quiet ? 6 : 8)
        .frame(minHeight: 30)
    }
}

extension SectionHeader where Trailing == Text? {
    /// A header with a count or a word at the right ("Grants  6").
    init(_ title: String, glyph: Glyph? = nil, quiet: Bool = false, trailing: String? = nil) {
        self.init(title, glyph: glyph, quiet: quiet) { trailing.map { Text($0) } }
    }
}

/// A standard page's group (the System board's `grp`): rows on raised paper, radius 20, 16 pt in from the page's sides,
/// a hairline ring in light. Rows are separated by a hairline that starts after their leading tile (`inset`, 62) or at
/// their text (14 when they have none).
struct RaisedGroup<Content: View>: View {
    var inset: CGFloat = 62
    @ViewBuilder var content: Content
    @Environment(\.colorScheme) private var scheme
    @Environment(\.onWallpaper) private var onWallpaper
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        let glass = onWallpaper && !reduceTransparency
        VStack(spacing: 0) {
            // On glass the hairline is softer and stops short of both edges.
            Separated(
                inset: inset, trailingInset: glass ? 16 : 0,
                color: glass ? (scheme == .dark ? .white.opacity(0.09) : .black.opacity(0.08)) : Palette.rule, height: glass ? 0.5 : 1
            ) { content }
        }
        .frame(maxWidth: .infinity)
        .background {
            if onWallpaper {
                FrostedPane(radius: 20)
            } else {
                shape.fill(Palette.paperRaised)
                if scheme == .light { shape.strokeBorder(Palette.rule, lineWidth: 1) }
            }
        }
        .clipShape(shape)
        .padding(.horizontal, 16)
    }
}

/// A quiet page's group (the System board's `flat`): rows straight on paper, 20 pt in from the sides, a hairline above,
/// below and between them (after the tile, 44).
struct FlatGroup<Content: View>: View {
    var inset: CGFloat = 44
    @ViewBuilder var content: Content

    var body: some View {
        VStack(spacing: 0) {
            Separated(inset: inset, trailingInset: 0) { content }
        }
        .frame(maxWidth: .infinity)
        .overlay(alignment: .top) { Rectangle().fill(Palette.rule).frame(height: 1) }
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
        .padding(.horizontal, 20)
    }
}

/// Lays out its subviews in a column with a hairline between each two, `inset` from the leading edge.
struct Separated<Content: View>: View {
    var inset: CGFloat
    var trailingInset: CGFloat = 0
    var color: Color = Palette.rule
    var height: CGFloat = 1
    @ViewBuilder var content: Content

    var body: some View {
        Group(subviews: content) { subviews in
            ForEach(subviews.indices, id: \.self) { i in
                subviews[i]
                    .overlay(alignment: .top) {
                        if i > 0 {
                            Rectangle().fill(color).frame(height: height)
                                .padding(.leading, inset).padding(.trailing, trailingInset)
                        }
                    }
            }
        }
    }
}

/// A row's glyph on a 32 pt tile, radius 9: ink-2 on paper-sunken (nothing on these pages asks anything), or the red
/// wash of a destructive row.
struct GlyphTile: View {
    var glyph: Glyph
    var danger = false
    var side: CGFloat = 32

    init(_ glyph: Glyph, danger: Bool = false, side: CGFloat = 32) {
        self.glyph = glyph
        self.danger = danger
        self.side = side
    }

    var body: some View {
        RoundedRectangle(cornerRadius: side * 0.28, style: .continuous)
            .fill(danger ? Palette.dangerWash : Palette.paperSunken)
            .frame(width: side, height: side)
            .overlay { GlyphIcon(glyph, size: side * 0.53).foregroundStyle(danger ? Palette.danger : Palette.ink2) }
            .accessibilityHidden(true)
    }
}

/// A list row (the System board's `lrow`): a tile or any leading view, a title in `row`, an optional value in ink-2
/// (mono for codes and versions), anything else, and a chevron when it opens a page. No subtitles. At least 56 pt.
struct TileRow<Leading: View, Trailing: View>: View {
    var title: String
    var value: String?
    var mono = false
    var danger = false
    var chevron = true
    var leading: Leading
    var trailing: Trailing
    @Environment(\.dynamicTypeSize) private var typeSize

    init(
        _ title: String, value: String? = nil, mono: Bool = false, danger: Bool = false, chevron: Bool = true,
        @ViewBuilder leading: () -> Leading, @ViewBuilder trailing: () -> Trailing
    ) {
        self.title = title
        self.value = value
        self.mono = mono
        self.danger = danger
        self.chevron = chevron
        self.leading = leading()
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: 12) {
            leading
            // At the accessibility sizes the value goes under the title, so neither is broken mid-word.
            if typeSize.isAccessibilitySize {
                VStack(alignment: .leading, spacing: 2) {
                    titleText
                    valueText
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                titleText.frame(maxWidth: .infinity, alignment: .leading)
                valueText
            }
            trailing
            if chevron { RowChevron() }
        }
        .padding(.vertical, 9)
        .padding(.horizontal, 14)
        .frame(minHeight: 56)
        .contentShape(Rectangle())
    }

    private var titleText: some View {
        Text(title)
            .textRole(.row)
            .foregroundStyle(danger ? Palette.danger : Palette.ink)
    }

    @ViewBuilder private var valueText: some View {
        if let value {
            Text(value)
                .font(mono ? RFont.mono(13) : RFont.sans(15))
                .foregroundStyle(Palette.ink2)
                .lineLimit(1)
        }
    }
}

extension TileRow where Leading == GlyphTile, Trailing == EmptyView {
    init(_ title: String, glyph: Glyph, value: String? = nil, mono: Bool = false, danger: Bool = false, chevron: Bool = true) {
        self.init(title, value: value, mono: mono, danger: danger, chevron: chevron) { GlyphTile(glyph, danger: danger) } trailing: { EmptyView() }
    }
}

extension TileRow where Leading == GlyphTile {
    init(
        _ title: String, glyph: Glyph, value: String? = nil, mono: Bool = false, danger: Bool = false, chevron: Bool = true,
        @ViewBuilder trailing: () -> Trailing
    ) {
        self.init(title, value: value, mono: mono, danger: danger, chevron: chevron, leading: { GlyphTile(glyph, danger: danger) }, trailing: trailing)
    }
}

/// The chevron at the end of a row that opens a page: 14 pt, ink-faint.
struct RowChevron: View {
    var body: some View {
        GlyphIcon(.chevronRight, size: 14).foregroundStyle(Palette.inkFaint).accessibilityHidden(true)
    }
}

/// A rounded surface for a group of rows or a waiting item.
struct Card<Content: View>: View {
    var padding: CGFloat = 14
    var radius: CGFloat = 20
    var stroke: Color? = nil
    @ViewBuilder var content: Content

    var body: some View {
        content
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: radius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: radius, style: .continuous)
                    .strokeBorder(stroke ?? Palette.rule, lineWidth: stroke == nil ? 0.5 : 1.5)
            )
    }
}

/// The integration a request uses ("Gmail"), tinted with the accent.
struct ServiceTag: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.sans(13, .medium))
            .foregroundStyle(Palette.violet)
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(Palette.violetWash, in: Capsule())
            .lineLimit(1)
    }
}

/// An address or account ("me@gmail.com"), in mono.
struct AccountTag: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.mono(12.5))
            .foregroundStyle(Palette.ink2)
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(Palette.paperSunken, in: Capsule())
            .lineLimit(1)
            .truncationMode(.middle)
            .environment(\.layoutDirection, .leftToRight)
    }
}

/// An outcome ("Allowed", "Denied", "Sent"): a Signal pill, coloured by how it went and always with its word.
struct OutcomePill: View {
    enum Tone { case neutral, good, bad, accent }
    var text: String
    var tone: Tone = .neutral

    var body: some View {
        let pill: Pill.Tone = switch tone {
        case .neutral: .neutral
        case .good: .success
        case .bad: .danger
        case .accent: .violet
        }
        Pill(text, tone: pill)
    }
}

/// The operation's icon on a tinted tile; several things at once show a stack and how many.
struct ActionTile: View {
    var kind: ActionKind
    var count: Int = 1
    var size: CGFloat = 44
    var tint: Color? = nil

    /// The kind's ink (`FieldKind`); an operation with no kind is ink-2.
    static func color(_ kind: ActionKind) -> Color {
        FieldKind.of(kind)?.ink ?? Palette.ink2
    }

    var body: some View {
        let color = tint ?? Self.color(kind)
        let stacked = count > 1
        ZStack(alignment: .bottomTrailing) {
            ZStack {
                RoundedRectangle(cornerRadius: size * 0.32, style: .continuous).fill(color.opacity(0.13))
                if stacked {
                    icon(color).opacity(0.28).offset(x: -size * 0.13, y: -size * 0.13)
                    icon(color).opacity(0.55).offset(x: -size * 0.065, y: -size * 0.065)
                }
                icon(color).offset(x: stacked ? size * 0.03 : 0, y: stacked ? size * 0.03 : 0)
            }
            .frame(width: size, height: size)
            if stacked {
                Text(count > 99 ? "99+" : "\(count)")
                    .font(RFont.fixedSans(max(10, size * 0.27), .semibold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, size * 0.11)
                    .padding(.vertical, size * 0.02)
                    .background(color, in: Capsule())
                    .offset(x: size * 0.14, y: size * 0.14)
            }
        }
        .accessibilityHidden(true)
    }

    private func icon(_ color: Color) -> some View {
        Image(systemName: kind.symbol)
            .font(.system(size: size * 0.42, weight: .semibold))
            .foregroundStyle(color)
    }
}

/// A small number on a tab or a row: round for one digit, a capsule for two or three ("99+"), never cut.
struct CountBadge: View {
    var count: Int
    var tint: Color = Palette.violet

    /// "1" to "99", then "99+"; nil when there is nothing to count (no badge).
    static func text(_ count: Int) -> String? {
        count <= 0 ? nil : count > 99 ? "99+" : "\(count)"
    }

    var body: some View {
        if let text = Self.text(count) {
            Text(text)
                .font(RFont.fixedSans(11, .semibold))
                .monospacedDigit()
                .foregroundStyle(.white)
                .lineLimit(1)
                .padding(.horizontal, 5)
                // As wide as it is tall at least, so a single digit sits in a circle.
                .frame(minWidth: 18, minHeight: 18)
                .background(tint, in: Capsule())
                .fixedSize()
        }
    }
}

/// Big capsule buttons: Approve (the strong one, `inverse`), quiet ones (`soft`), Allow (`violet`) and destructive ones
/// (`danger` wash), drawn by Signal's button look.
struct CapsuleButtonStyle: PrimitiveButtonStyle {
    enum Kind { case primary, secondary, accent, danger }
    var kind: Kind = .primary
    var height: CGFloat = 54

    func makeBody(configuration: Configuration) -> some View {
        let look: SignalButtonKind = switch kind {
        case .primary: .inverse
        case .secondary: .soft
        case .accent: .violet
        case .danger: .danger
        }
        return DefaultTapStyle(look: SignalButtonLook(kind: look, height: height)).makeBody(configuration: configuration)
    }
}

/// The height of everything beside a page title: the glass pills and the round buttons, so none stands taller.
enum HeaderControl {
    static let height: CGFloat = 40
}

/// A round header button (close, more, settings): `paper-raised` with a hairline ring, as Signal draws them (no glass).
struct GlassIconButton: View {
    var symbol: String
    var size: CGFloat = HeaderControl.height
    var label: String
    var action: () -> Void

    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            Image(systemName: symbol)
                .font(.system(size: size * 0.4, weight: .semibold))
                .foregroundStyle(Palette.ink)
                .frame(width: size, height: size)
                .background(Palette.paperRaised, in: Circle())
                .overlay(Circle().strokeBorder(Palette.rule, lineWidth: 1))
                .contentShape(Circle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(label)
    }
}

/// Nothing here yet.
struct EmptyState: View {
    var symbol: String
    var title: String
    var message: String? = nil

    /// All quiet: the pond and one line in the mood serif ("All quiet on the water.").
    static func calm(_ mood: String) -> CalmState { CalmState(mood: mood) }

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 34, weight: .regular))
                .foregroundStyle(Palette.inkMeta)
            Text(title).font(RFont.sans(17, .semibold)).foregroundStyle(Palette.ink)
            if let message {
                Text(message)
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.ink2)
                    .multilineTextAlignment(.center)
            }
        }
        .padding(32)
        .frame(maxWidth: .infinity)
    }
}

/// The empty state of a quiet screen: the pond drawing and its mood line, centred, in ink-2.
struct CalmState: View {
    var mood: String

    var body: some View {
        VStack(spacing: 18) {
            Illustration(.pond)
                .frame(maxWidth: 320)
            Text(mood)
                .textRole(.body)
                .foregroundStyle(Palette.ink2)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 20)
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .combine)
    }
}

/// A one-line message floating over the content.
struct Toast: View {
    var text: String
    var onDismiss: () -> Void

    var body: some View {
        Button(action: onDismiss) {
            Text(text)
                .font(RFont.sans(15, .medium))
                .foregroundStyle(Palette.ink)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 18)
                .padding(.vertical, 12)
        }
        .buttonStyle(.plain)
        .background(Palette.paperRaised, in: Capsule())
        .overlay(Capsule().strokeBorder(Palette.rule, lineWidth: 1))
        .shadow(color: .black.opacity(0.18), radius: 15, y: 8)
        .padding(.horizontal, 24)
        .task {
            try? await Task.sleep(for: .seconds(4))
            onDismiss()
        }
    }
}

/// The page title in the serif with optional trailing controls ("Activity  [Manual] [Integrations]").
struct PageHeader<Trailing: View>: View {
    var title: String
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            Text(title)
                .textRole(.title)
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 8)
            trailing
        }
    }
}

/// A plain button whose content dims to 55 % while pressed and 40 % when disabled.
struct PressDimsStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label.pressDims(pressed: configuration.isPressed, enabled: enabled)
    }
}

extension View {
    /// The app background behind a scrolling page.
    func pageBackground() -> some View {
        background(Palette.paper.ignoresSafeArea())
    }

    /// Plays a tap's feedback alongside a control's own action; `.tap` is the default tap, which gives way to what the
    /// action plays itself (the Open of the page it pushes).
    func feedbackTap(_ event: FeedbackEvent = .tap, _ feedback: Feedback) -> some View {
        simultaneousGesture(TapGesture().onEnded {
            if event == .tap { feedback.defaultTap() } else { feedback.play(event) }
        })
    }
}

/// A section root draws its own large header, so a phone hides the navigation bar; in the split view (iPad, an
/// unfolded iPhone Duo) the bar stays, because it carries the button that brings a hidden sidebar back.
private struct RootNavigationBar: ViewModifier {
    @Environment(\.horizontalSizeClass) private var sizeClass

    func body(content: Content) -> some View {
        content.toolbar(sizeClass == .regular ? .automatic : .hidden, for: .navigationBar)
    }
}

extension View {
    func rootNavigationBar() -> some View { modifier(RootNavigationBar()) }
}
