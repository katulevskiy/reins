import LocalAuthentication
import SwiftUI

// The pieces the Activity and Grants pages share (the Android app's design/Cards.kt): the service tile with the AI's
// avatar on its corner, rings, the timer pill, the page dock with its buttons, the Once | 1 hour | 1 day | Custom
// control, value rows and the (?) help.

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
                    .foregroundStyle(Palette.ink2)
            }
        }
        .frame(width: size, height: size)
        .background(Self.fill, in: shape)
        .overlay(shape.strokeBorder(Self.edge, lineWidth: 1))
        .accessibilityHidden(true)
    }
}

/// A ring from twelve o'clock over a faint track.
struct ArcRing: View {
    var fraction: Double
    var color: Color
    var track: Color = Palette.paperSunken
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
/// deadline (an upload) it says how long ago it came. `onField`: the timer on a field or waiting card, a white ring and
/// mono digits that become a white pill with red digits for the last 15 seconds.
struct TimerPill: View {
    var createdAt: Int64
    var waitUntil: Int64?
    var onField = false

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

    @ViewBuilder
    private func label(_ u: Urgency?, text: String) -> some View {
        if onField {
            fieldLabel(u, text: text)
        } else {
            paperLabel(u, text: text)
        }
    }

    private func fieldLabel(_ u: Urgency?, text: String) -> some View {
        let hot = u?.urgent == true
        return HStack(spacing: 6) {
            if let u, !hot {
                ArcRing(fraction: u.stale ? 0 : u.fraction, color: .white, track: .white.opacity(0.28), lineWidth: 2.2)
                    .frame(width: 18, height: 18)
            }
            Text(text)
                .font(RFont.mono(13.5, .medium))
                .monospacedDigit()
                .lineLimit(1)
        }
        .foregroundStyle(hot ? Palette.fieldDelete : Palette.onField)
        .padding(.horizontal, hot ? 10 : 0)
        .frame(minHeight: 26)
        .background(hot ? Color.white : .clear, in: Capsule())
        .animation(.easeInOut(duration: Motion.fade), value: hot)
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(u.map { $0.stale ? "Stopped waiting" : "\($0.remainingText) left" } ?? TimeText.relative(createdAt))
        .accessibilityIdentifier("waitTimer")
    }

    private func paperLabel(_ u: Urgency?, text: String) -> some View {
        let hot = u?.urgent == true
        let stale = u?.stale == true
        let color = hot ? Palette.danger : stale ? Palette.inkMeta : Palette.ink2
        return HStack(spacing: 6) {
            if let u {
                ArcRing(fraction: u.stale ? 0 : u.fraction, color: color, track: hot ? Palette.danger.opacity(0.22) : Palette.paperSunken, lineWidth: 2.5)
                    .frame(width: 16, height: 16)
            }
            Text(text)
                .font(RFont.mono(13.5, .medium))
                .monospacedDigit()
                .lineLimit(1)
        }
        .foregroundStyle(color)
        .padding(.leading, u == nil ? 11 : 8)
        .padding(.trailing, 11)
        .padding(.vertical, 7)
        .background(hot ? Palette.danger.opacity(0.13) : Palette.paperSunken, in: Capsule())
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

/// The capsule buttons of cards and the page dock, drawn by Signal's button look: `neutral` is soft, `light` the inverse
/// action, `danger` the red wash; `warning` and `success` are solid in their colour.
struct DockButtonStyle: PrimitiveButtonStyle {
    enum Kind { case neutral, light, danger, warning, success }
    var kind: Kind = .neutral
    var height: CGFloat = 54
    var fontSize: CGFloat = 16.5

    func makeBody(configuration: Configuration) -> some View {
        DefaultTapStyle(look: DockButtonLook(kind: kind, height: height, fontSize: fontSize)).makeBody(configuration: configuration)
    }
}

private struct DockButtonLook: ButtonStyle {
    var kind: DockButtonStyle.Kind
    var height: CGFloat
    var fontSize: CGFloat
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        switch kind {
        case .neutral: signal(.soft, configuration)
        case .light: signal(.inverse, configuration)
        case .danger: signal(.danger, configuration)
        case .warning, .success:
            configuration.label
                .font(RFont.sans(fontSize, .semibold))
                .labelStyle(SignalLabelStyle())
                .foregroundStyle(Palette.dynamic(0xFFFFFF, 0x121215))
                .lineLimit(1)
                .frame(maxWidth: .infinity, minHeight: height)
                .background(kind == .warning ? Palette.warning : Palette.success, in: Capsule())
                .contentShape(Capsule())
                .pressDims(pressed: configuration.isPressed, enabled: enabled)
        }
    }

    private func signal(_ look: SignalButtonKind, _ configuration: Configuration) -> some View {
        SignalButtonLook(kind: look, height: height, fontSize: fontSize).makeBody(configuration: configuration)
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

/// "Once | 1 hour | 1 day | Custom ▾": a few periods side by side, the longer ones behind Custom, as a `Chooser`.
/// `Value` is the screen's own choice type; the identifiers are `<idPrefix>:<id>` and `<idPrefix>:custom` for the menu.
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
        let inlineSegments = inline.map { option in
            Chooser.Segment(
                id: "\(idPrefix):\(option.id)", title: option.label, selected: !customChosen && option.value == selection,
                enabled: option.enabled, action: { pick(option.value) }
            )
        }
        var menu = more.map { option in
            Chooser.MenuItem(id: "\(idPrefix):\(option.id)", title: option.label, enabled: option.enabled) { pick(option.value) }
        }
        if let onOther {
            menu.append(Chooser.MenuItem(id: "\(idPrefix):other", title: "Other time…") {
                onOther()
                feedback.play(.selection)
            })
        }
        let custom = Chooser.Segment(
            id: "\(idPrefix):custom", title: chosenMore?.label ?? "Custom", selected: customChosen || chosenMore != nil, more: true,
            menu: menu
        )
        return Chooser(inlineSegments + [custom])
    }

    private func pick(_ value: Value) {
        guard value != selection || customChosen else { return }
        onSelect(value)
        feedback.play(.selection)
    }
}

// MARK: Rows

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
                .foregroundStyle(Palette.inkMeta)
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Help")
        .popover(isPresented: $open) {
            Text(text)
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 280, alignment: .leading)
                .padding(16)
                .presentationCompactAdaptation(.popover)
        }
    }
}

/// A round radio mark: a violet ring when chosen.
struct RadioMark: View {
    var on: Bool

    var body: some View {
        // Signal's radio: a violet ring 7 pt thick around a white dot when on, a faint 1.5 pt ring when off.
        Circle()
            .strokeBorder(on ? Palette.violetFill : Palette.inkFaint, lineWidth: on ? 7 : 1.5)
            .background(on ? Circle().fill(.white) : nil)
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
            Text(title)
                .textRole(.section)
                .foregroundStyle(Palette.inkMeta)
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

/// Reins is broken and needs you (`design/next/iphone`, Next-Activity): a short amber field that runs into the top of Activity, the
/// only colour on the screen ("REINS · NOTIFICATIONS  Notifications are off  [Turn on]", "off" in semibold). Danger-tinted ones are red.
struct CompactBanner: View {
    var text: String
    var symbol: String
    var tint: Color = Palette.warning
    var button: String
    /// What is broken, for VoiceOver ("Reins · Notifications").
    var label = "Reins"
    var action: () -> Void

    var body: some View {
        // Next-Activity, 02: the amber field under the status bar, the glyph, a few words and the button.
        HStack(spacing: 12) {
            GlyphIcon(Self.glyph(symbol), size: 22)
                .accessibilityHidden(true)
            Text(text)
                .font(RFont.sans(16, .semibold))
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityLabel("\(label): \(text)")
            Button(button, action: action)
                .buttonStyle(SignalButtonStyle(.onField, size: .small, fill: false))
                .fixedSize()
        }
        .foregroundStyle(Palette.onField)
        .padding(.horizontal, 20)
        .padding(.top, 2)
        .padding(.bottom, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(alignment: .top) {
            // A top field: it runs up under the status bar and the Dynamic Island to the very edge, and only its bottom
            // corners round. It sits in a scroll view, where ignoring the safe area cannot reach above the content's
            // top, so the colour reaches up itself (far enough for a pull to refresh). Two bars stack: the one above
            // is drawn over this one's reach (`zIndex` at the call site), so they read as one field.
            FieldSurface(kind: tint == Palette.danger ? .delete : .warn)
                .clipShape(UnevenRoundedRectangle(bottomLeadingRadius: 26, bottomTrailingRadius: 26, style: .continuous))
                .padding(.top, -Self.reach)
        }
        .accessibilityElement(children: .contain)
    }

    /// How far a bar's colour reaches above it.
    static let reach: CGFloat = 400

    /// The kit's glyph for the symbol a caller names.
    static func glyph(_ symbol: String) -> Glyph {
        switch symbol {
        case let s where s.hasPrefix("bell"): .bell
        case let s where s.hasPrefix("lock"): .lockOpen
        case let s where s.hasPrefix("iphone"): .phone
        default: .warning
        }
    }

    /// The sentence with its last word as the semibold target: "Notifications are" **off**.
    static func runs(_ text: String) -> [AskText.Run] {
        guard let space = text.lastIndex(of: " ") else { return [AskText.Run(text: text, target: true)] }
        return [AskText.Run(text: String(text[...space])), AskText.Run(text: String(text[text.index(after: space)...]), target: true)]
    }
}
