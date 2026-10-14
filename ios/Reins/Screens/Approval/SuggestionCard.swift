import SwiftUI

/// Autopilot's reasons, in a sheet the hero's pill opens (the Android app's AutopilotSheet): the verdict and how sure,
/// one line why, the likelihoods, the decisions of the user's it is like, and a way to its settings.
struct AutopilotSheet: View {
    var suggestion: SuggestionView
    var onSettings: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let s = suggestion
        let tint = s.judged && !s.floor ? SuggestionText.verdictTint(s.verdict) : Palette.secondary
        let p = s.verdict == .deny ? s.pDeny : s.pApprove
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if s.judged && !s.floor {
                        (Text(ApprovalWords.verdict(s.verdict)).font(RFont.sans(30, .semibold))
                            + Text("  ") + Text(SuggestionText.percent(p)).font(RFont.mono(24, .semibold)))
                            .foregroundStyle(tint)
                    } else {
                        Text(s.floor ? "Always asked" : "Not scored").font(RFont.sans(26, .semibold)).foregroundStyle(tint)
                    }
                    if !s.reason.trimmingCharacters(in: .whitespaces).isEmpty {
                        Text(untrusted(s.reason)).font(RFont.sans(14.5)).foregroundStyle(Palette.secondary).lineLimit(3).padding(.top, 4)
                    }
                    if s.judged {
                        VStack(spacing: 12) {
                            LikelihoodRow(label: "Approve", p: s.pApprove, color: Palette.success)
                            LikelihoodRow(label: "Deny", p: s.pDeny, color: Palette.danger)
                            LikelihoodRow(label: "Confidence", p: s.confidence, color: Palette.accent)
                        }
                        .padding(.top, 20)
                    }
                    if !s.neighbours.isEmpty {
                        Text("YOUR PAST DECISIONS")
                            .font(RFont.sans(12, .semibold))
                            .tracking(1)
                            .foregroundStyle(Palette.tertiary)
                            .padding(.top, 24)
                            .padding(.bottom, 4)
                        ForEach(Array(s.neighbours.prefix(4).enumerated()), id: \.offset) { i, n in
                            NeighbourRow(neighbour: n).accessibilityIdentifier("neighbour:\(i)")
                        }
                    }
                    ForEach(SuggestionText.notes(s), id: \.self) { note in
                        Label(note, systemImage: "info.circle")
                            .font(RFont.sans(13.5))
                            .foregroundStyle(Palette.secondary)
                            .padding(.top, 12)
                    }
                    Rectangle().fill(Palette.hairline).frame(height: 1).padding(.top, 14)
                    Button(action: onSettings) {
                        HStack {
                            Text("Autopilot settings").font(RFont.sans(15, .medium))
                            Spacer()
                            Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold))
                        }
                        .foregroundStyle(Palette.accent)
                        .padding(.vertical, 14)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("autopilotSettings")
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 20)
            }
            .navigationTitle("Autopilot")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button { dismiss() } label: { Image(systemName: "xmark") }
                        .accessibilityLabel("Close")
                        .accessibilityIdentifier("closeOverlay")
                }
            }
        }
        .presentationDetents([.medium, .large])
        .accessibilityContainer("suggestionDetail")
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
        if s.floor { out.append("Passwords, deletions, new connections, SSH and other risky requests always wait for you in Manual and Auto.") }
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

struct NeighbourRow: View {
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
