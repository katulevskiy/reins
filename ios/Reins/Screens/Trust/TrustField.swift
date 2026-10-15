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

    /// The entries it decided, newest first: since it started, for a bypass or an Overnight; the latest ones for a
    /// lockdown, whose start the core does not keep. All of them: the list folds to its latest few (`RunningListFold`).
    func entries(_ all: [ActivityEntry], settings s: AutopilotSettings) -> [ActivityEntry] {
        let since: Int64? = switch self {
        case .bypass: s.bypassSince
        case .overnight: s.overnightSince
        case .lockdown: nil
        }
        return all
            .filter { entry in entry.decidedBy == decidedBy && (since.map { entry.at >= $0 } ?? true) }
            .sorted { $0.at != $1.at ? $0.at > $1.at : $0.id > $1.id }
    }
}

/// The list under a running mode's field folds to its latest three, so a long bypass does not push Grants and the
/// profiles out of reach: "Show all 97" opens the rest in place, "Show less" folds it back. Folded each time Trust opens.
enum RunningListFold {
    static let folded = 3

    /// How many rows show.
    static func shown(_ total: Int, expanded: Bool) -> Int { expanded ? total : min(total, folded) }

    /// How many the fold hides (the number on "Show all"); nil when it hides none.
    static func hidden(_ total: Int) -> Int? { total > folded ? total - folded : nil }

    /// "Show all 97".
    static func showAll(_ total: Int) -> String? { hidden(total).map { "Show all \($0)" } }
}

/// The running mode's field. It folds into a compact bar as Trust scrolls (`TrustFold`, `RunningFieldLayout`): the
/// label line and the timer stay, the sentence fades and closes, the button moves to the right of the timer. Every
/// element keeps its label and identifier in both states.
struct TrustRunningField: View {
    var running: TrustRunning
    var settings: AutopilotSettings
    /// The page's scroll, and where the field's heights are written.
    var fold: TrustFold
    var onStopBypass: () -> Void
    var onStopOvernight: () -> Void
    var onEndLockdown: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let progress = fold.progress(reduceMotion: reduceMotion)
        ZStack(alignment: .top) {
            // Under Reduce Motion the progress is 0 or 1 and each state is its own view, so a change cross-fades
            // rather than moving anything.
            folded(progress)
                .id(reduceMotion ? Int(progress) : -1)
                .transition(.opacity)
        }
        .animation(reduceMotion ? .easeInOut(duration: Motion.fade) : nil, value: progress)
        .foregroundStyle(Palette.onField)
        .tint(Palette.onField)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("runningField")
    }

    private func folded(_ progress: CGFloat) -> some View {
        let fade = TrustFold.heroOpacity(progress)
        return RunningFieldLayout(progress: progress) {
            FieldSurface(kind: running.field)
                .clipShape(UnevenRoundedRectangle(bottomLeadingRadius: SignalField<EmptyView>.radius, bottomTrailingRadius: SignalField<EmptyView>.radius, style: .continuous))
                .ignoresSafeArea(edges: .top)
                .accessibilityHidden(true)
                .layoutValue(key: RunningFieldPart.self, value: .surface)
            labelLine
                .layoutValue(key: RunningFieldPart.self, value: .label)
            if let until, !AutopilotText.isUntilOff(until) {
                clock(until)
                    .scaleEffect(RunningFieldGeometry.clockScale(progress), anchor: .topLeading)
                    .layoutValue(key: RunningFieldPart.self, value: .clock)
            }
            // Faded by its colour, not its opacity, so VoiceOver still reads it when compact; it takes no touches.
            VStack(alignment: .leading, spacing: 8) {
                AskText([AskText.Run(text: running.hero.lead), AskText.Run(text: running.hero.italic, target: true)], role: .ask, lineLimit: nil)
                    .foregroundStyle(Palette.onField.opacity(fade))
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(running.hero.lead + running.hero.italic)
                    .accessibilityIdentifier("heroMode")
                line
                    .font(RFont.sans(14.5))
                    .foregroundStyle(Palette.onField2.opacity(fade))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .allowsHitTesting(false)
            .layoutValue(key: RunningFieldPart.self, value: .hero)
            button(height: RunningFieldGeometry.buttonHeight(progress))
                .layoutValue(key: RunningFieldPart.self, value: .button)
            // Where the compact field ends, for the page to know how far the fold runs.
            Color.clear
                .frame(width: 0, height: 0)
                .accessibilityHidden(true)
                .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.space)).minY } action: { fold.compact = $0 }
                .layoutValue(key: RunningFieldPart.self, value: .compactEdge)
        }
        .coordinateSpace(.named(Self.space))
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { fold.open = $0 }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(TrustFold.stateIdentifier(progress))
    }

    private static let space = "runningField"

    /// The dot (the lock for a lockdown), "AUTOPILOT · BYPASS", and the infinity sign for a mode without an end.
    private var labelLine: some View {
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

    /// The one way out: across the width open, its own width at the timer's right compact (never under 44 pt tall).
    @ViewBuilder private func button(height: CGFloat) -> some View {
        switch running {
        case .bypass:
            Button("Stop now", action: onStopBypass)
                .buttonStyle(SignalButtonStyle(.onField, height: height))
                .accessibilityIdentifier("stopBypass")
        case .overnight:
            Button("Stop now", action: onStopOvernight)
                .buttonStyle(SignalButtonStyle(.onField, height: height))
                .accessibilityIdentifier("stopOvernight")
        case .lockdown:
            Button(action: onEndLockdown) {
                Label("End lockdown", systemImage: "lock.open.fill")
            }
            .buttonStyle(SignalButtonStyle(.onField, height: height))
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
