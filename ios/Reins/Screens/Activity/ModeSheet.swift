import SwiftUI

// The one Mode sheet (Next-Activity, 03; Next-Flows): the header's mode chip opens it, and Trust may too. Titled
// "Mode" with a close button, the five modes as `ModeSheetRow`s in one group (a check on the mode in force, the length
// a timed one starts with), and Autopilot's settings in a group of its own under them. Manual, Auto and Lockdown take
// effect on a tap; Overnight and Bypass open their own held sheets (the same as Trust's); a running Overnight, Bypass
// or Lockdown is stopped by a tap on its own row. Every row keeps the identifiers the old chip menu had.

/// The global mode as the header's chip shows it: a bypass or an Overnight running, else the mode.
extension ModeChip.Mode {
    init(_ settings: AutopilotSettings, now: Int64) {
        switch settings.mode {
        case .manual: self = .manual
        case .auto: self = .auto
        case .overnight: self = .overnight(left: Self.left(settings.overnightUntil, now: now))
        case .lockdown: self = .lockdown
        case .bypass: self = .bypass(left: Self.left(settings.bypassUntil, now: now))
        }
    }

    /// "47:12" left, "23:39" from an hour up (no seconds), "∞" for one without an end, nothing when the end is unknown.
    static func left(_ until: Int64?, now: Int64) -> String {
        guard let until else { return "" }
        return AutopilotText.chipClock(until: until, now: now)
    }
}

extension ModeKind {
    init(_ mode: AutopilotMode) {
        switch mode {
        case .manual: self = .manual
        case .auto: self = .auto
        case .overnight: self = .overnight
        case .lockdown: self = .lockdown
        case .bypass: self = .bypass
        }
    }

    var autopilotMode: AutopilotMode {
        switch self {
        case .manual: .manual
        case .auto: .auto
        case .overnight: .overnight
        case .lockdown: .lockdown
        case .bypass: .bypass
        }
    }
}

/// What a tap on a mode's row does (pure, unit-tested): the same as the chip's menu did.
enum ModeSheetAction: Equatable {
    /// Manual, Auto or Lockdown, at once.
    case set(AutopilotMode)
    /// Nothing: the mode is already in force.
    case none
    case askOvernight, stopOvernight
    case askBypass, stopBypass
    case endLockdown

    static func of(_ mode: ModeKind, _ s: AutopilotSettings) -> ModeSheetAction {
        switch mode {
        case .manual, .auto:
            return s.mode == mode.autopilotMode ? .none : .set(mode.autopilotMode)
        case .overnight:
            return s.mode == .overnight && s.overnightUntil != nil ? .stopOvernight : .askOvernight
        case .lockdown:
            return s.mode == .lockdown ? .endLockdown : .set(.lockdown)
        case .bypass:
            return s.mode == .bypass ? .stopBypass : .askBypass
        }
    }

    /// The row's identifier, as the chip's menu had it.
    static func identifier(_ mode: ModeKind) -> String { "quick:\(mode.autopilotMode.raw)" }

    /// The length a timed mode starts with, shown at the right of its row; nil while it runs (the check shows).
    static func length(_ mode: ModeKind, _ s: AutopilotSettings) -> String? {
        guard s.mode != mode.autopilotMode else { return nil }
        switch mode {
        case .overnight: return short(TimedMode.overnight.initial)
        case .bypass: return short(TimedMode.bypass.initial)
        default: return nil
        }
    }

    /// "1h", "8h", "15m".
    static func short(_ minutes: UInt32) -> String { minutes >= 60 && minutes % 60 == 0 ? "\(minutes / 60)h" : "\(minutes)m" }
}

private extension AutopilotMode {
    var raw: String {
        switch self {
        case .manual: "MANUAL"
        case .auto: "AUTO"
        case .overnight: "OVERNIGHT"
        case .lockdown: "LOCKDOWN"
        case .bypass: "BYPASS"
        }
    }
}

/// The header's mode chip: a tap opens the Mode sheet.
struct ModeChipButton: View {
    var settings: AutopilotSettings
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            open = true
            feedback.play(.tap)
        } label: {
            LiveClock(interval: 1) { now in
                ModeChip(mode: ModeChip.Mode(settings, now: Int64(now)), chevron: true)
                    .overlay {
                        // Open: a soft violet ring, as the board draws the chip under its sheet.
                        if open { Capsule().strokeBorder(Palette.violetFill.opacity(0.25), lineWidth: 4).padding(-4) }
                    }
            }
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel("Mode: \(ModeLook.name(settings.mode))")
        .accessibilityHint("Opens the modes")
        .accessibilityIdentifier("modePill")
        #if DEBUG
        // `-openModeSheet` (screenshots): the sheet opens with the page.
        .onAppear { if ProcessInfo.processInfo.arguments.contains("-openModeSheet") { open = true } }
        #endif
        .modeSheet(isPresented: $open)
    }
}

extension View {
    /// Presents the Mode sheet, as tall as its rows.
    func modeSheet(isPresented: Binding<Bool>) -> some View {
        modifier(ModeSheetPresenter(isPresented: isPresented))
    }
}

private struct ModeSheetPresenter: ViewModifier {
    @Binding var isPresented: Bool
    @Environment(\.feedback) private var feedback
    @State private var height: CGFloat = 460

    func body(content: Content) -> some View {
        content
            .sheet(isPresented: $isPresented) {
                ModeSheet { isPresented = false }
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
                    .presentationDetents([.height(height)])
                    .presentationDragIndicator(.visible)
                    .presentationCornerRadius(32)
                    .presentationBackground(Palette.paperRaised)
                    .environment(\.feedback, feedback)
            }
            .presentationFeedback(isPresented)
    }
}

/// The sheet itself: title and close, the five modes, Autopilot.
struct ModeSheet: View {
    var close: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var ap = AutopilotModel()
    @State private var bypassAsk = false
    @State private var overnightAsk = false

    var body: some View {
        let settings = model.autopilot
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    Text("Mode")
                        .textRole(.cardAsk)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    RoundHeaderButton(glyph: .close, label: "Close") { close() }
                        .accessibilityIdentifier("modeSheetClose")
                }
                .padding(.horizontal, 20)
                .padding(.top, 20)
                .padding(.bottom, 4)
                if let settings {
                    SheetGroup {
                        ForEach(ModeKind.allCases, id: \.self) { mode in row(mode, settings) }
                    }
                    .padding(.top, 8)
                }
                SheetGroup {
                    Button {
                        close()
                        model.open(.trust)
                        feedback.defaultTap()
                    } label: {
                        HStack(spacing: 12) {
                            RoundedRectangle(cornerRadius: 9, style: .continuous)
                                .fill(Palette.violet.opacity(0.14))
                                .frame(width: 32, height: 32)
                                .overlay { ModeGlyph(.auto, size: 17).foregroundStyle(Palette.violet) }
                            Text("Autopilot").textRole(.row).foregroundStyle(Palette.ink).frame(maxWidth: .infinity, alignment: .leading)
                            RowChevron()
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 56)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(RowButtonStyle())
                    .accessibilityIdentifier("quickMore")
                }
                .padding(.top, 12)
            }
            .padding(.bottom, 24)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("autopilotQuick")
        }
        .scrollBounceBehavior(.basedOnSize)
        .onAppear { ap.bind(model) }
        .sheet(isPresented: $bypassAsk) {
            BypassSheet(who: nil) { length in
                feedback.quietClose()
                close()
                Task { await ap.startBypass(minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(bypassAsk)
        .sheet(isPresented: $overnightAsk) {
            OvernightSheet(who: nil, ap: ap) { length in
                feedback.quietClose()
                close()
                Task { await ap.startOvernight(minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(overnightAsk)
    }

    private func row(_ mode: ModeKind, _ s: AutopilotSettings) -> some View {
        let current = s.mode == mode.autopilotMode
        return Button {
            pick(mode, s)
        } label: {
            ModeSheetRow(mode: mode, current: current, length: ModeSheetAction.length(mode, s))
        }
        .buttonStyle(RowButtonStyle())
        .accessibilityAddTraits(current ? .isSelected : [])
        .accessibilityHint(current && !mode.calm ? "Stops it" : "")
        .accessibilityIdentifier(ModeSheetAction.identifier(mode))
    }

    private func pick(_ mode: ModeKind, _ s: AutopilotSettings) {
        switch ModeSheetAction.of(mode, s) {
        case .none: close()
        case let .set(m):
            close()
            Task { await ap.setMode(m) }
        case .askOvernight: overnightAsk = true
        case .stopOvernight:
            close()
            Task { await ap.stopOvernight() }
        case .askBypass: bypassAsk = true
        case .stopBypass:
            close()
            Task { await ap.stopBypass() }
        case .endLockdown:
            close()
            Task { await ap.endLockdown() }
        }
    }
}

/// A group on a sheet: raised with a hairline ring in light, sunken in dark (the board's `.sheet .grp`).
struct SheetGroup<Content: View>: View {
    @ViewBuilder var content: Content
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        VStack(spacing: 0) {
            Separated(inset: 62) { content }
        }
        .frame(maxWidth: .infinity)
        .background {
            shape.fill(scheme == .dark ? Palette.paperSunken : Palette.paperRaised)
            if scheme == .light { shape.strokeBorder(Palette.rule, lineWidth: 1) }
        }
        .clipShape(shape)
        .padding(.horizontal, 16)
    }
}
