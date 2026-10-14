import SwiftUI

// The header's Autopilot pill opens a compact switcher rather than the whole Autopilot page (the Android app's
// AutopilotQuick.kt). Release builds use the menu, a card dropped from under the pill; debug builds can compare it
// with a risk slider and with the page (Settings, Developer).

/// What the mode pill opens.
enum QuickStyle: String, CaseIterable, Identifiable {
    case menu, slider, page

    static let key = "autopilot.quick"

    var id: String { rawValue }

    var label: String {
        switch self {
        case .menu: "Menu"
        case .slider: "Slider"
        case .page: "Page"
        }
    }

    /// The stored choice in a debug build; always the menu in a release build.
    static func resolve(_ raw: String) -> QuickStyle {
        #if DEBUG
        return QuickStyle(rawValue: raw) ?? .menu
        #else
        return .menu
        #endif
    }
}

/// The mode pill with its switcher. The menu (`autopilotQuickMenu`, on the page under the pill) drops from the pill;
/// the slider is a popover; the page style opens the Autopilot page.
struct AutopilotQuickPill: View {
    var settings: AutopilotSettings
    /// The menu is open (the page that holds the pill draws it, see `autopilotQuickMenu`).
    @Binding var menuOpen: Bool

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @AppStorage(QuickStyle.key) private var styleRaw = QuickStyle.menu.rawValue
    @State private var ap = AutopilotModel()
    @State private var sliderOpen = false
    @State private var bypassAsk = false

    var body: some View {
        Group {
            switch QuickStyle.resolve(styleRaw) {
            case .menu:
                ActivityModePill(settings: settings) {
                    let next = !menuOpen
                    withAnimation(.spring(duration: 0.3, bounce: 0.2)) { menuOpen = next }
                    feedback.play(.expand(next))
                }
                // Where the menu drops from.
                .anchorPreference(key: QuickPillAnchor.self, value: .bounds) { $0 }
            case .slider:
                ActivityModePill(settings: settings) {
                    sliderOpen = true
                    feedback.play(.open)
                }
                .popover(isPresented: $sliderOpen, arrowEdge: .top) {
                    RiskSliderCard(
                        settings: settings,
                        modelReady: ap.modelReady,
                        onSet: { mode in Task { await ap.setMode(mode) } },
                        onBypass: {
                            sliderOpen = false
                            bypassAsk = true
                        },
                        onStop: { Task { await ap.stopBypass() } },
                        onMore: {
                            sliderOpen = false
                            model.show(.autopilot)
                        }
                    )
                    .presentationCompactAdaptation(.popover)
                }
            case .page:
                ActivityModePill(settings: settings) {
                    model.show(.autopilot)
                    feedback.defaultTap()
                }
            }
        }
        .onAppear { ap.bind(model) }
        .sheet(isPresented: $bypassAsk) {
            BypassSheet(who: nil) { length in
                feedback.quietClose()
                Task { await ap.startBypass(minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(bypassAsk)
    }
}

/// Where the header's mode pill is: the quick menu drops from under it.
struct QuickPillAnchor: PreferenceKey {
    static var defaultValue: Anchor<CGRect>? { nil }

    static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) {
        value = value ?? nextValue()
    }
}

extension View {
    /// The quick menu, drawn over this page under its mode pill while `isPresented`: the page steps back behind a
    /// dark scrim (a tap on it closes the menu).
    func autopilotQuickMenu(isPresented: Binding<Bool>) -> some View {
        modifier(AutopilotQuickMenu(isPresented: isPresented))
    }
}

/// The quick menu's place on the page: the scrim, the card under the pill with its right edge on the pill's, and the
/// bypass sheet its Bypass opens (the same sheet as the Autopilot page's: 15 min, 1 hour, 1 day or Custom, held to
/// confirm, then the owner's check).
struct AutopilotQuickMenu: ViewModifier {
    @Binding var isPresented: Bool

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.colorScheme) private var scheme
    @State private var ap = AutopilotModel()
    @State private var bypassAsk = false

    func body(content: Content) -> some View {
        content
            .overlayPreferenceValue(QuickPillAnchor.self) { anchor in
                GeometryReader { geo in
                    ZStack(alignment: .topLeading) {
                        if isPresented {
                            // The page steps back, as in the design: darker on dark.
                            Color.black.opacity(scheme == .dark ? 0.6 : 0.3)
                                .ignoresSafeArea()
                                .contentShape(Rectangle())
                                .onTapGesture {
                                    feedback.play(.expand(false))
                                    close()
                                }
                                .transition(.opacity)
                                .accessibilityAddTraits(.isButton)
                                .accessibilityLabel("Close")
                                .accessibilityIdentifier("quickScrim")
                        }
                        if isPresented, let settings = model.autopilot {
                            let pill = anchor.map { geo[$0] }
                            let width = min(300, geo.size.width - 24)
                            let x = pill.map { min(max($0.maxX - width, 12), geo.size.width - width - 12) } ?? geo.size.width - width - 14
                            let y = pill.map { $0.maxY + 8 } ?? 62
                            QuickMenuCard(
                                settings: settings,
                                modelReady: ap.modelReady,
                                onMode: pick,
                                onBypass: {
                                    close()
                                    bypassAsk = true
                                },
                                onStopBypass: {
                                    close()
                                    Task { await ap.stopBypass() }
                                },
                                onLockdown: {
                                    close()
                                    Task { await ap.setMode(.lockdown) }
                                },
                                onEndLockdown: {
                                    close()
                                    Task { await ap.endLockdown() }
                                },
                                onMore: {
                                    close()
                                    model.show(.autopilot)
                                }
                            )
                            .frame(width: width)
                            .offset(x: x, y: y)
                            .transition(.scale(scale: 0.92, anchor: .topTrailing).combined(with: .opacity))
                            .accessibilityAction(.escape) { close() }
                        }
                    }
                    .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
                }
                // Closed, the page under it takes every touch.
                .allowsHitTesting(isPresented)
            }
            .onAppear { ap.bind(model) }
            .sheet(isPresented: $bypassAsk) {
                BypassSheet(who: nil) { length in
                    feedback.quietClose()
                    Task { await ap.startBypass(minutes: length.minutes) }
                }
                .environment(\.feedback, feedback)
            }
            .presentationFeedback(bypassAsk)
    }

    private func pick(_ mode: AutopilotMode) {
        close()
        if mode != model.autopilot?.mode { Task { await ap.setMode(mode) } }
    }

    private func close() {
        withAnimation(.spring(duration: 0.25, bounce: 0.1)) { isPresented = false }
    }
}

/// The quick menu, dropped from the header's mode pill (the Android app's `MenuCard`): Manual and Auto side by side
/// (the one in force outlined in its colour), Bypass and Lock down under them, then the Autopilot page. A running
/// bypass is the Bypass button itself ("Bypass on · 8:12", tap to stop), a lockdown the Lock down button: there is no
/// other Stop.
private struct QuickMenuCard: View {
    var settings: AutopilotSettings
    var modelReady: Bool
    var onMode: (AutopilotMode) -> Void
    var onBypass: () -> Void
    var onStopBypass: () -> Void
    var onLockdown: () -> Void
    var onEndLockdown: () -> Void
    var onMore: () -> Void

    static let fill = Palette.dynamic(0xFFFFFF, 0x18181B)
    static let edge = Palette.dynamic(0xE2E2E6, 0x2A2A30)
    static let divider = Palette.dynamic(0xE2E2E6, 0x26262B)

    var body: some View {
        let bypass = settings.mode == .bypass
        let lockdown = settings.mode == .lockdown
        let shape = RoundedRectangle(cornerRadius: 22, style: .continuous)
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                ForEach([AutopilotMode.manual, .auto], id: \.self) { mode in
                    let needs = !modelReady && AutopilotText.needsModel(mode)
                    QuickModeCard(
                        mode: mode,
                        selected: settings.mode == mode,
                        quiet: bypass || lockdown,
                        line: needs ? "Needs the model" : AutopilotText.line(mode)
                    ) {
                        onMode(mode)
                    }
                }
            }
            .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 8) {
                if bypass {
                    QuickStateButton(
                        title: "Bypass on",
                        until: settings.bypassUntil.flatMap { AutopilotText.isUntilOff($0) ? nil : $0 },
                        hint: "Tap to stop",
                        fill: Palette.danger,
                        text: Palette.dynamic(0xFFFFFF, 0x2A0606),
                        action: onStopBypass
                    )
                    .accessibilityIdentifier("quick:BYPASS")
                } else {
                    QuickMenuButton(title: "Bypass", symbol: "bolt.fill", fill: Palette.danger.opacity(0.12), text: Palette.danger, action: onBypass)
                        .accessibilityIdentifier("quick:BYPASS")
                }
                if lockdown {
                    QuickStateButton(
                        title: "Locked down",
                        until: nil,
                        hint: "Tap to end",
                        fill: Palette.warning,
                        text: Palette.dynamic(0xFFFFFF, 0x231A00),
                        action: onEndLockdown
                    )
                    .accessibilityIdentifier("quick:LOCKDOWN")
                } else {
                    QuickMenuButton(title: "Lock down", symbol: "lock.fill", fill: Palette.dual(UIColor(white: 0, alpha: 0.05), UIColor(white: 1, alpha: 0.06)), text: Palette.text, action: onLockdown)
                        .accessibilityIdentifier("quick:LOCKDOWN")
                }
            }
            .padding(.top, 8)
            Rectangle().fill(Self.divider).frame(height: 1).padding(.top, 10)
            Button(action: onMore) {
                HStack(spacing: 6) {
                    Text("Autopilot settings").font(RFont.sans(13.5, .medium))
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.right").font(.system(size: 12, weight: .semibold))
                }
                .foregroundStyle(Palette.accent)
                .padding(.horizontal, 4)
                .padding(.top, 10)
                .padding(.bottom, 4)
                .contentShape(Rectangle())
            }
            .buttonStyle(PressDim())
            .accessibilityIdentifier("quickMore")
        }
        .padding(12)
        .background(Self.fill, in: shape)
        .overlay(shape.strokeBorder(Self.edge, lineWidth: 1))
        .shadow(color: .black.opacity(0.5), radius: 24, y: 14)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("autopilotQuick")
    }
}

/// Manual or Auto in the menu: its tile, name and what it does; the one in force outlined and washed in its colour.
/// While a bypass or a lockdown is in force neither is: both stay tappable, quieter.
private struct QuickModeCard: View {
    var mode: AutopilotMode
    var selected: Bool
    var quiet: Bool
    var line: String
    var action: () -> Void

    var body: some View {
        let tint = AutopilotStyle.tint(mode)
        let shape = RoundedRectangle(cornerRadius: 16, style: .continuous)
        let on = selected && !quiet
        Button(action: action) {
            VStack(alignment: .leading, spacing: 0) {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(tint.opacity(0.17))
                    .frame(width: 32, height: 32)
                    .overlay {
                        Image(systemName: AutopilotText.symbol(mode))
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(tint)
                    }
                Text(AutopilotText.name(mode))
                    .font(RFont.sans(15, .semibold))
                    .foregroundStyle(Palette.text)
                    .lineLimit(1)
                    .padding(.top, 8)
                Text(line)
                    .font(RFont.sans(11.5))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.85)
                    .padding(.top, 2)
                Spacer(minLength: 0)
            }
            .padding(11)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(on ? tint.opacity(0.10) : Palette.dynamic(0xF3F3F5, 0x0D0D0F), in: shape)
            .overlay(shape.strokeBorder(on ? tint : Palette.dynamic(0xE2E2E6, 0x26262B), lineWidth: on ? 1.5 : 1))
            .opacity(quiet ? 0.45 : 1)
            .contentShape(shape)
        }
        .buttonStyle(ModeCardPress())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("quick:\(AutopilotText.key(mode).uppercased())")
    }
}

/// A button of the menu: its icon and words on its own fill, 44 pt tall.
private struct QuickMenuButton: View {
    var title: String
    var symbol: String
    var fill: Color
    var text: Color
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol)
                .labelStyle(.titleAndIcon)
                .font(RFont.sans(14, .semibold))
                .foregroundStyle(text)
                .lineLimit(1)
                .minimumScaleFactor(0.85)
                .frame(maxWidth: .infinity, minHeight: 44)
                .background(fill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(PressDim())
    }
}

/// A button of the menu that is also a state, solid in its colour: "Bypass on · 8:12" (or "Bypass on" without an end)
/// over "Tap to stop"; "Locked down" over "Tap to end". Words only.
private struct QuickStateButton: View {
    var title: String
    /// A bypass's end, counted down beside the title.
    var until: Int64?
    var hint: String
    var fill: Color
    var text: Color
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 1) {
                Group {
                    if let until {
                        LiveClock(interval: 1) { now in
                            Text("\(title) · \(AutopilotText.clock(until: until, now: Int64(now)))").monospacedDigit()
                        }
                    } else {
                        Text(title)
                    }
                }
                .font(RFont.sans(14, .semibold))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                Text(hint)
                    .font(RFont.sans(11, .medium))
                    .opacity(0.75)
                    .lineLimit(1)
            }
            .foregroundStyle(text)
            .frame(maxWidth: .infinity, minHeight: 44)
            .background(fill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(PressDim())
        .accessibilityElement(children: .combine)
    }
}

/// Lockdown to Bypass on one track. The thumb's colour and glow follow the risk as it moves; each detent ticks,
/// climbing the scale. Letting go sets the mode, except Bypass, which slides back and opens the bypass sheet.
struct RiskSliderCard: View {
    var settings: AutopilotSettings
    var modelReady: Bool
    var onSet: (AutopilotMode) -> Void
    var onBypass: () -> Void
    var onStop: () -> Void
    var onMore: () -> Void

    @Environment(\.feedback) private var feedback
    @Environment(\.dismiss) private var dismiss
    @State private var pos: CGFloat
    @State private var detent: Int

    private let modes = AutopilotText.byRisk
    private let thumb: CGFloat = 30

    init(
        settings: AutopilotSettings, modelReady: Bool, onSet: @escaping (AutopilotMode) -> Void,
        onBypass: @escaping () -> Void, onStop: @escaping () -> Void, onMore: @escaping () -> Void
    ) {
        self.settings = settings
        self.modelReady = modelReady
        self.onSet = onSet
        self.onBypass = onBypass
        self.onStop = onStop
        self.onMore = onMore
        let at = AutopilotText.byRisk.firstIndex(of: settings.mode) ?? 1
        _pos = State(initialValue: CGFloat(at))
        _detent = State(initialValue: at)
    }

    private var current: Int { modes.firstIndex(of: settings.mode) ?? 1 }

    var body: some View {
        let last = CGFloat(modes.count - 1)
        let at = min(max(pos, 0), last)
        let lo = min(Int(at), modes.count - 2)
        let tint = ModeLook.tint(modes[lo]).mix(with: ModeLook.tint(modes[lo + 1]), by: Double(at - CGFloat(lo)))
        let risk = at / last
        let shown = modes[detent]
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 12) {
                Image(systemName: AutopilotText.symbol(shown))
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 40, height: 40)
                    .background(tint, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                VStack(alignment: .leading, spacing: 2) {
                    Text(AutopilotText.name(shown)).font(RFont.sans(18, .semibold)).foregroundStyle(Palette.text)
                    Text(!modelReady && AutopilotText.needsModel(shown) ? "Needs the model" : AutopilotText.line(shown))
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.secondary)
                }
                Spacer(minLength: 0)
                if settings.mode == .bypass, let until = settings.bypassUntil {
                    LiveClock(interval: 1) { now in
                        Text(AutopilotText.clock(until: until, now: Int64(now))).font(RFont.mono(14, .semibold)).monospacedDigit()
                            .foregroundStyle(Palette.danger)
                    }
                }
            }
            .accessibilityElement(children: .combine)

            track(at: at, tint: tint, risk: risk)

            HStack(spacing: 0) {
                ForEach(Array(modes.enumerated()), id: \.offset) { i, mode in
                    Button {
                        move(to: i)
                        land(i)
                    } label: {
                        Image(systemName: AutopilotText.symbol(mode))
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(i == detent ? ModeLook.tint(mode) : Palette.tertiary)
                            .frame(width: thumb, height: 30)
                            .frame(maxWidth: .infinity, alignment: i == 0 ? .leading : i == modes.count - 1 ? .trailing : .center)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(AutopilotText.name(mode))
                    .accessibilityAddTraits(i == detent ? .isSelected : [])
                }
            }

            if settings.mode == .bypass {
                Button {
                    onStop()
                    dismiss()
                } label: {
                    // Words only, as everywhere a bypass is stopped.
                    Text("Stop bypass")
                        .font(RFont.sans(14, .semibold))
                        .foregroundStyle(Palette.danger)
                        .frame(maxWidth: .infinity, minHeight: 36)
                        .background(Palette.danger.opacity(0.12), in: Capsule())
                }
                .buttonStyle(.plain)
            }

            HStack {
                Spacer()
                Button("More", action: onMore)
                    .font(RFont.sans(14, .medium))
                    .foregroundStyle(Palette.secondary)
                    .buttonStyle(.plain)
            }
        }
        .padding(18)
        .frame(width: 320)
        .background(LinearGradient(colors: [tint.opacity(0.16 + 0.10 * risk), .clear], startPoint: .top, endPoint: .bottom))
        .onChange(of: settings.mode) {
            detent = current
            withAnimation(.spring(duration: 0.3, bounce: 0.25)) { pos = CGFloat(current) }
        }
    }

    /// The coloured track with a dot per detent, the glow and the thumb; drag anywhere on it, or tap a spot.
    private func track(at: CGFloat, tint: Color, risk: CGFloat) -> some View {
        GeometryReader { geo in
            let step = (geo.size.width - thumb) / CGFloat(modes.count - 1)
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(LinearGradient(colors: modes.map { ModeLook.tint($0).opacity(0.5) }, startPoint: .leading, endPoint: .trailing))
                    .frame(height: 8)
                    .padding(.horizontal, thumb / 2)
                ForEach(0..<modes.count, id: \.self) { i in
                    Circle().fill(Color.white.opacity(0.6)).frame(width: 4, height: 4)
                        .offset(x: step * CGFloat(i) + thumb / 2 - 2)
                }
                Circle()
                    .fill(tint.opacity(0.18 + 0.32 * risk))
                    .frame(width: thumb * 2.6, height: thumb * 2.6)
                    .blur(radius: 10)
                    .offset(x: step * at + thumb / 2 - thumb * 1.3)
                Circle()
                    .fill(tint)
                    .overlay(Circle().strokeBorder(Color.white.opacity(0.9), lineWidth: 2))
                    .frame(width: thumb, height: thumb)
                    .shadow(color: tint.opacity(0.6), radius: 6 + 10 * risk)
                    .offset(x: step * at)
            }
            .frame(width: geo.size.width, height: 44)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        let p = min(max((value.location.x - thumb / 2) / step, 0), CGFloat(modes.count - 1))
                        pos = p
                        move(to: Int(p.rounded()))
                    }
                    .onEnded { _ in land(detent) }
            )
        }
        .frame(height: 44)
        .accessibilityElement()
        .accessibilityLabel("Autopilot")
        .accessibilityValue(AutopilotText.name(modes[detent]))
        .accessibilityAdjustableAction { direction in
            let i = direction == .increment ? min(detent + 1, modes.count - 1) : max(detent - 1, 0)
            move(to: i)
            land(i)
        }
        .accessibilityIdentifier("riskSlider")
    }

    /// The thumb reached detent `i`: a tick, one note higher per step.
    private func move(to i: Int) {
        guard i != detent else { return }
        detent = i
        feedback.play(.detent, step: i)
    }

    private func land(_ i: Int) {
        let mode = modes[i]
        if mode == .bypass && settings.mode != .bypass {
            // No bypass from a slide: back to where it was, and the sheet asks for a length and a hold.
            move(to: current)
            withAnimation(.spring(duration: 0.3, bounce: 0.25)) { pos = CGFloat(current) }
            onBypass()
        } else {
            withAnimation(.spring(duration: 0.3, bounce: 0.3)) { pos = CGFloat(i) }
            if mode != settings.mode {
                onSet(mode)
                closeSoon()
            }
        }
    }

    /// Long enough to see the new mode land.
    private func closeSoon() {
        Task {
            try? await Task.sleep(for: .milliseconds(320))
            dismiss()
        }
    }
}
