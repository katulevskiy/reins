import SwiftUI

// The pieces Autopilot's screens share (the Android app's `AutopilotParts`): each mode's colour and symbol, tiles,
// rings and meters, stat tiles, segments and chips. (Activity's mode pill, the approval sheet's suggestion and the
// activity badge live with those screens.)

enum AutopilotStyle {
    /// Blue for Manual, the violet accent for Auto, night indigo for Overnight, red for Bypass, amber for Lockdown.
    static func tint(_ mode: AutopilotMode) -> Color {
        switch mode {
        case .manual: Palette.kindRead
        case .auto: Palette.violet
        case .overnight: Palette.violet
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }

    static func tint(_ verdict: Verdict) -> Color {
        switch verdict {
        case .approve: Palette.success
        case .deny: Palette.danger
        case .ask: Palette.kindRead
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
    static let track = Palette.paperSunken
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
                .foregroundStyle(Palette.ink2)
                .frame(width: 92, alignment: .leading)
                .lineLimit(1)
            MeterBar(fraction: Double(p), tint: tint)
            Text(AutopilotText.percent(p))
                .font(RFont.mono(13, .medium))
                .foregroundStyle(Palette.ink)
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
                    .foregroundStyle(Palette.ink)
                    .lineLimit(2)
                Text("\(AutopilotText.pastVerdict(neighbour.verdict)) · \(Self.relative(neighbour.at))")
                    .font(RFont.sans(12))
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            Text("\(AutopilotText.percent(neighbour.similarity)) alike")
                .font(RFont.mono(12, .medium))
                .foregroundStyle(Palette.ink2)
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

/// A radio (the kit's `rad`): 22 pt, a 1.5 pt faint ring; chosen, a ring 7 pt deep in `tint` (violet), on a spring.
struct RadioDot: View {
    var selected: Bool
    var tint: Color = Palette.violetFill

    var body: some View {
        Circle()
            .strokeBorder(selected ? tint : Palette.inkFaint, lineWidth: selected ? 7 : 1.5)
            .frame(width: 22, height: 22)
            .animation(.spring(response: 0.3, dampingFraction: 0.55), value: selected)
            .accessibilityHidden(true)
    }
}

/// The (?) that shows or hides an explanation.
struct HelpMark: View {
    var open: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            GlyphIcon(.help, size: 17)
                .foregroundStyle(open ? Palette.violet : Palette.inkMeta)
                .frame(width: 32, height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(open ? "Hide help" : "Help")
        .accessibilityIdentifier("help")
    }
}

/// A profile's glyph on a plain tile (its row and its page): the icon it keeps, drawn as a glyph, never an emoji.
struct ProfileTile: View {
    var profile: ProfileView
    var size: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.31, style: .continuous)
            .fill(Palette.paperSunken)
            .frame(width: size, height: size)
            .overlay { GlyphIcon(AutopilotText.profileGlyph(AutopilotText.profileIcon(profile)), size: size * 0.46).foregroundStyle(Palette.ink) }
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
            .foregroundStyle(Palette.inkMeta)
            .accessibilityAddTraits(.isHeader)
    }
}

/// An info line with its symbol, for what holds a suggestion back.
struct AutopilotNoteRow: View {
    var text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "info.circle").font(.system(size: 14)).foregroundStyle(Palette.ink2)
            Text(text).font(RFont.sans(13.5)).foregroundStyle(Palette.ink2).fixedSize(horizontal: false, vertical: true)
        }
    }
}
