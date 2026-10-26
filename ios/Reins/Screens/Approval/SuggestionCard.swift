import SwiftUI

/// Autopilot's reasons (Next-Approve 05, the Android app's AutopilotSheet), from the field's pick: the verdict and how
/// sure, one line why, the risk it saw, what each bar measures, the user's similar decisions, and a way to Autopilot.
/// It suggests; the dock decides.
struct AutopilotSheet: View {
    var suggestion: SuggestionView
    var onSettings: () -> Void

    var body: some View {
        let s = suggestion
        let tint = s.judged && !s.floor ? SuggestionText.verdictTint(s.verdict) : Palette.ink2
        let p = s.verdict == .deny ? s.pDeny : s.pApprove
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    ModeGlyph(.auto, size: 22).foregroundStyle(Palette.violet).alignmentGuide(.firstTextBaseline) { $0[.bottom] - 3 }
                    Group {
                        if s.judged && !s.floor {
                            Text(ApprovalWords.verdict(s.verdict)).frame(maxWidth: .infinity, alignment: .leading)
                            Text(SuggestionText.percent(p))
                        } else {
                            Text(s.floor ? "Always asked" : "Not scored").frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    .font(RFont.display(26.5))
                    .tracking(RFont.displayTracking(26.5))
                    .foregroundStyle(tint)
                }
                .accessibilityElement(children: .combine)
                .padding(.top, 26)
                if !s.reason.trimmingCharacters(in: .whitespaces).isEmpty {
                    Text(untrusted(s.reason)).font(RFont.sans(14.5)).foregroundStyle(Palette.ink2).lineLimit(3).padding(.top, 6)
                }
                if let risk = s.risk {
                    RiskChips(risk: risk)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, 10)
                }
                if s.judged {
                    VStack(spacing: 0) {
                        LikelihoodRow(label: "Approve", p: s.pApprove, color: Palette.success, glyph: .check)
                        LikelihoodRow(label: "Deny", p: s.pDeny, color: Palette.danger, glyph: .close)
                        LikelihoodRow(label: "Sure", p: s.confidence, color: Palette.violet, glyph: .shieldCheck)
                    }
                    .padding(.top, 14)
                }
                ForEach(SuggestionText.notes(s), id: \.self) { note in
                    Text(note)
                        .font(RFont.sans(13.5))
                        .foregroundStyle(Palette.ink2)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 10)
                }
            }
            .padding(.horizontal, 20)
            VStack(alignment: .leading, spacing: 8) {
                if !s.neighbours.isEmpty {
                    FactHeader("Similar", glyph: .activity).padding(.top, 12)
                    FactGroup {
                        ForEach(Array(s.neighbours.prefix(4).enumerated()), id: \.offset) { i, n in
                            NeighbourRow(neighbour: n).accessibilityIdentifier("neighbour:\(i)")
                        }
                    }
                }
                FactGroup {
                    Button(action: onSettings) {
                        TileRow("Autopilot") {
                            ModeGlyph(.auto, size: 17)
                                .foregroundStyle(Palette.violet)
                                .frame(width: 32, height: 32)
                                .background(Palette.violet.opacity(0.14), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                        } trailing: { EmptyView() }
                    }
                    .buttonStyle(RowButtonStyle())
                    .accessibilityIdentifier("autopilotSettings")
                }
                .padding(.top, 4)
            }
            .padding(.top, 6)
            .padding(.bottom, 20)
        }
        .background(Palette.paperRaised.ignoresSafeArea())
        .presentationBackground(Palette.paperRaised)
        .presentationDetents([.fraction(0.6), .large])
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
        if s.floor { return Palette.ink2 }
        if !s.judged { return Palette.inkMeta }
        return verdictTint(s.verdict)
    }

    static func verdictTint(_ v: Verdict) -> Color {
        switch v {
        case .approve: Palette.success
        case .deny: Palette.danger
        case .ask: Palette.kindRead
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

/// "✓ Approve  ███████░  97%": one labelled likelihood, saying what it measures (the kit's bar: 32 pt, an 8 pt track).
struct LikelihoodRow: View {
    var label: String
    var p: Float
    var color: Color
    var glyph: Glyph? = nil

    var body: some View {
        let value = Double(min(max(p, 0), 1))
        HStack(spacing: 10) {
            HStack(spacing: 6) {
                if let glyph { GlyphIcon(glyph, size: 16).foregroundStyle(color) }
                Text(label).font(RFont.sans(14, .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
            }
            .frame(width: 92, alignment: .leading)
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(Palette.paperSunken)
                    Capsule().fill(color).frame(width: geo.size.width * max(value, 0.02))
                }
            }
            .frame(height: 8)
            Text(SuggestionText.percent(p)).font(RFont.mono(13)).foregroundStyle(Palette.ink).frame(width: 40, alignment: .trailing)
        }
        .frame(minHeight: 32)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(label) \(SuggestionText.percent(p))")
    }
}

/// A past decision like this one: how it ended, what it was, how alike.
struct NeighbourRow: View {
    var neighbour: NeighbourView

    var body: some View {
        let tint = SuggestionText.verdictTint(neighbour.verdict)
        HStack(spacing: 12) {
            GlyphIcon(neighbour.verdict == .approve ? .check : neighbour.verdict == .deny ? .close : .help, size: 17)
                .foregroundStyle(tint)
            Text(untrusted(neighbour.label)).font(RFont.mono(14)).foregroundStyle(Palette.ink).lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            Text(SuggestionText.percent(neighbour.similarity)).font(RFont.mono(13)).foregroundStyle(Palette.inkMeta)
        }
        .padding(.horizontal, 14)
        .frame(minHeight: 46)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(SuggestionText.pastVerdict(neighbour.verdict)): \(untrusted(neighbour.label)), \(SuggestionText.percent(neighbour.similarity)) alike, \(TimeText.relative(neighbour.at))")
    }
}
