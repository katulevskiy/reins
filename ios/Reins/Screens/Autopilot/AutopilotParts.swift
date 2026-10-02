import SwiftUI

// The pieces Autopilot's screens share (the Android app's `AutopilotParts`): each mode's colour and symbol, tiles,
// rings and meters, the mode picker and chips. (Activity's mode pill, the approval sheet's suggestion and the
// activity badge live with those screens.)

enum AutopilotStyle {
    /// Neutral for Manual, blue for Assisted, the accent for Auto, red for Bypass, amber for Lockdown.
    static func tint(_ mode: AutopilotMode) -> Color {
        switch mode {
        case .manual: Palette.secondary
        case .assisted: Palette.search
        case .auto: Palette.accent
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }

    static func tint(_ verdict: Verdict) -> Color {
        switch verdict {
        case .approve: Palette.success
        case .deny: Palette.danger
        case .ask: Palette.search
        }
    }

    static func symbol(_ verdict: Verdict) -> String {
        switch verdict {
        case .approve: "checkmark"
        case .deny: "xmark"
        case .ask: "hand.raised.fill"
        }
    }

    /// The faint track under rings and meters.
    static let track = Palette.controlFill
}

/// A rounded square holding a symbol on a wash of its colour; `filled` turns it solid.
struct IconTile: View {
    var symbol: String
    var tint: Color
    var size: CGFloat = 40
    var filled = false

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
            .fill(filled ? tint : tint.opacity(0.13))
            .frame(width: size, height: size)
            .overlay {
                Image(systemName: symbol)
                    .font(.system(size: size * 0.42, weight: .semibold))
                    .foregroundStyle(filled ? Color.white : tint)
            }
            .animation(.smooth(duration: 0.25), value: filled)
            .accessibilityHidden(true)
    }
}

/// A ring from twelve o'clock over a faint track. It fills from empty when it appears, then springs to new values;
/// `content` sits in the middle.
struct ProgressRing<Content: View>: View {
    var fraction: Double
    var tint: Color
    var size: CGFloat = 52
    var stroke: CGFloat = 5
    @ViewBuilder var content: Content
    @State private var shown: Double = 0

    var body: some View {
        ZStack {
            Circle().stroke(AutopilotStyle.track, lineWidth: stroke)
            Circle()
                .trim(from: 0, to: max(shown, 0.0001))
                .stroke(tint, style: StrokeStyle(lineWidth: stroke, lineCap: .round))
                .rotationEffect(.degrees(-90))
                .opacity(shown > 0.001 ? 1 : 0)
            content
        }
        .padding(stroke / 2)
        .frame(width: size, height: size)
        .onAppear { withAnimation(.spring(response: 0.7, dampingFraction: 0.85)) { shown = clamped } }
        .onChange(of: fraction) { withAnimation(.spring(response: 0.6, dampingFraction: 0.85)) { shown = clamped } }
    }

    private var clamped: Double { min(max(fraction, 0), 1) }
}

extension ProgressRing where Content == EmptyView {
    init(fraction: Double, tint: Color, size: CGFloat = 52, stroke: CGFloat = 5) {
        self.init(fraction: fraction, tint: tint, size: size, stroke: stroke) { EmptyView() }
    }
}

/// A thin bar that fills to `fraction` in `tint`, over the same faint track; nil sweeps a band across (unknown size).
struct MeterBar: View {
    var fraction: Double?
    var tint: Color
    var height: CGFloat = 6
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(AutopilotStyle.track)
                if let fraction {
                    Capsule().fill(tint)
                        .frame(width: geo.size.width * min(max(fraction, 0), 1))
                        .animation(.spring(response: 0.5, dampingFraction: 0.9), value: fraction)
                } else {
                    TimelineView(.animation(paused: reduceMotion)) { context in
                        let phase = reduceMotion ? 0.35 : context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1.3) / 1.3
                        let band = geo.size.width * 0.35
                        Capsule().fill(tint)
                            .frame(width: band)
                            .offset(x: (geo.size.width + band) * phase - band)
                    }
                    .clipShape(Capsule())
                }
            }
        }
        .frame(height: height)
    }
}

/// "Approve  ███████░ 97%": one labelled probability.
struct ProbabilityRow: View {
    var label: String
    var p: Float
    var tint: Color

    var body: some View {
        HStack(spacing: 10) {
            Text(label)
                .font(RFont.sans(13.5, .medium))
                .foregroundStyle(Palette.secondary)
                .frame(width: 92, alignment: .leading)
                .lineLimit(1)
            MeterBar(fraction: Double(p), tint: tint)
            Text(AutopilotText.percent(p))
                .font(RFont.mono(13, .medium))
                .foregroundStyle(Palette.text)
                .frame(width: 44, alignment: .trailing)
                .lineLimit(1)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(label) \(AutopilotText.percent(p))")
    }
}

/// A remembered decision like the one at hand: what the user did, what it was, how alike.
struct AutopilotNeighbourRow: View {
    var neighbour: NeighbourView

    var body: some View {
        let tint = AutopilotStyle.tint(neighbour.verdict)
        HStack(spacing: 10) {
            Circle().fill(tint.opacity(0.14))
                .frame(width: 22, height: 22)
                .overlay {
                    Image(systemName: AutopilotStyle.symbol(neighbour.verdict))
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(tint)
                }
            VStack(alignment: .leading, spacing: 1) {
                Text(untrusted(neighbour.label))
                    .font(RFont.sans(14, .medium))
                    .foregroundStyle(Palette.text)
                    .lineLimit(2)
                Text("\(AutopilotText.pastVerdict(neighbour.verdict)) · \(Self.relative(neighbour.at))")
                    .font(RFont.sans(12))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            Text("\(AutopilotText.percent(neighbour.similarity)) alike")
                .font(RFont.mono(12, .medium))
                .foregroundStyle(Palette.secondary)
                .lineLimit(1)
        }
        .padding(.vertical, 6)
        .accessibilityElement(children: .combine)
    }

    static func relative(_ at: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(at))
        if abs(date.timeIntervalSinceNow) < 60 { return "just now" }
        return date.formatted(.relative(presentation: .named))
    }
}

/// A choice in a row of choices: tinted when chosen. Plays the selection.
struct AutopilotChip: View {
    var title: String
    var selected: Bool
    var identifier: String? = nil
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            feedback.play(.selection)
            action()
        } label: {
            Text(title)
                .font(RFont.sans(14.5, selected ? .semibold : .medium))
                .foregroundStyle(selected ? Palette.accent : Palette.text)
                .lineLimit(1)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(selected ? Palette.accentSoft : Palette.controlFill, in: Capsule())
                .overlay(Capsule().strokeBorder(selected ? Palette.accent.opacity(0.6) : .clear, lineWidth: 1.25))
                .contentShape(Capsule())
        }
        .buttonStyle(PressDim())
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier(identifier ?? title)
    }
}

/// A round selection mark that fills with a spring.
struct RadioDot: View {
    var selected: Bool
    var tint: Color

    var body: some View {
        Circle()
            .strokeBorder(selected ? tint : Palette.tertiary.opacity(0.6), lineWidth: 1.75)
            .frame(width: 22, height: 22)
            .overlay {
                Circle().fill(tint)
                    .frame(width: 12, height: 12)
                    .scaleEffect(selected ? 1 : 0.001)
            }
            .animation(.spring(response: 0.3, dampingFraction: 0.55), value: selected)
            .accessibilityHidden(true)
    }
}

/// The big mode selector: five rows and one highlight that springs from the old row to the new one, taking on the
/// new mode's colour on the way. Each row says what the mode does; Assisted and Auto say when they need the model.
struct ModePicker: View {
    var selected: AutopilotMode
    var modelReady: Bool
    var identifierPrefix = "mode"
    var onPick: (AutopilotMode) -> Void
    @Namespace private var highlight

    var body: some View {
        VStack(spacing: 0) {
            ForEach(AutopilotText.modes, id: \.self) { mode in
                row(mode)
            }
        }
        .padding(6)
        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 0.5))
        .animation(.spring(response: 0.38, dampingFraction: 0.78), value: selected)
    }

    private func row(_ mode: AutopilotMode) -> some View {
        let on = mode == selected
        let tint = AutopilotStyle.tint(mode)
        let needsModel = !modelReady && AutopilotText.needsModel(mode)
        return Button {
            onPick(mode)
        } label: {
            HStack(spacing: 14) {
                IconTile(symbol: AutopilotText.symbol(mode), tint: tint, size: 42, filled: on)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 8) {
                        Text(AutopilotText.name(mode))
                            .font(RFont.sans(16.5, .semibold))
                            .foregroundStyle(Palette.text)
                        if needsModel {
                            Text("NEEDS MODEL")
                                .font(RFont.sans(10, .semibold))
                                .tracking(0.6)
                                .foregroundStyle(Palette.tertiary)
                                .padding(.horizontal, 7)
                                .padding(.vertical, 3)
                                .background(Palette.controlFill, in: Capsule())
                        }
                    }
                    Text(AutopilotText.line(mode))
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 10)
                RadioDot(selected: on, tint: tint)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 12)
            .frame(minHeight: 70)
            .background {
                if on {
                    RoundedRectangle(cornerRadius: 17, style: .continuous)
                        .fill(tint.opacity(0.11))
                        .overlay(RoundedRectangle(cornerRadius: 17, style: .continuous).strokeBorder(tint.opacity(0.35), lineWidth: 1))
                        .matchedGeometryEffect(id: "highlight", in: highlight)
                }
            }
            .contentShape(RoundedRectangle(cornerRadius: 17, style: .continuous))
        }
        .buttonStyle(PressDim())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
        .accessibilityHint(needsModel ? "Needs the on-device model" : "")
        .accessibilityIdentifier("\(identifierPrefix):\(AutopilotText.key(mode))")
    }
}

/// The bypass countdown: a ring that empties as the time runs out, and the minutes and seconds left.
struct BypassClock: View {
    var until: Int64
    var size: CGFloat = 112
    /// The core keeps only the end; the ring measures against the length the bypass was most likely given.
    @State private var length: Int64?

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let now = Int64(context.date.timeIntervalSince1970)
            let total = length ?? AutopilotText.bypassLength(leftSeconds: until - now)
            let left = max(until - now, 0)
            ProgressRing(fraction: Double(left) / Double(max(total, 1)), tint: Palette.danger, size: size, stroke: 7) {
                VStack(spacing: 0) {
                    Text(AutopilotText.clock(until: until, now: now))
                        .font(RFont.mono(24, .semibold))
                        .foregroundStyle(Palette.text)
                        .monospacedDigit()
                        .accessibilityIdentifier("bypassClock")
                    Text("left").font(RFont.sans(12)).foregroundStyle(Palette.secondary)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Bypass, \(AutopilotText.minutesLeft(until: until, now: now))")
        }
        .onAppear { length = AutopilotText.bypassLength(leftSeconds: until - Int64(Date().timeIntervalSince1970)) }
        .onChange(of: until) { length = AutopilotText.bypassLength(leftSeconds: until - Int64(Date().timeIntervalSince1970)) }
    }
}

/// A profile's emoji on a soft disc.
struct ProfileAvatar: View {
    var profile: ProfileView
    var size: CGFloat

    var body: some View {
        Circle().fill(Palette.accentSoft)
            .frame(width: size, height: size)
            .overlay { Text(AutopilotText.profileIcon(profile)).font(.system(size: size * 0.48)) }
            .accessibilityHidden(true)
    }
}

/// A small caption inside a card ("LEARNS INTO").
struct AutopilotCaption: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text.uppercased())
            .font(RFont.sans(11.5, .semibold))
            .tracking(0.6)
            .foregroundStyle(Palette.tertiary)
            .accessibilityAddTraits(.isHeader)
    }
}

/// An info line with its symbol, for what holds a suggestion back.
struct AutopilotNoteRow: View {
    var text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "info.circle").font(.system(size: 14)).foregroundStyle(Palette.secondary)
            Text(text).font(RFont.sans(13.5)).foregroundStyle(Palette.secondary).fixedSize(horizontal: false, vertical: true)
        }
    }
}
