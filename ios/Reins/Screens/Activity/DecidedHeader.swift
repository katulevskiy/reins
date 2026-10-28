import SwiftUI

/// An entry, decided (Next-Activity, 06 "What happened"): decided means paper. Its request icon, the sentence in the
/// display face (who did it as a run of its own, then what, told as it ended), the target in mono, the verdict (the
/// outcome's tag and word, Autopilot's certainty), and the risk line. The icon is the one the field folded into.
struct DecidedHeader: View {
    var entry: ActivityEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            let line = EntryLook.line(entry)
            let who = EntryRowText.who(entry)
            RequestIcon(spec: line.icon, size: 48, ring: Palette.paper)
                .accessibilityHidden(true)
                .accessibilityIdentifier("entryIcon")
            Text(Self.sentence(who: who, runs: EntryRowText.runs(entry, who: who)))
                .tracking(-0.02 * Self.size)
                .fixedSize(horizontal: false, vertical: true)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("detailTitle")
            if entry.label?.command == nil, let mono = line.mono {
                Text(verbatim: mono)
                    .font(RFont.mono(13.5))
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .environment(\.layoutDirection, .leftToRight)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("detailDetails")
            }
            // An entry without a label says what it was about under its line.
            let detail = untrusted(entry.detail).trimmingCharacters(in: .whitespacesAndNewlines)
            if entry.label == nil, !detail.isEmpty {
                Text(detail).textRole(.body).foregroundStyle(Palette.ink2).textSelection(.enabled).accessibilityIdentifier("detailSummary")
            }
            verdict
            if let risk = EntryRowText.risk(entry) {
                HStack(spacing: 5) {
                    GlyphIcon(.warning, size: 14)
                    Text(risk).font(RFont.sans(13.5, .medium))
                }
                .foregroundStyle(Palette.warning)
                .padding(.top, -4)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("entryRisk")
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("entryHero")
    }

    /// The display size of the sentence (the board's 36, in Sora).
    static let size: CGFloat = 26.5

    /// The sentence: who, in ink-2 (the other side's words, apart from Reins'), then what, the target in 600.
    static func sentence(who: String?, runs: [AskText.Run]) -> AttributedString {
        var out = AttributedString()
        if let who, !who.isEmpty {
            var name = AttributedString(who + " ")
            name.font = RFont.display(size)
            name.foregroundColor = Palette.ink2
            out += name
        }
        for run in runs {
            var part = AttributedString(run.text)
            part.font = RFont.display(size, target: run.target)
            out += part
        }
        return out
    }

    /// Autopilot's verdicts: its tag, the word and how sure it was; everyone else's as one tinted pill in words.
    @ViewBuilder private var verdict: some View {
        let outcome = EntryRowText.outcome(entry)
        let denied = entry.outcome == "denied"
        Group {
            if outcome == .autoApproved || outcome == .autoDenied {
                HStack(spacing: 8) {
                    OutcomeMark(outcome)
                    Text(denied ? "Denied" : "Allowed")
                        .font(RFont.sans(16, .semibold))
                        .foregroundStyle(denied ? Palette.danger : Palette.success)
                    if let p = entry.autopilot.map({ denied ? $0.pDeny : $0.pApprove }), p > 0 {
                        Text(SuggestionText.percent(p)).font(RFont.mono(13.5)).foregroundStyle(Palette.inkMeta)
                    }
                }
            } else {
                let tint = Self.verdictTint(entry)
                HStack(spacing: 4) {
                    GlyphIcon(Self.verdictGlyph(entry), size: 14)
                    Text(EntryOutcome.text(entry)).lineLimit(2)
                }
                .font(RFont.sans(13.5, .semibold))
                .foregroundStyle(tint)
                .padding(.horizontal, 10)
                .frame(minHeight: 28)
                .background(tint.opacity(0.13), in: Capsule())
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(EntryOutcome.text(entry))
        .accessibilityIdentifier("entryOutcome")
    }

    /// "GITHUB · PUSH · TODAY 14:32": the core's kind, and when (sharing).
    static func labelLine(_ entry: ActivityEntry) -> String {
        "\(EntryLook.line(entry).kind) · \(AskedText.dayTime(entry.at).replacingOccurrences(of: ",", with: ""))"
    }

    /// The line as an ask (ASK-TEXT): told as it ended, its target in 600.
    static func runs(_ entry: ActivityEntry) -> [AskText.Run] { EntryLook.line(entry).runs }

    static func verdictGlyph(_ entry: ActivityEntry) -> Glyph {
        switch EntryText.mark(entry) {
        case .approved: .check
        case .denied: .close
        case .grant, .permission: .key
        case .overnight: .moon
        case .bypass: .shieldAlert
        case .lockdown: .block
        case .auto: .autopilot
        case .failed, .started, .unknown, .withheld, .notSent: .circleAlert
        case .held: .send
        case .undone: .undo
        }
    }

    /// Autopilot's verdicts in violet; yours in green or red; failures and Lockdown in amber.
    static func verdictTint(_ entry: ActivityEntry) -> Color {
        switch EntryText.mark(entry) {
        case .auto, .grant, .permission: Palette.violet
        case .overnight: Palette.nightText
        case .denied, .bypass: Palette.danger
        case .failed, .lockdown, .started, .unknown, .withheld, .notSent: Palette.warning
        case .approved: Palette.success
        case .held: Palette.violet
        case .undone: Palette.ink2
        }
    }
}

/// A fact without a key (the kit's `fact`): its glyph says what it is; the value in 15.5/500, mono for machine values;
/// a chevron when it opens something.
struct GlyphFact: View {
    var glyph: Glyph
    var value: String
    var mono = false
    var tint: Color? = nil
    var glyphTint: Color = Palette.inkMeta
    var action: (() -> Void)? = nil

    var body: some View {
        if let action {
            Button(action: action) { content(chevron: true) }
                .buttonStyle(RowButtonStyle())
        } else {
            content(chevron: false)
        }
    }

    private func content(chevron: Bool) -> some View {
        HStack(spacing: 12) {
            GlyphIcon(glyph, size: 18).foregroundStyle(glyphTint).frame(width: 20)
            Text(value)
                .font(mono ? RFont.mono(14) : RFont.sans(15.5, .medium))
                .foregroundStyle(tint ?? Palette.ink)
                .lineLimit(2)
                .truncationMode(mono ? .middle : .tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            if chevron { RowChevron() }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .frame(minHeight: 50)
        .contentShape(Rectangle())
    }
}
