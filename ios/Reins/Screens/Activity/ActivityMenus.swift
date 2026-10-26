import SwiftUI

// Activity's header menu (iOS-Activity.png, 03), a Signal menu growing from the mode chip: Manual and Auto switch at
// once; Overnight, Lockdown and Bypass… as on Trust; Autopilot settings opens Trust. It replaces the quick menu the mode
// pill used to drop; its rows keep their identifiers. The filter button and its chips are in ActivityFilterBar.swift.

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

    /// "47:12" left, "∞" for one without an end, nothing when the end is unknown.
    static func left(_ until: Int64?, now: Int64) -> String {
        guard let until else { return "" }
        return AutopilotText.isUntilOff(until) ? "∞" : ModeLook.clock(until: until, now: now)
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
}

/// The mode chip in Activity's header with its menu, and the sheets its Bypass and Overnight open (the same sheets as
/// Trust's: a length, held to confirm).
struct ActivityModeMenu: View {
    var settings: AutopilotSettings
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var ap = AutopilotModel()
    @State private var bypassAsk = false
    @State private var overnightAsk = false

    var body: some View {
        LiveClock(interval: 1) { now in
            ModeChip(mode: ModeChip.Mode(settings, now: Int64(now)), chevron: true)
        }
        .signalMenu(
            id: "modeMenu", items: Self.items(settings, modelReady: ap.modelReady), selected: Self.selected(settings),
            identifier: "autopilotQuick", scrimIdentifier: "quickScrim", onPick: pick
        )
        .accessibilityLabel("Autopilot: \(ModeLook.name(settings.mode))")
        .accessibilityIdentifier("modePill")
        .onAppear { ap.bind(model) }
        .sheet(isPresented: $bypassAsk) {
            BypassSheet(who: nil) { length in
                feedback.quietClose()
                Task { await ap.startBypass(minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(bypassAsk)
        .sheet(isPresented: $overnightAsk) {
            OvernightSheet(who: nil, ap: ap) { length in
                feedback.quietClose()
                Task { await ap.startOvernight(minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(overnightAsk)
    }

    /// The mode in force, among Manual and Auto (nothing is chosen while a bypass, an Overnight or Lockdown runs).
    static func selected(_ s: AutopilotSettings) -> String? {
        switch s.mode {
        case .manual: "quick:MANUAL"
        case .auto: "quick:AUTO"
        default: nil
        }
    }

    /// Safest first: Manual, Auto, Overnight, Lockdown; Bypass… apart; Autopilot settings last. A running bypass,
    /// Overnight or Lockdown is its own row, which stops it.
    static func items(_ s: AutopilotSettings, modelReady: Bool) -> [SignalMenuItem] {
        let now = Timers.nowSeconds()
        let overnight = s.mode == .overnight && s.overnightUntil != nil
        let bypass = s.mode == .bypass
        let lockdown = s.mode == .lockdown
        func left(_ until: Int64?) -> String {
            guard let until, !AutopilotText.isUntilOff(until) else { return "" }
            return " · " + ModeLook.clock(until: until, now: now)
        }
        // What loosens control, or stops what runs, needs a tap on its row: a slide from the chip never picks it.
        return [
            SignalMenuItem(
                id: "quick:MANUAL", title: "Manual", subtitle: AutopilotText.line(.manual), glyph: .person, tapOnly: lockdown
            ),
            SignalMenuItem(
                id: "quick:AUTO", title: "Auto", subtitle: !modelReady ? "Needs the model" : AutopilotText.line(.auto),
                glyph: .autopilot, tone: .solid(Palette.violetFill), tapOnly: s.mode == .manual || lockdown
            ),
            overnight
                ? SignalMenuItem(
                    id: "quick:OVERNIGHT", title: "\(AutopilotText.overnightName) on\(left(s.overnightUntil))", subtitle: "Tap to stop",
                    glyph: .moon, tone: .solid(Palette.violetFill), tapOnly: true
                )
                : SignalMenuItem(
                    id: "quick:OVERNIGHT", title: AutopilotText.overnightName, subtitle: AutopilotText.line(.overnight),
                    glyph: .moon, tone: .kind(.grant), more: true
                ),
            lockdown
                ? SignalMenuItem(id: "quick:LOCKDOWN", title: "Locked down", subtitle: "Tap to end", glyph: .lock, tone: .solid(Palette.fieldWarn), tapOnly: true)
                : SignalMenuItem(id: "quick:LOCKDOWN", title: "Lockdown", subtitle: "Deny everything now", glyph: .lock, tone: .kind(.warn)),
            bypass
                ? SignalMenuItem(
                    id: "quick:BYPASS", title: "Bypass on\(left(s.bypassUntil))", subtitle: "Tap to stop", glyph: .autopilot,
                    tone: .solid(Palette.fieldDelete), separated: true, tapOnly: true
                )
                : SignalMenuItem(
                    id: "quick:BYPASS", title: "Bypass…", subtitle: "Approve everything for a while", glyph: .autopilot,
                    tone: .danger, separated: true, more: true
                ),
            SignalMenuItem(id: "quickMore", title: "Autopilot settings", separated: true, more: true),
        ]
    }

    private func pick(_ id: String) {
        let s = settings
        switch id {
        case "quick:MANUAL": setMode(.manual)
        case "quick:AUTO": setMode(.auto)
        case "quick:OVERNIGHT":
            if s.mode == .overnight && s.overnightUntil != nil { Task { await ap.stopOvernight() } } else { overnightAsk = true }
        case "quick:LOCKDOWN":
            if s.mode == .lockdown { Task { await ap.endLockdown() } } else { Task { await ap.setMode(.lockdown) } }
        case "quick:BYPASS":
            if s.mode == .bypass { Task { await ap.stopBypass() } } else { bypassAsk = true }
        case "quickMore": model.open(.trust)
        default: break
        }
    }

    private func setMode(_ mode: AutopilotMode) {
        if mode != settings.mode { Task { await ap.setMode(mode) } }
    }
}

