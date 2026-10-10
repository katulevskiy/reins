import SwiftUI

// The shared building blocks. Surfaces follow Zeron: plain cards on the background, Liquid Glass for floating
// controls (buttons over content, the tab bar, toasts); one violet accent; Geist type.

/// "WAITING FOR YOU": a small uppercase heading above a group.
struct SectionHeader: View {
    var title: String
    var trailing: String?

    init(_ title: String, trailing: String? = nil) {
        self.title = title
        self.trailing = trailing
    }

    var body: some View {
        HStack {
            Text(title.uppercased())
                .font(RFont.sans(12.5, .medium))
                .tracking(0.6)
                .foregroundStyle(Palette.secondary)
            Spacer()
            if let trailing {
                Text(trailing).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
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
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: radius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: radius, style: .continuous)
                    .strokeBorder(stroke ?? Palette.hairline, lineWidth: stroke == nil ? 0.5 : 1.5)
            )
    }
}

/// The integration a request uses ("Gmail"), tinted with the accent.
struct ServiceTag: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.sans(13, .medium))
            .foregroundStyle(Palette.accent)
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(Palette.accentSoft, in: Capsule())
            .lineLimit(1)
    }
}

/// An address or account ("me@gmail.com"), in mono.
struct AccountTag: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.mono(12.5))
            .foregroundStyle(Palette.secondary)
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(Palette.controlFill, in: Capsule())
            .lineLimit(1)
            .truncationMode(.middle)
            .environment(\.layoutDirection, .leftToRight)
    }
}

/// An outcome ("Allowed", "Denied", "Sent"), coloured by how it went.
struct OutcomePill: View {
    enum Tone { case neutral, good, bad, accent }
    var text: String
    var tone: Tone = .neutral

    var body: some View {
        let color: Color = switch tone {
        case .neutral: Palette.secondary
        case .good: Palette.success
        case .bad: Palette.danger
        case .accent: Palette.accent
        }
        Text(text)
            .font(RFont.sans(12.5, .medium))
            .foregroundStyle(color)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(tone == .neutral ? Palette.controlFill : color.opacity(0.13), in: Capsule())
            .lineLimit(1)
    }
}

/// The operation's icon on a tinted tile; several things at once show a stack and how many.
struct ActionTile: View {
    var kind: ActionKind
    var count: Int = 1
    var size: CGFloat = 44
    var tint: Color? = nil

    static func color(_ kind: ActionKind) -> Color {
        switch kind {
        case .search, .list: Palette.search
        case .write, .send: Palette.send
        case .read, .upload: Palette.read
        case .grant: Palette.grant
        case .accounts: Palette.accounts
        case .pair, .join: Palette.pair
        case .other: Palette.secondary
        }
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

/// A small number on a tab or a row.
struct CountBadge: View {
    var count: Int
    var tint: Color = Palette.accent

    var body: some View {
        if count > 0 {
            Text(count > 99 ? "99+" : "\(count)")
                .font(RFont.fixedSans(11, .semibold))
                .foregroundStyle(.white)
                .padding(.horizontal, 6)
                .padding(.vertical, 1.5)
                .background(tint, in: Capsule())
        }
    }
}

/// Big capsule buttons: Approve (the strong one), Deny (quiet), and destructive confirmations.
struct CapsuleButtonStyle: PrimitiveButtonStyle {
    enum Kind { case primary, secondary, accent, danger }
    var kind: Kind = .primary
    var height: CGFloat = 54

    func makeBody(configuration: Configuration) -> some View {
        DefaultTapStyle(look: CapsuleLook(kind: kind, height: height)).makeBody(configuration: configuration)
    }
}

private struct CapsuleLook: ButtonStyle {
    var kind: CapsuleButtonStyle.Kind
    var height: CGFloat

    func makeBody(configuration: Configuration) -> some View {
        let (fg, bg): (Color, Color) = switch kind {
        case .primary: (Palette.background, Palette.text)
        case .secondary: (Palette.text, Palette.controlFill)
        case .accent: (.white, Palette.accent)
        case .danger: (.white, Palette.danger)
        }
        configuration.label
            .font(RFont.sans(17, .semibold))
            .foregroundStyle(fg)
            .frame(maxWidth: .infinity, minHeight: height)
            .background(bg, in: Capsule())
            .opacity(configuration.isPressed ? 0.75 : 1)
            .scaleEffect(configuration.isPressed ? 0.98 : 1)
            .animation(.spring(duration: 0.2), value: configuration.isPressed)
    }
}

/// A compact pill button on a row ("Review").
struct PillButtonStyle: PrimitiveButtonStyle {
    var tint: Color = Palette.accent

    func makeBody(configuration: Configuration) -> some View {
        DefaultTapStyle(look: PillLook(tint: tint)).makeBody(configuration: configuration)
    }
}

private struct PillLook: ButtonStyle {
    var tint: Color

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(RFont.sans(15, .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
            .background(tint, in: Capsule())
            .opacity(configuration.isPressed ? 0.75 : 1)
    }
}

/// The height of everything beside a page title: the glass pills and the round buttons, so none stands taller.
enum HeaderControl {
    static let height: CGFloat = 40
}

/// A round Liquid Glass icon button (close, more, settings).
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
                .foregroundStyle(Palette.text)
                .frame(width: size, height: size)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: Circle())
        .accessibilityLabel(label)
    }
}

/// A Liquid Glass capsule with an icon and text ("Manual", "Integrations").
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
            .foregroundStyle(tint ?? Palette.text)
            .padding(.horizontal, 14)
            .frame(height: HeaderControl.height)
        }
        .buttonStyle(.plain)
        .glassEffect(tint.map { .regular.tint($0.opacity(0.18)).interactive() } ?? .regular.interactive(), in: Capsule())
    }
}

/// Nothing here yet.
struct EmptyState: View {
    var symbol: String
    var title: String
    var message: String? = nil

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 34, weight: .regular))
                .foregroundStyle(Palette.tertiary)
            Text(title).font(RFont.sans(17, .semibold)).foregroundStyle(Palette.text)
            if let message {
                Text(message)
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.secondary)
                    .multilineTextAlignment(.center)
            }
        }
        .padding(32)
        .frame(maxWidth: .infinity)
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
                .foregroundStyle(Palette.text)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 18)
                .padding(.vertical, 12)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular, in: Capsule())
        .padding(.horizontal, 24)
        .task {
            try? await Task.sleep(for: .seconds(4))
            onDismiss()
        }
    }
}

/// The large page title with optional trailing controls, as on the Android screens ("Activity  [Manual] [Integrations]").
struct PageHeader<Trailing: View>: View {
    var title: String
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            Text(title)
                .font(RFont.sans(34, .bold))
                .foregroundStyle(Palette.text)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 8)
            trailing
        }
    }
}

extension View {
    /// The app background behind a scrolling page.
    func pageBackground() -> some View {
        background(Palette.background.ignoresSafeArea())
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
