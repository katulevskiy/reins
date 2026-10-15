import SwiftUI

// Activity's header menus (iOS-Activity.png, 03 and 04), both Signal menus growing from their trigger: the mode chip's
// (Manual and Auto switch at once; Overnight, Lockdown and Bypass… as on Trust; Autopilot settings opens Trust) and the
// filter button's. They replace the quick menu the mode pill used to drop; its rows keep their identifiers.

/// The global mode as the header's chip shows it: a bypass or an Overnight running, else the mode.
extension ModeChip.Mode {
    init(_ settings: AutopilotSettings, now: Int64) {
        switch settings.mode {
        case .manual: self = .manual
        case .auto: self = .auto
        case .overnight: self = .overnight
        case .lockdown: self = .lockdown
        case .bypass:
            let until = settings.bypassUntil
            self = .bypass(left: until.map { AutopilotText.isUntilOff($0) ? "∞" : ModeLook.clock(until: $0, now: now) } ?? "")
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
        return [
            SignalMenuItem(
                id: "quick:MANUAL", title: "Manual", subtitle: AutopilotText.line(.manual), glyph: .person
            ),
            SignalMenuItem(
                id: "quick:AUTO", title: "Auto", subtitle: !modelReady ? "Needs the model" : AutopilotText.line(.auto),
                glyph: .autopilot, tone: .solid(Palette.violetFill)
            ),
            overnight
                ? SignalMenuItem(
                    id: "quick:OVERNIGHT", title: "\(AutopilotText.overnightName) on\(left(s.overnightUntil))", subtitle: "Tap to stop",
                    glyph: .moon, tone: .solid(Palette.violetFill)
                )
                : SignalMenuItem(
                    id: "quick:OVERNIGHT", title: AutopilotText.overnightName, subtitle: AutopilotText.line(.overnight),
                    glyph: .moon, tone: .kind(.grant), more: true
                ),
            lockdown
                ? SignalMenuItem(id: "quick:LOCKDOWN", title: "Locked down", subtitle: "Tap to end", glyph: .lock, tone: .solid(Palette.fieldWarn))
                : SignalMenuItem(id: "quick:LOCKDOWN", title: "Lockdown", subtitle: "Deny everything now", glyph: .lock, tone: .kind(.warn)),
            bypass
                ? SignalMenuItem(
                    id: "quick:BYPASS", title: "Bypass on\(left(s.bypassUntil))", subtitle: "Tap to stop", glyph: .autopilot,
                    tone: .solid(Palette.fieldDelete), separated: true
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

/// The round filter button and its menu: everything, what Autopilot decided, the denials, what a grant covered.
struct ActivityFilterMenu: View {
    @Binding var filter: ActivityFilter
    /// One AI's entries only ("" for any), and the AIs to choose from.
    @Binding var ai: String
    var ais: [(id: String, name: String)]

    var body: some View {
        GlyphIcon(.filter, size: 20)
            .foregroundStyle(Palette.ink)
            .frame(width: 40, height: 40)
            .background(Palette.paperRaised, in: Circle())
            .overlay(Circle().strokeBorder(Palette.rule, lineWidth: 1))
            .overlay(alignment: .topTrailing) {
                if filter != .all || !ai.isEmpty {
                    Circle().fill(Palette.violetFill).frame(width: 9, height: 9).offset(x: -2, y: 2)
                }
            }
            .signalMenu(
                id: "filterMenu",
                items: ActivityFilter.allCases.map { f in
                    SignalMenuItem(id: "filter:\(f.rawValue)", title: f.title, glyph: Self.glyph(f))
                } + AnyAIChip.items(ais, separated: true),
                selected: ai.isEmpty ? "filter:\(filter.rawValue)" : "ai:\(ai)",
                identifier: "filterMenu"
            ) { id in
                if id.hasPrefix("ai:") {
                    ai = AnyAIChip.connection(id)
                } else if let f = ActivityFilter(rawValue: String(id.dropFirst("filter:".count))) {
                    filter = f
                }
            }
            .accessibilityLabel(filter == .all && ai.isEmpty ? "Filter" : "Filter: \(filter.title)")
            .accessibilityIdentifier("filterToggle")
    }

    static func glyph(_ f: ActivityFilter) -> Glyph {
        switch f {
        case .all: .activity
        case .autopilot: .autopilot
        case .denied: .block
        case .grants: .key
        }
    }
}

/// "Any AI ⌄": a chip whose menu narrows the river to one AI's entries, or back to any.
struct AnyAIChip: View {
    @Binding var ai: String
    var ais: [(id: String, name: String)]

    var body: some View {
        let name = ais.first { $0.id == ai }?.name
        HStack(spacing: 4) {
            Text(name ?? "Any AI").lineLimit(1)
            GlyphIcon(.chevronDown, size: 14)
        }
        .font(RFont.sans(14, .medium))
        .foregroundStyle(ai.isEmpty ? Palette.ink : Palette.onInverse)
        .padding(.horizontal, 13)
        .frame(height: 34)
        .background(ai.isEmpty ? Palette.paperRaised : Palette.inverse, in: Capsule())
        .overlay {
            if ai.isEmpty { Capsule().strokeBorder(Palette.rule, lineWidth: 1) }
        }
        .signalMenu(
            id: "aiMenu", items: [SignalMenuItem(id: "ai:", title: "Any AI", glyph: .activity)] + Self.items(ais, separated: true),
            selected: "ai:\(ai)", identifier: "aiMenu"
        ) { id in ai = Self.connection(id) }
        .accessibilityLabel(name.map { "AI: \($0)" } ?? "Any AI")
        .accessibilityIdentifier("filter:ai")
    }

    /// One row per AI, its name from the history (untrusted, one line).
    static func items(_ ais: [(id: String, name: String)], separated: Bool) -> [SignalMenuItem] {
        ais.enumerated().map { i, ai in
            SignalMenuItem(id: "ai:\(ai.id)", title: ai.name.isEmpty ? "Unnamed AI" : ai.name, glyph: .person, separated: separated && i == 0)
        }
    }

    static func connection(_ menuId: String) -> String { String(menuId.dropFirst("ai:".count)) }
}
