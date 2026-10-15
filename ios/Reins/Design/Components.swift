import SwiftUI

// The shared building blocks, in Signal: paper, ink and hairlines; raised plates with a hairline ring for header
// controls and toasts; one violet; the type roles of Typography.swift.

/// "WAITING FOR YOU": a section label (mono capitals, tracked) above a group.
struct SectionHeader: View {
    var title: String
    var trailing: String?

    init(_ title: String, trailing: String? = nil) {
        self.title = title
        self.trailing = trailing
    }

    var body: some View {
        HStack {
            Text(title)
                .textRole(.label)
                .foregroundStyle(Palette.inkMeta)
            Spacer()
            if let trailing {
                Text(trailing).textRole(.meta).foregroundStyle(Palette.inkMeta)
            }
        }
        .padding(.horizontal, 4)
        .accessibilityAddTraits(.isHeader)
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

/// A compact pill button on a row ("Review").
struct PillButtonStyle: PrimitiveButtonStyle {
    var kind: SignalButtonKind = .violet

    func makeBody(configuration: Configuration) -> some View {
        DefaultTapStyle(look: SignalButtonLook(kind: kind, size: .small, fill: false)).makeBody(configuration: configuration)
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

/// A header chip with an icon and text ("Manual", "Integrations"): `paper-raised` with a hairline ring, or filled in its
/// tint.
struct GlassPill: View {
    var symbol: String?
    var text: String
    var tint: Color? = nil
    var action: () -> Void

    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            HStack(spacing: 7) {
                if let symbol { Image(systemName: symbol).font(.system(size: 14, weight: .semibold)) }
                Text(text).font(RFont.sans(15, .medium)).lineLimit(1)
            }
            .foregroundStyle(tint == nil ? Palette.ink : .white)
            .padding(.horizontal, 14)
            .frame(height: HeaderControl.height)
            .background(tint ?? Palette.paperRaised, in: Capsule())
            .overlay {
                if tint == nil { Capsule().strokeBorder(Palette.rule, lineWidth: 1) }
            }
            .contentShape(Capsule())
        }
        .buttonStyle(PressDimsStyle())
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
                .textRole(.mood)
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
