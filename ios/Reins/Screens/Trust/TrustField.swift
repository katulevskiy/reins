import SwiftUI

// A bypass, an Overnight or a lockdown running, as a field on top of Trust (iOS-Autopilot.png, 02 and 03): a bypass in
// red, Overnight in violet, Lockdown in amber. The time left in the serif, what it does in a sentence with one italic
// word, what still asks, and one button that ends it. Under it, what happened while it ran.

/// Which field a running mode paints Trust with; nil when none runs.
enum TrustRunning: Equatable {
    case bypass(until: Int64)
    case overnight(until: Int64)
    case lockdown

    static func of(_ s: AutopilotSettings) -> TrustRunning? {
        switch s.mode {
        case .bypass: s.bypassUntil.map { .bypass(until: $0) }
        case .overnight: s.overnightUntil.map { .overnight(until: $0) }
        case .lockdown: .lockdown
        default: nil
        }
    }

    var field: FieldKind {
        switch self {
        case .bypass: .delete
        case .overnight: .grant
        case .lockdown: .warn
        }
    }

    /// "Autopilot · Bypass".
    var label: String {
        switch self {
        case .bypass: "Autopilot · Bypass"
        case .overnight: "Autopilot · \(AutopilotText.overnightName)"
        case .lockdown: "Autopilot · Lockdown"
        }
    }

    /// The hero sentence: its lead, and the one italic word.
    var hero: (lead: String, italic: String) {
        switch self {
        case .bypass: ("Approving ", "everything")
        case .overnight: ("\(AutopilotText.overnightName) ", "decides")
        case .lockdown: ("Every request ", "is denied")
        }
    }

    /// What its list under the field holds: who decided the entries made while it runs.
    var decidedBy: String {
        switch self {
        case .bypass: "bypass"
        case .overnight: "overnight"
        case .lockdown: "lockdown"
        }
    }

    /// "During this bypass", "During this Overnight", "Denied while locked".
    var listTitle: String {
        switch self {
        case .bypass: "During this bypass"
        case .overnight: "During this \(AutopilotText.overnightName)"
        case .lockdown: "Denied while locked"
        }
    }

    /// The entries it decided: since it started, for a bypass or an Overnight; the latest ones for a lockdown, whose
    /// start the core does not keep.
    func entries(_ all: [ActivityEntry], settings s: AutopilotSettings) -> [ActivityEntry] {
        let since: Int64? = switch self {
        case .bypass: s.bypassSince
        case .overnight: s.overnightSince
        case .lockdown: nil
        }
        let mine = all.filter { entry in entry.decidedBy == decidedBy && (since.map { entry.at >= $0 } ?? true) }
        return Array(mine.prefix(20))
    }
}

/// The running mode's field.
struct TrustRunningField: View {
    var running: TrustRunning
    var settings: AutopilotSettings
    /// The status bar's height: the field runs under it.
    var top: CGFloat
    var onStopBypass: () -> Void
    var onStopOvernight: () -> Void
    var onEndLockdown: () -> Void

    var body: some View {
        SignalField(kind: running.field) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 8) {
                    if running != .lockdown {
                        Circle().fill(Palette.onField).frame(width: 7, height: 7).accessibilityHidden(true)
                    } else {
                        GlyphIcon(.lock, size: 14)
                    }
                    Text(running.label).textRole(.label).foregroundStyle(Palette.onField2).lineLimit(1)
                    Spacer(minLength: 0)
                    if let until = until, AutopilotText.isUntilOff(until) {
                        GlyphIcon(.infinity, size: 20)
                            .accessibilityLabel("On until you stop it")
                    }
                }
                if let until, !AutopilotText.isUntilOff(until) {
                    clock(until)
                        .padding(.top, 18)
                }
                AskText([AskText.Run(text: running.hero.lead), AskText.Run(text: running.hero.italic, target: true)], role: .ask, lineLimit: nil)
                    .foregroundStyle(Palette.onField)
                    .padding(.top, until.map { AutopilotText.isUntilOff($0) } ?? true ? 22 : 4)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(running.hero.lead + running.hero.italic)
                    .accessibilityIdentifier("heroMode")
                line
                    .font(RFont.sans(14.5))
                    .foregroundStyle(Palette.onField2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 8)
                button.padding(.top, 18)
            }
            .padding(.horizontal, 22)
            .padding(.top, top + 6)
            .padding(.bottom, 22)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("runningField")
    }

    private var until: Int64? {
        switch running {
        case let .bypass(until), let .overnight(until): until
        case .lockdown: nil
        }
    }

    /// "47:12 left": the clock in the serif, "left" in italic; its own clock.
    private func clock(_ until: Int64) -> some View {
        LiveClock(interval: 1) { now in
            HStack(alignment: .lastTextBaseline, spacing: 6) {
                Text(verbatim: AutopilotText.clock(until: until, now: Int64(now)))
                    .font(RFont.serif(58))
                    .monospacedDigit()
                Text("left").font(RFont.serif(26, italic: true))
            }
            .foregroundStyle(Palette.onField)
            .lineLimit(1)
            .minimumScaleFactor(0.7)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("\(running == .lockdown ? "" : AutopilotText.name(running.mode)), \(AutopilotText.timeLeft(until: until, now: Int64(now)))")
            .accessibilityIdentifier(running.clockIdentifier)
        }
    }

    /// What it does, or (without an end) how long it has run; its own clock.
    @ViewBuilder private var line: some View {
        switch running {
        case let .bypass(until):
            LiveClock(interval: 30) { now in
                Text(AutopilotText.bypassLine(until: until, since: settings.bypassSince, now: Int64(now)) + ". New connections and new phones still ask.")
                    .accessibilityIdentifier("bypassLine")
            }
            .overlay { if AutopilotText.isUntilOff(until) { clockAnchor(until) } }
        case let .overnight(until):
            LiveClock(interval: 30) { now in
                Text(AutopilotText.overnightLine(until: until, since: settings.overnightSince, now: Int64(now)))
                    .accessibilityIdentifier("overnightLine")
            }
            .overlay { if AutopilotText.isUntilOff(until) { clockAnchor(until) } }
        case .lockdown:
            Text("Everything that would ask is denied until you end it.")
        }
    }

    /// A bypass or an Overnight without an end has no clock to show; its identifier stays for what checks it is on.
    private func clockAnchor(_ until: Int64) -> some View {
        Color.clear
            .accessibilityElement()
            .accessibilityLabel("\(AutopilotText.name(running.mode)), on until you stop it")
            .accessibilityIdentifier(running.clockIdentifier)
    }

    @ViewBuilder private var button: some View {
        switch running {
        case .bypass:
            Button("Stop now", action: onStopBypass)
                .buttonStyle(SignalButtonStyle(.onField))
                .accessibilityIdentifier("stopBypass")
        case .overnight:
            Button("Stop now", action: onStopOvernight)
                .buttonStyle(SignalButtonStyle(.onField))
                .accessibilityIdentifier("stopOvernight")
        case .lockdown:
            Button(action: onEndLockdown) {
                Label("End lockdown", systemImage: "lock.open.fill")
            }
            .buttonStyle(SignalButtonStyle(.onField))
            .accessibilityIdentifier("endLockdown")
        }
    }
}

private extension TrustRunning {
    var mode: AutopilotMode {
        switch self {
        case .bypass: .bypass
        case .overnight: .overnight
        case .lockdown: .lockdown
        }
    }

    var clockIdentifier: String {
        switch self {
        case .bypass: "bypassClock"
        case .overnight: "overnightClock"
        case .lockdown: "lockdownClock"
        }
    }
}
