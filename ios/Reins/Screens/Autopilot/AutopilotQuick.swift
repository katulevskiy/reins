import SwiftUI

// The header's Autopilot pill opens a compact switcher rather than the whole Autopilot page (the Android app's
// AutopilotQuick.kt). Release builds use the menu; debug builds can compare it with a risk slider and with the page
// (Settings, Developer).

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

/// The mode pill with its switcher. The menu lists the modes riskiest first, each with its symbol and a few words; a
/// tap switches (Bypass and Lockdown ask for Face ID first, as everywhere). Bypass opens a submenu of lengths;
/// "More" opens the Autopilot page.
struct AutopilotQuickPill: View {
    var settings: AutopilotSettings

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @AppStorage(QuickStyle.key) private var styleRaw = QuickStyle.menu.rawValue
    @State private var ap = AutopilotModel()
    @State private var sliderOpen = false

    var body: some View {
        Group {
            switch QuickStyle.resolve(styleRaw) {
            case .menu: menu
            case .slider:
                ActivityModePill(settings: settings) {
                    sliderOpen = true
                    feedback.play(.open)
                }
                .popover(isPresented: $sliderOpen, arrowEdge: .top) {
                    RiskSliderCard(
                        settings: settings,
                        modelReady: ap.modelReady,
                        onSet: { mode, minutes in Task { await ap.setMode(mode, minutes: minutes) } },
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
    }

    private var menu: some View {
        Menu {
            if settings.mode == .bypass {
                Button(role: .destructive) {
                    Task { await ap.stopBypass() }
                } label: {
                    Label("Stop bypass", systemImage: "stop.fill")
                }
            }
            ForEach(AutopilotText.byRisk.reversed(), id: \.self) { mode in
                if mode == .bypass && settings.mode != .bypass {
                    Menu {
                        ForEach(AutopilotText.bypassMinutes, id: \.self) { minutes in
                            Button(AutopilotText.bypassChoice(minutes)) {
                                Task { await ap.setMode(.bypass, minutes: minutes) }
                            }
                        }
                    } label: {
                        modeLabel(mode)
                    }
                } else {
                    Button {
                        if mode != settings.mode { Task { await ap.setMode(mode) } }
                    } label: {
                        modeLabel(mode)
                    }
                }
            }
            Divider()
            Button {
                model.show(.autopilot)
            } label: {
                Label("More", systemImage: "ellipsis.circle")
            }
        } label: {
            ModePillLabel(settings: settings)
        }
        .menuOrder(.fixed)
        .buttonStyle(.plain)
        .modePillGlass(settings)
    }

    /// A mode's row: its name and a few words (or that it needs the model), its symbol, a tick on the one in force.
    private func modeLabel(_ mode: AutopilotMode) -> some View {
        let needs = !ap.modelReady && AutopilotText.needsModel(mode)
        return Label {
            Text(AutopilotText.name(mode))
            Text(needs ? "Needs the model" : AutopilotText.line(mode))
        } icon: {
            Image(systemName: mode == settings.mode ? "checkmark" : AutopilotText.symbol(mode))
        }
    }
}

/// Lockdown to Bypass on one track. The thumb's colour and glow follow the risk as it moves; each detent ticks,
/// climbing the scale. Letting go sets the mode, except Bypass, which waits for a length (or ✕, which slides back).
struct RiskSliderCard: View {
    var settings: AutopilotSettings
    var modelReady: Bool
    var onSet: (AutopilotMode, UInt32?) -> Void
    var onStop: () -> Void
    var onMore: () -> Void

    @Environment(\.feedback) private var feedback
    @Environment(\.dismiss) private var dismiss
    @State private var pos: CGFloat
    @State private var detent: Int
    @State private var askBypass = false

    private let modes = AutopilotText.byRisk
    private let thumb: CGFloat = 30

    init(
        settings: AutopilotSettings, modelReady: Bool, onSet: @escaping (AutopilotMode, UInt32?) -> Void,
        onStop: @escaping () -> Void, onMore: @escaping () -> Void
    ) {
        self.settings = settings
        self.modelReady = modelReady
        self.onSet = onSet
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
                        Text(ModeLook.clock(until: until, now: Int64(now))).font(RFont.mono(14, .semibold)).monospacedDigit()
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

            if askBypass {
                HStack(spacing: 6) {
                    ForEach(AutopilotText.bypassMinutes, id: \.self) { minutes in
                        Button(minutes == 60 ? "1 h" : "\(minutes) min") {
                            askBypass = false
                            onSet(.bypass, minutes)
                            closeSoon()
                        }
                        .font(RFont.sans(14, .semibold))
                        .foregroundStyle(Palette.danger)
                        .frame(maxWidth: .infinity, minHeight: 36)
                        .background(Palette.danger.opacity(0.14), in: Capsule())
                        .buttonStyle(.plain)
                    }
                    Button {
                        cancelBypass()
                    } label: {
                        Image(systemName: "xmark").font(.system(size: 13, weight: .bold)).foregroundStyle(Palette.secondary)
                            .frame(width: 36, height: 36)
                            .background(Palette.controlFill, in: Circle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Cancel")
                }
                .accessibilityIdentifier("quickBypassTimes")
            } else if settings.mode == .bypass {
                Button {
                    onStop()
                    dismiss()
                } label: {
                    Label("Stop bypass", systemImage: "stop.fill")
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
        .animation(.smooth(duration: 0.2), value: askBypass)
        .onChange(of: settings.mode) {
            guard !askBypass else { return }
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
        withAnimation(.spring(duration: 0.3, bounce: 0.3)) { pos = CGFloat(i) }
        let mode = modes[i]
        if mode == .bypass && settings.mode != .bypass {
            askBypass = true
        } else if mode != settings.mode {
            askBypass = false
            onSet(mode, nil)
            closeSoon()
        } else {
            askBypass = false
        }
    }

    private func cancelBypass() {
        askBypass = false
        move(to: current)
        withAnimation(.spring(duration: 0.3, bounce: 0.25)) { pos = CGFloat(current) }
    }

    /// Long enough to see the new mode land.
    private func closeSoon() {
        Task {
            try? await Task.sleep(for: .milliseconds(320))
            dismiss()
        }
    }
}
