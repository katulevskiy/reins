import SwiftUI

/// An entry, decided (iOS-Activity.png, 05): decided means paper. Its kind tile and the label line with when, the line
/// in the serif with its target in italic (in the past when it was done), and the verdict, in violet when Autopilot
/// gave it. The tile is the one the field folded into.
struct DecidedHeader: View {
    var entry: ActivityEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                Group {
                    if let field = EntryLook.field(entry) {
                        KindTile(field, glyph: EntryLook.glyph(entry))
                    } else {
                        GlyphIcon(EntryLook.glyph(entry), size: 20)
                            .foregroundStyle(Palette.ink2)
                            .frame(width: 38, height: 38)
                            .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                    }
                }
                .accessibilityHidden(true)
                Text(Self.labelLine(entry))
                    .textRole(.label)
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            AskText(Self.runs(entry), role: .sheetTitle)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("detailTitle")
            // An entry without a label says what it was about under its line.
            let detail = untrusted(entry.detail).trimmingCharacters(in: .whitespacesAndNewlines)
            if entry.label == nil, !detail.isEmpty {
                Text(detail).textRole(.body).foregroundStyle(Palette.ink2).textSelection(.enabled).accessibilityIdentifier("detailSummary")
            }
            HStack(spacing: 6) {
                if !entry.decidedBy.isEmpty { GlyphIcon(.autopilot, size: 16) }
                Text(EntryOutcome.text(entry)).font(RFont.sans(14.5, .medium))
            }
            .foregroundStyle(Self.verdictTint(entry))
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("entryOutcome")
            if let risk = entry.autopilot?.risk { RiskChips(risk: risk) }
        }
        .padding(.horizontal, 24)
        .padding(.top, 8)
        .padding(.bottom, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("entryHero")
    }

    /// "GITHUB · PUSH · TODAY 14:32" (set in capitals).
    static func labelLine(_ entry: ActivityEntry) -> String {
        let service = entry.label?.provider ?? entry.service
        let action = entry.label?.action ?? (entry.op.isEmpty ? entry.action : entry.op)
        let parts = [service.isEmpty ? "" : serviceName(service), action.replacingOccurrences(of: "_", with: " "),
                     AskedText.dayTime(entry.at).replacingOccurrences(of: ",", with: "")]
        return parts.filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// The line as an ask: the label's lead, its subject in italic, its tail; else the operation's title.
    static func runs(_ entry: ActivityEntry) -> [AskText.Run] {
        guard let label = entry.label else { return [AskText.Run(text: EntryText.title(entry))] }
        let done = ActivityText.done(entry)
        let lead = untrusted(label.lead)
        let subject = untrusted(label.subject)
        let tail = untrustedLine(label.tail)
        let said = done ? ActivityText.past(lead) : lead
        var runs: [AskText.Run] = []
        if !said.isEmpty { runs.append(.init(text: subject.isEmpty ? said : said + " ")) }
        if !subject.isEmpty { runs.append(.init(text: subject, target: true)) }
        if !tail.isEmpty { runs.append(.init(text: tail)) }
        return runs
    }

    /// Autopilot's verdicts in violet; yours in ink-2; denials, failures and Lockdown in their colour.
    static func verdictTint(_ entry: ActivityEntry) -> Color {
        switch EntryText.mark(entry) {
        case .auto, .overnight, .grant: Palette.violet
        case .denied, .bypass: Palette.danger
        case .failed, .lockdown: Palette.warning
        case .approved: Palette.ink2
        }
    }
}
