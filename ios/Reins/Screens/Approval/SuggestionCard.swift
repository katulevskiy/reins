import SwiftUI

/// What Autopilot made of the request (Assisted and Auto), as a card that opens to its reasons: how likely an approval
/// and a denial are, how sure it is, the decisions of the user's it is like, and why it may not judge.
struct SuggestionCard: View {
    var suggestion: SuggestionView
    @State private var open = false
    @Environment(\.feedback) private var feedback
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let s = suggestion
        let tint = SuggestionText.tint(s)
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.spring(duration: 0.35, bounce: 0.1)) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack(spacing: 12) {
                    if s.judged && !s.floor {
                        Ring(fraction: Double(s.verdict == .deny ? s.pDeny : s.pApprove), color: tint) {
                            Image(systemName: "sparkles").font(.system(size: 13, weight: .semibold)).foregroundStyle(tint)
                        }
                        .frame(width: 34, height: 34)
                    } else {
                        Image(systemName: s.floor ? "shield.fill" : "sparkles")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(tint)
                            .frame(width: 34, height: 34)
                            .background(tint.opacity(0.13), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(SuggestionText.headline(s))
                            .font(RFont.sans(15, .semibold))
                            .foregroundStyle(Palette.text)
                            .lineLimit(2)
                            .accessibilityIdentifier("suggestionHeadline")
                        if !open && s.judged && !s.reason.trimmingCharacters(in: .whitespaces).isEmpty {
                            Text(untrusted(s.reason)).font(RFont.sans(12.5)).foregroundStyle(Palette.secondary).lineLimit(1)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Palette.tertiary)
                        .rotationEffect(.degrees(open ? 90 : 0))
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("suggestionToggle")

            if open { detail(s).transition(.opacity) }
        }
        .background(tint.opacity(scheme == .dark ? 0.10 : 0.07), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(tint.opacity(0.25), lineWidth: 0.75))
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityIdentifier("suggestion")
    }

    @ViewBuilder private func detail(_ s: SuggestionView) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if s.judged {
                LikelihoodRow(label: "Approve", p: s.pApprove, color: Palette.success)
                LikelihoodRow(label: "Deny", p: s.pDeny, color: Palette.danger)
                LikelihoodRow(label: "Confidence", p: s.confidence, color: Palette.accent)
                if !s.reason.trimmingCharacters(in: .whitespaces).isEmpty {
                    Text(untrusted(s.reason)).font(RFont.sans(14)).foregroundStyle(Palette.text).padding(.top, 4)
                }
            }
            if !s.neighbours.isEmpty {
                Text("LIKE THESE DECISIONS OF YOURS")
                    .font(RFont.sans(11.5, .semibold))
                    .tracking(0.6)
                    .foregroundStyle(Palette.tertiary)
                    .padding(.top, 4)
                ForEach(Array(s.neighbours.prefix(4).enumerated()), id: \.offset) { i, n in
                    NeighbourRow(neighbour: n).accessibilityIdentifier("neighbour:\(i)")
                }
            }
            ForEach(SuggestionText.notes(s), id: \.self) { note in
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: "info.circle").font(.system(size: 14)).foregroundStyle(Palette.secondary)
                    Text(note).font(RFont.sans(13.5)).foregroundStyle(Palette.secondary)
                }
                .padding(.top, 2)
            }
            Text("Profile \(untrusted(s.profileName)) · decided on this phone")
                .font(RFont.sans(12))
                .foregroundStyle(Palette.tertiary)
                .padding(.top, 4)
        }
        .padding(.horizontal, 14)
        .padding(.bottom, 14)
        .accessibilityIdentifier("suggestionDetail")
    }
}

/// Autopilot's words on the approval sheet (the Android app's AutopilotText, the parts this sheet uses).
enum SuggestionText {
    static func percent(_ p: Float) -> String { "\(Int((min(max(p, 0), 1) * 100).rounded()))%" }

    /// "Autopilot would approve · 97%".
    static func headline(_ s: SuggestionView) -> String {
        if s.floor { return "Autopilot always asks you for this" }
        if !s.judged { return "Autopilot did not judge this" }
        switch s.verdict {
        case .approve: return "Autopilot would approve · \(percent(s.pApprove))"
        case .deny: return "Autopilot would deny · \(percent(s.pDeny))"
        case .ask: return "Autopilot would ask you · approve \(percent(s.pApprove))"
        }
    }

    /// Why a suggestion is limited, if it is: the hard floor, a target never seen, no model.
    static func notes(_ s: SuggestionView) -> [String] {
        var out: [String] = []
        if s.floor { out.append("Passwords, deletions, new connections, SSH and other risky requests always wait for you, in every mode.") }
        if s.novel { out.append("This target was never approved for this AI before, so Autopilot asks you even in Auto.") }
        if !s.judged && !s.floor && !s.reason.trimmingCharacters(in: .whitespaces).isEmpty { out.append(untrusted(s.reason)) }
        return out
    }

    static func tint(_ s: SuggestionView) -> Color {
        if s.floor { return Palette.secondary }
        if !s.judged { return Palette.tertiary }
        return verdictTint(s.verdict)
    }

    static func verdictTint(_ v: Verdict) -> Color {
        switch v {
        case .approve: Palette.success
        case .deny: Palette.danger
        case .ask: Palette.search
        }
    }

    static func pastVerdict(_ v: Verdict) -> String {
        switch v {
        case .approve: "Approved"
        case .deny: "Denied"
        case .ask: "Asked"
        }
    }
}

/// "Approve  ███████░ 97%": one labelled probability.
struct LikelihoodRow: View {
    var label: String
    var p: Float
    var color: Color

    var body: some View {
        HStack(spacing: 10) {
            Text(label).font(RFont.sans(13.5, .medium)).foregroundStyle(Palette.secondary).frame(width: 92, alignment: .leading)
            ProgressView(value: Double(min(max(p, 0), 1)))
                .tint(color)
                .progressViewStyle(.linear)
            Text(SuggestionText.percent(p)).font(RFont.mono(13, .medium)).foregroundStyle(Palette.text).frame(width: 44, alignment: .trailing)
        }
        .accessibilityElement(children: .combine)
    }
}

private struct NeighbourRow: View {
    var neighbour: NeighbourView

    var body: some View {
        let tint = SuggestionText.verdictTint(neighbour.verdict)
        HStack(spacing: 10) {
            Image(systemName: neighbour.verdict == .approve ? "checkmark" : neighbour.verdict == .deny ? "xmark" : "hand.raised")
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(tint)
                .frame(width: 22, height: 22)
                .background(tint.opacity(0.14), in: Circle())
            VStack(alignment: .leading, spacing: 1) {
                Text(untrusted(neighbour.label)).font(RFont.sans(14, .medium)).foregroundStyle(Palette.text).lineLimit(2)
                Text("\(SuggestionText.pastVerdict(neighbour.verdict)) · \(TimeText.relative(neighbour.at))")
                    .font(RFont.sans(12))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Text("\(SuggestionText.percent(neighbour.similarity)) alike").font(RFont.mono(12, .medium)).foregroundStyle(Palette.secondary)
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }
}

/// A ring from twelve o'clock over a faint track, with something in the middle.
private struct Ring<Inner: View>: View {
    var fraction: Double
    var color: Color
    @ViewBuilder var inner: Inner
    @State private var shown: Double = 0

    var body: some View {
        ZStack {
            Circle().stroke(Palette.controlFill, lineWidth: 3.5)
            Circle()
                .trim(from: 0, to: shown)
                .stroke(color, style: StrokeStyle(lineWidth: 3.5, lineCap: .round))
                .rotationEffect(.degrees(-90))
            inner
        }
        .onAppear { withAnimation(.spring(duration: 0.9, bounce: 0.15)) { shown = min(max(fraction, 0), 1) } }
        .onChange(of: fraction) { _, f in withAnimation(.spring(duration: 0.6)) { shown = min(max(f, 0), 1) } }
    }
}
