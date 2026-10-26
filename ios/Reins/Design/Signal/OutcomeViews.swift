import SwiftUI

/// How a decided request went, at the end of its Activity row (the System board, Outcomes): the universal ones are a
/// glyph (your check, your cross); the rest carry one word on a tint (Auto, Grant, Bypass, Locked, Night, Failed).
enum Outcome: Equatable {
    case approved, denied
    case autoApproved, autoDenied
    case grant, bypass, locked, night, failed
    case none

    /// What VoiceOver reads for it.
    var spoken: String {
        switch self {
        case .approved: "You approved"
        case .denied: "You denied"
        case .autoApproved: "Autopilot approved"
        case .autoDenied: "Autopilot denied"
        case .grant: "A grant allowed it"
        case .bypass: "Allowed in a bypass"
        case .locked: "Denied in lockdown"
        case .night: "Held overnight"
        case .failed: "Approved, then failed"
        case .none: ""
        }
    }
}

struct OutcomeMark: View {
    var outcome: Outcome

    init(_ outcome: Outcome) { self.outcome = outcome }

    var body: some View {
        Group {
            switch outcome {
            case .approved: GlyphIcon(.check, size: 17).foregroundStyle(Palette.success)
            case .denied: GlyphIcon(.close, size: 16).foregroundStyle(Palette.danger)
            case .autoApproved: Pill("Auto", tone: .violet, mode: .auto)
            case .autoDenied: Pill("Auto", tone: .danger, mode: .auto)
            case .grant: Pill("Grant", tone: .violet, glyph: .key)
            case .bypass: Pill("Bypass", tone: .danger, glyph: .shieldAlert)
            case .locked: Pill("Locked", tone: .warning, glyph: .block)
            case .night: Pill("Night", tone: .night, glyph: .moon)
            case .failed: Pill("Failed", tone: .warning, glyph: .circleAlert)
            case .none: EmptyView()
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(outcome.spoken)
    }
}

/// An Activity row (the kit's `arow`): the request's icon, one sentence (who and what, the target in 600), a short
/// amber reason when it was risky, then the time and the outcome at the right.
struct ActivityRowView<Icon: View>: View {
    var runs: [AskText.Run]
    var time: String
    var outcome: Outcome
    var risk: String?
    var terms: [String] = []
    @ViewBuilder var icon: Icon

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            icon
            VStack(alignment: .leading, spacing: 3) {
                AskRunsText(runs: runs, size: TextRole.body.size, terms: terms)
                    .foregroundStyle(Palette.ink)
                    .lineSpacing(3)
                    .lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if let risk {
                    HStack(spacing: 5) {
                        GlyphIcon(.warning, size: 13)
                        Text(risk).font(RFont.sans(12.5, .medium)).lineLimit(1)
                    }
                    .foregroundStyle(Palette.warning)
                }
            }
            VStack(alignment: .trailing, spacing: 4) {
                Text(time).textRole(.meta).foregroundStyle(Palette.inkMeta).monospacedDigit().lineLimit(1)
                OutcomeMark(outcome)
            }
            .frame(minWidth: 34, alignment: .trailing)
        }
        .padding(.vertical, 9)
        .padding(.horizontal, 14)
        .frame(minHeight: 56)
        .accessibilityElement(children: .combine)
    }
}
