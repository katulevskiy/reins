import SwiftUI

/// How a decided request went, at the end of its Activity row (the System board, Outcomes): the universal ones are a
/// glyph (your check, your cross); the rest carry one word on a tint (Auto, Grant, Bypass, Locked, Night, Failed).
enum Outcome: Equatable {
    case approved, denied
    case autoApproved, autoDenied
    case grant, bypass, locked, night, failed
    /// Carried out, but how it ended needs a look ("Waiting", "Unknown", "Withheld"): one word on amber.
    case attention(word: String, spoken: String)
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
        case let .attention(_, spoken): spoken
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
            case let .attention(word, _): Pill(word, tone: .warning, glyph: .circleAlert)
            case .none: EmptyView()
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(outcome.spoken)
    }
}

/// An Activity row (the kit's `arow`): the request's icon, one sentence (who and what, the target in 600), a short
/// amber reason when it was risky, then the time and the outcome at the right. `who` (an agent's or connection's own
/// name) leads the sentence as its own run in its own style, never merged into Reins' words; `extra` goes under the
/// sentence (a search match). At accessibility text sizes the time and outcome move under the sentence.
struct ActivityRowView<Icon: View, Extra: View>: View {
    var runs: [AskText.Run]
    var who: String? = nil
    var time: String
    var outcome: Outcome
    var risk: String?
    var terms: [String] = []
    @ViewBuilder var icon: Icon
    @ViewBuilder var extra: Extra
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let ax = typeSize.isAccessibilitySize
        HStack(alignment: ax ? .top : .center, spacing: 12) {
            icon
            VStack(alignment: .leading, spacing: 3) {
                WhoRunsText(who: who, runs: runs, size: TextRole.body.size, terms: terms)
                    .foregroundStyle(Palette.ink)
                    .lineSpacing(3)
                    .lineLimit(ax ? nil : 3)
                    .fixedSize(horizontal: false, vertical: ax)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if let risk {
                    HStack(spacing: 5) {
                        GlyphIcon(.warning, size: 13)
                        Text(risk).font(RFont.sans(12.5, .medium)).lineLimit(ax ? 3 : 1)
                    }
                    .foregroundStyle(Palette.warning)
                }
                extra
                if ax {
                    HStack(spacing: 8) {
                        Text(time).textRole(.meta).foregroundStyle(Palette.inkMeta).monospacedDigit()
                        OutcomeMark(outcome)
                    }
                    .padding(.top, 2)
                }
            }
            if !ax {
                VStack(alignment: .trailing, spacing: 4) {
                    Text(time).textRole(.meta).foregroundStyle(Palette.inkMeta).monospacedDigit().lineLimit(1)
                    OutcomeMark(outcome)
                }
                .frame(minWidth: 34, alignment: .trailing)
            }
        }
        .padding(.vertical, 9)
        .padding(.horizontal, 14)
        .frame(minHeight: 56)
        .accessibilityElement(children: .combine)
    }
}

extension ActivityRowView where Extra == EmptyView {
    init(runs: [AskText.Run], who: String? = nil, time: String, outcome: Outcome, risk: String? = nil, terms: [String] = [], @ViewBuilder icon: () -> Icon) {
        self.init(runs: runs, who: who, time: time, outcome: outcome, risk: risk, terms: terms, icon: icon, extra: { EmptyView() })
    }
}

/// A sentence that may start with who did it: the name in 500 and ink-2, a run of its own (it is the other side's
/// words, so it never reads as part of Reins' sentence), then the sentence's runs (`AskRunsText`), matches set apart.
struct WhoRunsText: View {
    var who: String?
    var runs: [AskText.Run]
    var size: CGFloat
    var terms: [String] = []

    var body: some View {
        Text(Self.attributed(who: who, runs: runs, size: size, terms: terms))
    }

    static func attributed(who: String?, runs: [AskText.Run], size: CGFloat, terms: [String]) -> AttributedString {
        var out = AttributedString()
        if let who, !who.isEmpty {
            var name = AttributedString(who)
            name.font = RFont.sans(size, .medium)
            name.foregroundColor = Palette.ink2
            out += name
            out += AttributedString(" ")
        }
        out += AskRunsText.attributed(runs, size: size, terms: [])
        if !terms.isEmpty { TitleHighlight.apply(&out, terms: terms) }
        return out
    }
}
