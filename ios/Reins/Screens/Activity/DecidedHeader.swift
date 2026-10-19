import SwiftUI

/// An entry, decided (iOS-Activity.png, 05): decided means paper. Its kind tile and the label line with when, the line
/// in the serif with its target in italic (in the past when it was done), and the verdict, in violet when Autopilot
/// gave it. The tile is the one the field folded into.
struct DecidedHeader: View {
    var entry: ActivityEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            let line = EntryLook.line(entry)
            HStack(spacing: 12) {
                RequestIcon(spec: line.icon, size: 40)
                    .accessibilityHidden(true)
                    .accessibilityIdentifier("entryIcon")
                Text(Self.labelLine(entry))
                    .textRole(.label)
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            AskText(line.runs, role: .sheetTitle, lineLimit: nil)
                .foregroundStyle(Palette.ink)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(line.text)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("detailTitle")
            if entry.label?.command == nil, let mono = line.mono {
                Text(verbatim: mono)
                    .font(TextRole.value.font())
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

    /// "GITHUB · PUSH · TODAY 14:32" (set in capitals): the core's kind, and when.
    static func labelLine(_ entry: ActivityEntry) -> String {
        "\(EntryLook.line(entry).kind) · \(AskedText.dayTime(entry.at).replacingOccurrences(of: ",", with: ""))"
    }

    /// The line as an ask (ASK-TEXT): told as it ended, its target in italic.
    static func runs(_ entry: ActivityEntry) -> [AskText.Run] { EntryLook.line(entry).runs }

    /// Autopilot's verdicts in violet; yours in ink-2; denials, failures and Lockdown in their colour.
    static func verdictTint(_ entry: ActivityEntry) -> Color {
        switch EntryText.mark(entry) {
        case .auto, .overnight, .grant: Palette.violet
        case .denied, .bypass: Palette.danger
        case .failed, .lockdown, .started, .unknown, .withheld: Palette.warning
        case .approved: Palette.ink2
        }
    }
}
