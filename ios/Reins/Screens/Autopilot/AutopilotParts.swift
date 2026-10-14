import SwiftUI

// The pieces Autopilot's screens share (the Android app's `AutopilotParts`): each mode's colour and symbol, tiles,
// rings and meters, stat tiles, segments and chips. (Activity's mode pill, the approval sheet's suggestion and the
// activity badge live with those screens.)

enum AutopilotStyle {
    /// Blue for Manual, the violet accent for Auto, red for Bypass, amber for Lockdown.
    static func tint(_ mode: AutopilotMode) -> Color {
        switch mode {
        case .manual: Palette.search
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

/// The press of a Manual or Auto card (the Autopilot page's and the quick menu's): a slight shrink while the finger is
/// down and nothing else. No fade, so no card can be left looking dimmed or pressed after Manual, Auto, Manual, Auto
/// in quick succession; whether a card is chosen is only ever its border and wash.
struct ModeCardPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.98 : 1)
            .animation(.spring(duration: 0.2), value: configuration.isPressed)
    }
}

/// A choice in a row of choices: the shared `SelectChip` with its identifier. Plays the selection.
struct AutopilotChip: View {
    var title: String
    var selected: Bool
    var identifier: String? = nil
    var action: () -> Void

    var body: some View {
        SelectChip(title: title, selected: selected, action: action)
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

/// A running bypass's ring: it empties as the time runs out, with the clock in the middle; a bypass without an end is
/// a full ring around an infinity sign.
struct BypassRing: View {
    var until: Int64
    var since: Int64?
    var size: CGFloat = 76
    var stroke: CGFloat = 5

    var body: some View {
        LiveClock(interval: 1) { time in
            let now = Int64(time)
            Group {
                if AutopilotText.isUntilOff(until) {
                    Circle()
                        .strokeBorder(Palette.danger, lineWidth: stroke)
                        .frame(width: size, height: size)
                        .overlay {
                            Image(systemName: "infinity")
                                .font(.system(size: size * 0.38, weight: .semibold))
                                .foregroundStyle(Palette.danger)
                        }
                } else {
                    ProgressRing(fraction: AutopilotText.bypassFraction(until: until, since: since, now: now), tint: Palette.danger, size: size, stroke: stroke) {
                        Text(AutopilotText.clock(until: until, now: now))
                            .font(RFont.mono(size * 0.21, .semibold))
                            .foregroundStyle(Palette.danger)
                            .monospacedDigit()
                            .lineLimit(1)
                            .minimumScaleFactor(0.6)
                            .padding(.horizontal, stroke + 2)
                    }
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Bypass, \(AutopilotText.timeLeft(until: until, now: now))")
            .accessibilityIdentifier("bypassClock")
        }
    }
}

/// A number and what it counts, on its own small card ("142 requests scored"); `suffix` follows the number smaller
/// ("2 /9").
struct StatTile: View {
    var value: String
    var label: String
    var tint: Color = Palette.text
    var suffix: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline, spacing: 3) {
                Text(value)
                    .font(RFont.sans(22, .semibold))
                    .foregroundStyle(tint)
                if let suffix {
                    Text(suffix)
                        .font(RFont.sans(15))
                        .foregroundStyle(Palette.tertiary)
                }
            }
            .lineLimit(1)
            .minimumScaleFactor(0.6)
            Text(label)
                .font(RFont.sans(11.5))
                .foregroundStyle(Palette.tertiary)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 1))
        .accessibilityElement(children: .combine)
    }
}

/// One choice of a row of segments: its title, an optional leading symbol and trailing chevron, its colour.
struct SegmentItem<ID: Hashable>: Identifiable {
    var id: ID
    var title: String
    var symbol: String? = nil
    var chevron = false
    var tint: Color? = nil
    var identifier: String
}

/// A segmented control in Reins's look: a dark well and the chosen segment raised on it. Unlike the system's, a
/// segment can carry a symbol, a chevron (it opens more choices) and a colour of its own. Plays the selection.
struct Segments<ID: Hashable>: View {
    var items: [SegmentItem<ID>]
    var selected: ID?
    var onPick: (ID) -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 2) {
            ForEach(items) { item in
                let on = item.id == selected
                Button {
                    feedback.play(.selection)
                    onPick(item.id)
                } label: {
                    HStack(spacing: 3) {
                        if let symbol = item.symbol {
                            Image(systemName: symbol).font(.system(size: 12, weight: .semibold))
                        }
                        Text(item.title)
                            .font(RFont.sans(14, on ? .semibold : .medium))
                            .lineLimit(1)
                            .minimumScaleFactor(0.75)
                        if item.chevron {
                            Image(systemName: "chevron.down").font(.system(size: 10, weight: .semibold))
                        }
                    }
                    .foregroundStyle(item.tint ?? (on ? Palette.text : Palette.secondary))
                    .frame(maxWidth: .infinity, minHeight: 36)
                    .padding(.horizontal, 2)
                    .background {
                        if on {
                            RoundedRectangle(cornerRadius: 10, style: .continuous)
                                .fill(Palette.dynamic(0xFFFFFF, 0x2C2C31))
                                .shadow(color: .black.opacity(0.08), radius: 2, y: 1)
                        }
                    }
                    .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
                .accessibilityIdentifier(item.identifier)
            }
        }
        .padding(3)
        .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
        .animation(.smooth(duration: 0.2), value: selected)
    }
}

/// A line of text with a (?) after it that shows the longer explanation under it.
struct HelpLine: View {
    var text: String
    var help: String
    var alignment: HorizontalAlignment = .leading
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: alignment, spacing: 8) {
            HStack(spacing: 6) {
                Text(text)
                    .font(RFont.sans(14))
                    .foregroundStyle(Palette.secondary)
                    .multilineTextAlignment(alignment == .center ? .center : .leading)
                    .fixedSize(horizontal: false, vertical: true)
                HelpMark(open: open) {
                    let next = !open
                    withAnimation(.smooth(duration: 0.25)) { open = next }
                    feedback.play(.expand(next))
                }
            }
            if open {
                Text(help)
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.secondary)
                    .multilineTextAlignment(alignment == .center ? .center : .leading)
                    .fixedSize(horizontal: false, vertical: true)
                    .transition(.opacity)
                    .accessibilityIdentifier("helpText")
            }
        }
    }
}

/// The (?) that shows or hides an explanation.
struct HelpMark: View {
    var open: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "questionmark.circle")
                .font(.system(size: 16))
                .foregroundStyle(open ? Palette.accent : Palette.tertiary)
                .frame(width: 30, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(open ? "Hide help" : "Help")
        .accessibilityIdentifier("help")
    }
}

/// A profile's emoji on a rounded tile (its row and its page).
struct ProfileTile: View {
    var profile: ProfileView
    var size: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.31, style: .continuous)
            .fill(Palette.chip)
            .frame(width: size, height: size)
            .overlay { Text(AutopilotText.profileIcon(profile)).font(.system(size: size * 0.52)) }
            .accessibilityHidden(true)
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
