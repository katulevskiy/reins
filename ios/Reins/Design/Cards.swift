import LocalAuthentication
import SwiftUI

// The pieces the Activity and Grants pages share (the Android app's design/Cards.kt): the service tile with the AI's
// avatar on its corner, rings, the timer pill, the page dock with its buttons, the Once | 1 hour | 1 day | Custom
// control, value rows and the (?) help.

/// The colour of a kind of operation on cards and tiles; a request that always asks is red.
enum KindTint {
    static func of(_ kind: ActionKind, danger: Bool = false) -> Color {
        FieldKind.of(kind, danger: danger)?.ink ?? Palette.secondary
    }

    /// The approval hero's wash: the kind's colour from 16 % at the top to 4 % at the bottom, with a 24 % border.
    static func wash(_ tint: Color) -> LinearGradient {
        LinearGradient(colors: [tint.opacity(0.16), tint.opacity(0.04)], startPoint: .top, endPoint: .bottom)
    }
}

/// An integration's logo on a rounded tile ("gmail", "github", "mcp:<id>"); `symbol` stands in when there is no
/// service (a new connection, another phone).
struct ServiceTile: View {
    var service: String
    var symbol: String? = nil
    var size: CGFloat = 44

    static var fill: Color { Palette.dynamic(0xFFFFFF, 0x1C1C20) }
    static var edge: Color { Palette.dynamic(0xE2E2E6, 0x2C2C31) }

    var body: some View {
        let key = service.hasPrefix(mcpPrefix) ? "mcp" : service
        let shape = RoundedRectangle(cornerRadius: size * 0.32, style: .continuous)
        ZStack {
            if !key.isEmpty, UIImage(named: "service-\(key)") != nil {
                Image("service-\(key)")
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .foregroundStyle(serviceColor(key))
                    .frame(width: size * 0.5, height: size * 0.5)
            } else {
                Image(systemName: symbol ?? "link")
                    .font(.system(size: size * 0.4, weight: .medium))
                    .foregroundStyle(Palette.secondary)
            }
        }
        .frame(width: size, height: size)
        .background(Self.fill, in: shape)
        .overlay(shape.strokeBorder(Self.edge, lineWidth: 1))
        .accessibilityHidden(true)
    }
}

/// The service tile with the AI's avatar over its bottom-left corner: who asks and where, in one mark.
struct ServiceMini: View {
    var service: String
    var symbol: String? = nil
    var connectionId: String
    var label: String
    /// A new client (a pairing) has no connection yet, so no icon was picked for it.
    var known = true
    var size: CGFloat = 44

    var body: some View {
        let avatar = size / 2
        ServiceTile(service: service, symbol: symbol, size: size)
            .overlay(alignment: .bottomLeading) {
                Group {
                    if known {
                        ConnectionIcon(connectionId: connectionId, label: label, size: avatar)
                    } else {
                        ConnectionAvatar(label: untrusted(label), pick: nil, size: avatar)
                    }
                }
                .overlay(Circle().strokeBorder(Palette.background, lineWidth: 2.5).padding(-2.5))
                .offset(x: -size * 0.14, y: size * 0.14)
            }
            .padding(.leading, size * 0.14)
            .padding(.bottom, size * 0.14)
            .accessibilityHidden(true)
    }
}

/// A ring from twelve o'clock over a faint track.
struct ArcRing: View {
    var fraction: Double
    var color: Color
    var track: Color = Palette.controlFill
    var lineWidth: CGFloat = 2.5

    var body: some View {
        ZStack {
            Circle().stroke(track, lineWidth: lineWidth)
            if fraction > 0 {
                Circle()
                    .trim(from: 0, to: min(max(fraction, 0), 1))
                    .stroke(color, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
                    .rotationEffect(.degrees(-90))
            }
        }
        .padding(lineWidth / 2)
        .accessibilityHidden(true)
    }
}

/// How long the AI still waits, as a pill with a ring that empties: red for the last `urgentSeconds`. Without a
/// deadline (an upload) it says how long ago it came.
struct TimerPill: View {
    var createdAt: Int64
    var waitUntil: Int64?

    var body: some View {
        if waitUntil == nil {
            LiveClock(interval: 15) { _ in
                label(nil, text: TimerPill.ago(createdAt))
            }
        } else {
            UrgencyClock(createdAt: createdAt, waitUntil: waitUntil) { u in
                label(u, text: u.map(TimerPill.text) ?? "")
            }
        }
    }

    /// "40s", "1:20", "Late" once the AI stopped waiting.
    static func text(_ u: Urgency) -> String {
        if u.stale { return "Late" }
        let s = u.remainingSeconds
        return s < 60 ? "\(s)s" : String(format: "%d:%02d", s / 60, s % 60)
    }

    /// "now", "5m", "3h", "2d".
    static func ago(_ createdAt: Int64, now: Int64 = Timers.nowSeconds()) -> String {
        let d = max(now - createdAt, 0)
        return d < 45 ? "now" : compactDuration(d)
    }

    private func label(_ u: Urgency?, text: String) -> some View {
        let hot = u?.urgent == true
        let stale = u?.stale == true
        let color = hot ? Palette.danger : stale ? Palette.tertiary : Palette.secondary
        return HStack(spacing: 6) {
            if let u {
                ArcRing(fraction: u.stale ? 0 : u.fraction, color: color, track: hot ? Palette.danger.opacity(0.22) : Palette.controlFill, lineWidth: 2.5)
                    .frame(width: 16, height: 16)
            }
            Text(text)
                .font(RFont.sans(14, .semibold))
                .monospacedDigit()
                .lineLimit(1)
        }
        .foregroundStyle(color)
        .padding(.leading, u == nil ? 11 : 8)
        .padding(.trailing, 11)
        .padding(.vertical, 7)
        .background(hot ? Palette.danger.opacity(0.13) : Palette.controlFill, in: Capsule())
        .animation(.easeInOut(duration: 0.4), value: hot)
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(u.map { $0.stale ? "Stopped waiting" : "\($0.remainingText) left" } ?? TimeText.relative(createdAt))
        .accessibilityIdentifier("waitTimer")
    }
}

/// The phone's own check: Face ID, Touch ID or Optic ID, else a lock (the passcode).
enum BiometricSymbol {
    static let name: String = {
        let context = LAContext()
        _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
        switch context.biometryType {
        case .faceID: return "faceid"
        case .touchID: return "touchid"
        case .opticID: return "opticid"
        default: return "lock"
        }
    }()
}

// MARK: Buttons

/// The capsule buttons of cards and the page dock.
struct DockButtonStyle: PrimitiveButtonStyle {
    enum Kind { case neutral, light, danger, warning, success }
    var kind: Kind = .neutral
    var height: CGFloat = 54
    var fontSize: CGFloat = 17

    func makeBody(configuration: Configuration) -> some View {
        DefaultTapStyle(look: DockButtonLook(kind: kind, height: height, fontSize: fontSize)).makeBody(configuration: configuration)
    }

    static func colors(_ kind: Kind) -> (fg: Color, bg: Color) {
        switch kind {
        case .neutral: (Palette.text, Palette.dynamic(0xE7E7EB, 0x232327))
        case .light: (Palette.background, Palette.text)
        case .danger: (Palette.danger, Palette.danger.opacity(0.15))
        case .warning: (Palette.dynamic(0xFFFFFF, 0x231A00), Palette.warning)
        case .success: (Palette.dynamic(0xFFFFFF, 0x04210F), Palette.success)
        }
    }
}

private struct DockButtonLook: ButtonStyle {
    var kind: DockButtonStyle.Kind
    var height: CGFloat
    var fontSize: CGFloat
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        let (fg, bg) = DockButtonStyle.colors(kind)
        configuration.label
            .font(RFont.sans(fontSize, .semibold))
            .labelStyle(DockLabelStyle())
            .foregroundStyle(fg)
            .lineLimit(1)
            .frame(maxWidth: .infinity, minHeight: height)
            .background(bg, in: Capsule())
            .contentShape(Capsule())
            .opacity(enabled ? (configuration.isPressed ? 0.75 : 1) : 0.5)
            .scaleEffect(configuration.isPressed ? 0.98 : 1)
            .animation(.spring(duration: 0.2), value: configuration.isPressed)
    }
}

private struct DockLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 7) {
            configuration.icon
            configuration.title
        }
    }
}

/// The white round + beside a page title (New grant, Integrations).
struct RoundPlusButton: View {
    var label: String
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            Image(systemName: "plus")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(Palette.background)
                .frame(width: HeaderControl.height, height: HeaderControl.height)
                .background(Palette.text, in: Circle())
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

// MARK: Page dock

extension View {
    /// A page's own actions pinned at the bottom, like the approval sheet's decision: a raised plate with rounded top
    /// corners over the content.
    func pageDock<Dock: View>(when shown: Bool = true, @ViewBuilder _ dock: () -> Dock) -> some View {
        let content = dock()
        return safeAreaInset(edge: .bottom, spacing: 0) {
            if shown {
                content
                    .padding(.horizontal, 14)
                    .padding(.top, 14)
                    .padding(.bottom, 10)
                    .frame(maxWidth: .infinity)
                    .background {
                        UnevenRoundedRectangle(topLeadingRadius: 30, topTrailingRadius: 30, style: .continuous)
                            .fill(PageDockColors.fill)
                            .overlay(
                                UnevenRoundedRectangle(topLeadingRadius: 30, topTrailingRadius: 30, style: .continuous)
                                    .strokeBorder(PageDockColors.edge, lineWidth: 1)
                                    .padding(.bottom, -2)
                            )
                            .ignoresSafeArea(edges: .bottom)
                    }
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
    }
}

enum PageDockColors {
    static let fill = Palette.dynamic(0xFFFFFF, 0x101012)
    static let edge = Palette.dynamic(0xE2E2E6, 0x1E1E22)
}

// MARK: Durations

/// "Once | 1 hour | 1 day | Custom ▾": a few periods side by side, the longer ones behind Custom. `Value` is the
/// screen's own choice type; the identifiers are `<idPrefix>:<id>` and `<idPrefix>:custom` for the menu.
struct DurationControl<Value: Hashable>: View {
    struct Option {
        var value: Value
        var label: String
        var id: String
        var enabled = true
    }

    var inline: [Option]
    var more: [Option]
    var selection: Value?
    /// Custom shows as chosen (a typed time).
    var customChosen = false
    var idPrefix: String
    /// "Other time…" at the end of the menu (a typed time), when the screen has one.
    var onOther: (() -> Void)?
    var onSelect: (Value) -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        let chosenMore = more.first { $0.value == selection && !customChosen }
        HStack(spacing: 0) {
            ForEach(Array(inline.enumerated()), id: \.offset) { _, option in
                let on = !customChosen && option.value == selection
                Button {
                    pick(option.value)
                } label: {
                    segment(on: on) { Text(option.label) }
                }
                .buttonStyle(.plain)
                .disabled(!option.enabled)
                .opacity(option.enabled ? 1 : 0.4)
                .accessibilityAddTraits(on ? .isSelected : [])
                .accessibilityIdentifier("\(idPrefix):\(option.id)")
            }
            Menu {
                ForEach(Array(more.enumerated()), id: \.offset) { _, option in
                    Button(option.label) { pick(option.value) }
                        .disabled(!option.enabled)
                        .accessibilityIdentifier("\(idPrefix):\(option.id)")
                }
                if let onOther {
                    Button("Other time…") {
                        onOther()
                        feedback.play(.selection)
                    }
                    .accessibilityIdentifier("\(idPrefix):other")
                }
            } label: {
                let on = customChosen || chosenMore != nil
                segment(on: on) {
                    HStack(spacing: 3) {
                        Text(chosenMore?.label ?? "Custom")
                        Image(systemName: "chevron.down").font(.system(size: 11, weight: .semibold))
                    }
                }
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("\(idPrefix):custom")
        }
        .padding(3)
        .background(Palette.dynamic(0xE7E7EB, 0x060606), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .animation(.smooth(duration: 0.2), value: selection)
    }

    private func pick(_ value: Value) {
        guard value != selection || customChosen else { return }
        onSelect(value)
        feedback.play(.selection)
    }

    private func segment<Label: View>(on: Bool, @ViewBuilder _ label: () -> Label) -> some View {
        label()
            .font(RFont.sans(13.5, .semibold))
            .foregroundStyle(on ? Palette.text : Palette.secondary)
            .lineLimit(1)
            .minimumScaleFactor(0.8)
            .frame(maxWidth: .infinity, minHeight: 36)
            .background(on ? Palette.dynamic(0xFFFFFF, 0x232327) : .clear, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
            .contentShape(Rectangle())
    }
}

// MARK: Rows

/// "When        Today, 14:14": a quiet label and its value, in a grouped card.
struct ValueRow: View {
    var label: String
    var value: String
    var mono = false
    /// The value is a way further (a chevron, and the label in full colour).
    var action: (() -> Void)?

    var body: some View {
        if let action {
            Button(action: action) { content }
                .buttonStyle(RowButtonStyle())
        } else {
            content
        }
    }

    private var content: some View {
        HStack(spacing: 8) {
            Text(label)
                .font(RFont.sans(16, action == nil ? .regular : .medium))
                .foregroundStyle(action == nil ? Palette.tertiary : Palette.text)
                .lineLimit(1)
            Spacer(minLength: 8)
            Text(value)
                .font(mono ? RFont.mono(14) : RFont.sans(15))
                .foregroundStyle(action == nil ? Palette.text : Palette.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            if action != nil {
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Palette.tertiary)
            }
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 52)
        .contentShape(Rectangle())
    }
}

/// A round (?) that shows a short explanation in a popover.
struct HelpButton: View {
    var text: String
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            open = true
            feedback.play(.open)
        } label: {
            Image(systemName: "questionmark.circle")
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(Palette.tertiary)
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Help")
        .popover(isPresented: $open) {
            Text(text)
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.text)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 280, alignment: .leading)
                .padding(16)
                .presentationCompactAdaptation(.popover)
        }
    }
}

/// A round radio mark: filled in the accent when chosen.
struct RadioMark: View {
    var on: Bool

    var body: some View {
        ZStack {
            Circle().strokeBorder(on ? Palette.accent : Palette.dynamic(0xB4B4BC, 0x4A4A52), lineWidth: 2)
            if on { Circle().fill(Palette.accent).frame(width: 11, height: 11) }
        }
        .frame(width: 22, height: 22)
        .animation(.spring(duration: 0.22), value: on)
        .accessibilityHidden(true)
    }
}

/// The small uppercase heading of a page section ("WAITING · 4"), with room for a control at the right.
struct PageSectionHeader<Trailing: View>: View {
    var title: String
    @ViewBuilder var trailing: Trailing

    init(_ title: String, @ViewBuilder trailing: () -> Trailing) {
        self.title = title
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: 8) {
            Text(title.uppercased())
                .font(RFont.sans(12.5, .semibold))
                .tracking(0.9)
                .foregroundStyle(Palette.tertiary)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 8)
            trailing
        }
        .padding(.leading, 22)
        .padding(.trailing, 16)
        .padding(.top, 18)
        .padding(.bottom, 8)
        .frame(minHeight: 28)
    }
}

extension PageSectionHeader where Trailing == EmptyView {
    init(_ title: String) {
        self.init(title) { EmptyView() }
    }
}

/// A one-line notice with one button ("Notifications are off  [Turn on]"): Activity's banners.
struct CompactBanner: View {
    var text: String
    var symbol: String
    var tint: Color = Palette.warning
    var button: String
    var action: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(tint)
                .accessibilityHidden(true)
            Text(text)
                .font(RFont.sans(14.5, .medium))
                .foregroundStyle(Palette.text)
                .lineLimit(1)
                .minimumScaleFactor(0.85)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(button, action: action)
                .font(RFont.sans(13.5, .semibold))
                .foregroundStyle(Palette.background)
                .padding(.horizontal, 12)
                .frame(minHeight: 30)
                .background(Palette.text, in: Capsule())
                .buttonStyle(.plain)
                .fixedSize()
        }
        .padding(.leading, 14)
        .padding(.trailing, 8)
        .padding(.vertical, 8)
        .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .accessibilityElement(children: .contain)
    }
}
