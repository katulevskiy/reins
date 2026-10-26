import SwiftUI
import UIKit

// A timed mode running (the Running board, C; the Autopilot board, 02 to 04): Trust becomes the mode's field. Its name
// and count sit in the Dynamic Island's ears (or a row under the status bar), then the big clock, what a bypass still
// asks, and Stop now. Scrolling collapses it to the clock and Stop on one line, then to the ears alone.

/// Which mode runs on Trust; nil when none does.
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

    var kind: ModeKind {
        switch self {
        case .bypass: .bypass
        case .overnight: .overnight
        case .lockdown: .lockdown
        }
    }

    var mode: AutopilotMode {
        switch self {
        case .bypass: .bypass
        case .overnight: .overnight
        case .lockdown: .lockdown
        }
    }

    /// When it ends on its own; nil for a lockdown (the core keeps no start or end for it).
    var until: Int64? {
        switch self {
        case let .bypass(until), let .overnight(until): until
        case .lockdown: nil
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

    /// The list's header: "During bypass", "Denied".
    var listTitle: String {
        switch self {
        case .bypass: "During bypass"
        case .overnight: "During \(AutopilotText.overnightName.lowercased())"
        case .lockdown: "Denied"
        }
    }

    /// The right ear's glyph: approved in a bypass, held overnight, denied in a lockdown.
    var countGlyph: Glyph {
        switch self {
        case .bypass: .check
        case .overnight: .pause
        case .lockdown: .close
        }
    }

    /// The right ear, spoken: "4 approved".
    func countSpoken(_ n: Int) -> String {
        switch self {
        case .bypass: "\(n) approved"
        case .overnight: "\(n) decided"
        case .lockdown: "\(n) denied"
        }
    }

    /// How its entries end their row.
    var outcome: Outcome {
        switch self {
        case .bypass: .bypass
        case .overnight: .night
        case .lockdown: .locked
        }
    }

    var clockIdentifier: String {
        switch self {
        case .bypass: "bypassClock"
        case .overnight: "overnightClock"
        case .lockdown: "lockdownClock"
        }
    }

    var stopIdentifier: String {
        switch self {
        case .bypass: "stopBypass"
        case .overnight: "stopOvernight"
        case .lockdown: "endLockdown"
        }
    }

    /// The entries it decided, newest first: since it started, for a bypass or an Overnight; the latest ones for a
    /// lockdown, whose start the core does not keep.
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

/// The running list folds to its newest entry with the rest stacked under it ("+3"); tapped, it opens in place.
enum RunningListFold {
    /// How many rows show.
    static func shown(_ total: Int, expanded: Bool) -> Int { expanded ? total : min(total, 1) }

    /// How many the fold hides (the number on "+3"); nil when it hides none.
    static func hidden(_ total: Int) -> Int? { total > 1 ? total - 1 : nil }

    /// "Show all 3", what the "+3" says to VoiceOver.
    static func showAll(_ total: Int) -> String? { hidden(total).map { "Show all \($0)" } }

    /// The cards peeking out under the folded row: none, one or two.
    static func peeks(_ total: Int) -> Int { min(max(total - 1, 0), 2) }
}

/// The clock's words: "47:12", "5:59:12", "∞" without an end.
enum RunningClock {
    static func text(_ running: TrustRunning, now: Int64) -> String? {
        running.until.map { AutopilotText.clock(until: $0, now: now) }
    }

    static func spoken(_ running: TrustRunning, now: Int64) -> String {
        guard let until = running.until else { return running.kind.name }
        if AutopilotText.isUntilOff(until) { return "\(running.kind.name), on until you stop it" }
        return "\(running.kind.name), \(AutopilotText.timeLeft(until: until, now: now))"
    }
}

/// The colour of a running mode with its grain.
struct ModeSurface: View {
    var mode: ModeKind

    var body: some View {
        ZStack {
            mode.fill
            Grain()
        }
    }
}

/// The full field: the page's first row, from the very top of the screen. Its colour also runs up past the top, so
/// pulling to refresh never shows paper above it.
struct RunningFieldFull: View {
    var running: TrustRunning
    var placement: EarsPlacement
    /// It is the field in force (not scrolled under a pinned bar): its button carries the stop identifier.
    var current: Bool
    var onStop: () -> Void

    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            LiveClock(interval: 1) { now in
                if let text = RunningClock.text(running, now: Int64(now)) {
                    BigClock(text: text)
                        .accessibilityLabel(RunningClock.spoken(running, now: Int64(now)))
                        .accessibilityIdentifier(current ? running.clockIdentifier : "")
                } else {
                    Text(running.kind.effect)
                        .textRole(.hero)
                        .foregroundStyle(Palette.onField)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier(current ? running.clockIdentifier : "")
                }
            }
            if running.kind == .bypass {
                StillAsksRow()
                    .padding(.top, 12)
            }
            Button("Stop now", action: onStop)
                .buttonStyle(SignalButtonStyle(.onField))
                .padding(.top, 18)
                .accessibilityIdentifier(current ? running.stopIdentifier : "\(running.stopIdentifier)Scrolled")
        }
        .foregroundStyle(Palette.onField)
        .padding(.horizontal, 20)
        .padding(.top, placement.contentTop)
        .padding(.bottom, 20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background {
            GeometryReader { geo in
                let shape = UnevenRoundedRectangle(bottomLeadingRadius: 30, bottomTrailingRadius: 30, style: .continuous)
                VStack(spacing: 0) {
                    running.kind.fill.frame(height: 800)
                    ModeSurface(mode: running.kind).frame(height: geo.size.height)
                }
                .clipShape(shape)
                .offset(y: -800)
            }
            .accessibilityHidden(true)
        }
    }
}

/// The clock in the display face, 71 pt (`TextRole.clock`) and tabular, set in a box as tall as its size (the boards' line-height 1).
struct BigClock: View {
    var text: String
    var size: CGFloat = TextRole.clock.size

    var body: some View {
        let font = UIFont(name: RFont.displayName(false), size: size)
        let slack = max((font?.lineHeight ?? size * 1.26) - size, 0)
        Text(verbatim: text)
            .font(RFont.fixedDisplay(size))
            .tracking(RFont.displayTracking(size))
            .monospacedDigit()
            .lineLimit(1)
            .minimumScaleFactor(0.4)
            .foregroundStyle(Palette.onField)
            .padding(.vertical, -slack / 2)
            .accessibilityElement(children: .ignore)
    }
}

/// "Still asks  [New AI] [New phone]", wrapping at large text sizes.
struct StillAsksRow: View {
    var body: some View {
        ViewThatFits(in: .horizontal) {
            StillAsks()
            VStack(alignment: .leading, spacing: 6) {
                label
                HStack(spacing: 6) { pills }
            }
            VStack(alignment: .leading, spacing: 6) {
                label
                pills
            }
        }
        .foregroundStyle(.white)
        .accessibilityElement(children: .combine)
    }

    private var label: some View { Text("Still asks").font(RFont.sans(13, .semibold)).opacity(0.85) }

    @ViewBuilder private var pills: some View {
        Pill("New AI", tone: .onFieldWash, glyph: .link, size: .pill)
        Pill("New phone", tone: .onFieldWash, glyph: .phone, size: .pill)
    }
}

/// What floats over the top of Trust while a mode runs: the ears always, and the short or mini bar once the page has
/// scrolled. It carries the field's identifier and its state ("open", "short", "compact").
struct RunningOverlay: View {
    var running: TrustRunning
    var count: Int
    var placement: EarsPlacement
    var fold: TrustFold
    var onStop: () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let stage = fold.stage
        ZStack(alignment: .top) {
            if stage != .full {
                bar(stage)
                    .transition(.opacity)
            } else {
                // The ears' band in the field's colour, drawn as the top of the full field: what scrolls up under the
                // ears goes under it.
                ModeSurface(mode: running.kind)
                    .frame(height: max(fold.fieldBottom, placement.miniHeight))
                    .frame(height: placement.top + placement.height + 4, alignment: .top)
                    .clipped()
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
            ears(stage)
        }
        .frame(maxWidth: .infinity, alignment: .top)
        .animation(reduceMotion ? .easeInOut(duration: Motion.fade) : .spring(duration: 0.38, bounce: 0.28), value: stage)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("runningField")
        .accessibilityValue(stage.value)
    }

    /// The pinned bar: short (clock and Stop on one line, under the ears) or mini (the ears' height only).
    private func bar(_ stage: RunningStage) -> some View {
        let radius: CGFloat = stage == .mini ? 22 : 30
        let shape = UnevenRoundedRectangle(bottomLeadingRadius: radius, bottomTrailingRadius: radius, style: .continuous)
        return ZStack(alignment: .topLeading) {
            ModeSurface(mode: running.kind)
                .clipShape(shape)
                .accessibilityHidden(true)
            if stage == .short {
                HStack(spacing: 12) {
                    LiveClock(interval: 1) { now in
                        if let text = RunningClock.text(running, now: Int64(now)) {
                            BigClock(text: text, size: 41)
                                .accessibilityLabel(RunningClock.spoken(running, now: Int64(now)))
                                .accessibilityIdentifier(running.clockIdentifier)
                        } else {
                            Text(running.kind.effect)
                                .textRole(.cardAsk)
                                .foregroundStyle(Palette.onField)
                                .lineLimit(1)
                                .minimumScaleFactor(0.7)
                                .accessibilityIdentifier(running.clockIdentifier)
                        }
                    }
                    Spacer(minLength: 8)
                    Button("Stop", action: onStop)
                        .buttonStyle(SignalButtonStyle(.onField, size: .small))
                        .fixedSize()
                        .accessibilityLabel("Stop now")
                        .accessibilityIdentifier(running.stopIdentifier)
                }
                .padding(.horizontal, 20)
                .padding(.top, placement.contentTop - 6)
            }
        }
        .frame(height: stage == .mini ? placement.miniHeight : placement.shortHeight)
        .frame(maxWidth: .infinity)
    }

    /// The left ear: the mode's glyph and name (the clock, mini). The right: its count (Stop, mini).
    private func ears(_ stage: RunningStage) -> some View {
        HStack(alignment: .center, spacing: 0) {
            left(stage)
                .frame(maxWidth: placement.maxWidth, alignment: .leading)
                .allowsHitTesting(false)
            Spacer(minLength: 0)
            right(stage)
                .frame(maxWidth: placement.maxWidth, alignment: .trailing)
        }
        .frame(height: placement.height)
        .padding(.horizontal, placement.side)
        .padding(.top, placement.top)
        .foregroundStyle(Palette.onField)
        .dynamicTypeSize(...DynamicTypeSize.accessibility3)
    }

    private var earFont: Font { placement.accessibility ? RFont.sans(22, .semibold) : RFont.fixedSans(13.5, .semibold) }

    @ViewBuilder private func left(_ stage: RunningStage) -> some View {
        HStack(spacing: 5) {
            ModeGlyph(running.kind, size: placement.accessibility ? 20 : 16)
            if stage == .mini, running.until.map({ !AutopilotText.isUntilOff($0) }) ?? false {
                LiveClock(interval: 1) { now in
                    Text(verbatim: RunningClock.text(running, now: Int64(now)) ?? "")
                        .font(RFont.fixedMono(14, .medium))
                        .monospacedDigit()
                        .lineLimit(1)
                        .accessibilityLabel(RunningClock.spoken(running, now: Int64(now)))
                        .accessibilityIdentifier(running.clockIdentifier)
                }
            } else {
                Text(running.kind.name)
                    .font(earFont)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder private func right(_ stage: RunningStage) -> some View {
        if stage == .mini {
            Button(action: onStop) {
                Text("Stop")
                    .font(RFont.fixedSans(13.5, .semibold))
                    .foregroundStyle(Palette.onFieldInk)
                    .padding(.horizontal, 14)
                    .frame(height: 30)
                    .background(Color.white, in: Capsule())
                    .frame(minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(PressDimsStyle())
            .accessibilityLabel("Stop now")
            .accessibilityIdentifier(running.stopIdentifier)
        } else {
            HStack(spacing: 3) {
                GlyphIcon(running.countGlyph, size: placement.accessibility ? 20 : 16)
                Text("\(count)")
                    .font(earFont)
                    .monospacedDigit()
                    .lineLimit(1)
            }
            .allowsHitTesting(false)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(running.countSpoken(count))
        }
    }
}
