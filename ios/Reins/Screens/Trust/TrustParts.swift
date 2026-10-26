import SwiftUI

// Trust's pieces (the Trust and Running boards): a raised group whose rows are separate list rows (so each grant can
// be swiped), the grant row with its service plate, agent avatar, use dots and time-left pie, a section header with
// its +, and the running mode's stacked list.

/// Where a list row sits in a raised group: it draws its slice of the group's rounded plate, and the hairline above it.
enum SlicePosition {
    case only, first, middle, last

    static func of(_ index: Int, count: Int) -> SlicePosition {
        if count <= 1 { return .only }
        if index == 0 { return .first }
        return index == count - 1 ? .last : .middle
    }

    var top: Bool { self == .only || self == .first }
    var bottom: Bool { self == .only || self == .last }
}

private struct TrustSlice: ViewModifier {
    var position: SlicePosition
    var inset: CGFloat
    @Environment(\.colorScheme) private var scheme

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .background {
                // The whole group's plate, run past this row's ends where it goes on, then cut to the row.
                ZStack {
                    shape.fill(Palette.paperRaised)
                    if scheme == .light { shape.strokeBorder(Palette.rule, lineWidth: 1) }
                }
                .padding(.top, position.top ? 0 : -40)
                .padding(.bottom, position.bottom ? 0 : -40)
            }
            .overlay(alignment: .top) {
                if !position.top { Rectangle().fill(Palette.rule).frame(height: 1).padding(.leading, inset) }
            }
            .clipShape(Rectangle())
            .padding(.horizontal, 16)
    }
}

extension View {
    /// This list row as a slice of a raised group (`RaisedGroup`'s look), with a hairline after its tile (`inset`).
    func trustSlice(_ position: SlicePosition, inset: CGFloat = 62) -> some View {
        modifier(TrustSlice(position: position, inset: inset))
    }
}

/// A section header with a count and a round + at the right ("Grants  6  (+)").
struct AddSectionHeader: View {
    var title: String
    var glyph: Glyph
    var count: Int?
    var addLabel: String
    var addID: String
    var onAdd: () -> Void

    var body: some View {
        SectionHeader(title, glyph: glyph) {
            HStack(spacing: 10) {
                if let count { Text("\(count)").foregroundStyle(Palette.ink2) }
                RoundHeaderButton(glyph: .plus, label: addLabel, size: 30, action: onAdd)
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
                    .accessibilityIdentifier(addID)
            }
            .frame(height: 20)
        }
    }
}

/// A grant's service on its plate with the agent's avatar on the bottom-left corner (the boards' `mini`).
struct GrantMini: View {
    var service: String
    var symbol: String? = nil
    var connectionId: String
    var label: String
    var size: CGFloat = 38
    var dim = false

    var body: some View {
        let inset = (size * 0.14).rounded()
        ServiceTile(service: service, symbol: symbol, size: size)
            .opacity(dim ? 0.55 : 1)
            .padding(.leading, inset)
            .padding(.bottom, inset)
            .overlay(alignment: .bottomLeading) {
                ConnectionIcon(connectionId: connectionId, label: label, size: (size / 2).rounded())
                    .padding(2)
                    .background(Palette.paperRaised, in: Circle())
                    .offset(x: -2, y: 2)
            }
            .accessibilityHidden(true)
    }
}

/// A grant's time left: a ring in the accent (red when it ends soon) with the amount in mono inside ("50m").
struct GrantPie: View {
    var clock: GrantClock
    var size: CGFloat = 40

    var body: some View {
        let color = clockColor(clock)
        ZStack {
            ArcRing(fraction: clock.fraction, color: color, track: Palette.paperSunken, lineWidth: size * 0.1)
            Text(clock.label)
                .font(RFont.fixedMono((size * 0.27).rounded(), .semibold))
                .foregroundStyle(color)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .environment(\.layoutDirection, .leftToRight)
        }
        .frame(width: size, height: size)
        .accessibilityElement()
        .accessibilityLabel(clock.remaining.map { "\(compactDuration($0)) left" } ?? "No time limit")
    }
}

/// One bar per use, the used ones in violet, then "3/5"; a thin meter when there are too many to count.
struct GrantUseDots: View {
    var uses: Int
    var maxUses: Int

    var body: some View {
        HStack(spacing: 3) {
            if maxUses <= 12 {
                ForEach(0..<maxUses, id: \.self) { i in
                    Capsule().fill(i < uses ? Palette.violet : Palette.rule2).frame(width: 14, height: 4)
                }
            } else {
                let fraction = min(max(Double(uses) / Double(maxUses), 0), 1)
                Capsule().fill(Palette.rule2).frame(width: 96, height: 4)
                    .overlay(alignment: .leading) { Capsule().fill(Palette.violet).frame(width: 96 * fraction, height: 4) }
            }
            Text("\(uses)/\(maxUses)")
                .textRole(.meta)
                .foregroundStyle(Palette.inkMeta)
                .monospacedDigit()
                .padding(.leading, 5)
        }
        .accessibilityElement()
        .accessibilityLabel("\(uses) of \(maxUses) uses")
    }
}

/// How a grant's summary reads: what it does, then what it is about in 600 ("Read emails from **@bank.com**"). The
/// summary is the core's; the split only sets the part after its last "from", "of", "in" or "to" in the target weight.
enum GrantWords {
    static func runs(_ summary: String) -> [AskText.Run] {
        // One clean line: the summary carries the AI's parties.
        let clean = untrustedName(summary, limit: 300)
        for word in [" from ", " of ", " in ", " to "] {
            if let r = clean.range(of: word, options: .backwards), r.upperBound < clean.endIndex {
                return [AskText.Run(text: String(clean[..<r.upperBound])), AskText.Run(text: String(clean[r.upperBound...]), target: true)]
            }
        }
        return [AskText.Run(text: clean)]
    }
}

/// A running grant on Trust (the boards' `grant_row`): plate and avatar, the summary with its target in 600, the use
/// dots when capped, and the pie.
struct GrantRow: View {
    var grant: GrantView
    var now: Int64

    var body: some View {
        let clock = grantClock(grant, now: now)
        HStack(alignment: .center, spacing: 14) {
            GrantMini(service: grant.service, symbol: ActionKind.of(grant.action).symbol, connectionId: grant.connectionId, label: grant.connectionLabel)
            VStack(alignment: .leading, spacing: 6) {
                AskRunsText(runs: GrantWords.runs(grant.summary), size: TextRole.row.size)
                    .foregroundStyle(Palette.ink)
                    .lineLimit(3)
                    .multilineTextAlignment(.leading)
                if let cap = grant.maxUses, cap > 1 {
                    GrantUseDots(uses: Int(grant.uses), maxUses: Int(cap))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            GrantPie(clock: clock)
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 14)
        .frame(minHeight: 60)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(untrusted(grant.summary)), for \(untrustedName(grant.connectionLabel))")
        .accessibilityHint("Opens the grant")
    }
}

/// A grant row following its clock: every second in its last two minutes, every 10 s before, never for one without an
/// end.
struct LiveGrantRow: View {
    var grant: GrantView
    @State private var close = false

    var body: some View {
        Group {
            if grant.expiresAt == nil {
                GrantRow(grant: grant, now: nowSeconds())
            } else {
                TimelineView(.periodic(from: .now, by: close ? 1 : 10)) { ctx in
                    GrantRow(grant: grant, now: nowSeconds(ctx.date))
                }
            }
        }
        .task(id: grant.expiresAt) {
            guard let end = grant.expiresAt else { return }
            let wait = end - 120 - nowSeconds()
            close = wait <= 0
            guard wait > 0 else { return }
            try? await Task.sleep(for: .seconds(wait))
            if !Task.isCancelled { close = true }
        }
    }
}

/// An entry a running mode decided, as Activity shows it (the kit's `arow`), ending in the mode's tag.
struct RunningEntryRow: View {
    var entry: ActivityEntry
    var outcome: Outcome
    var now: Int64 = nowSeconds()
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let line = EntryLook.line(entry, who: entry.connectionLabel)
        if typeSize.isAccessibilitySize {
            // Largest text: the time and the outcome move under the sentence.
            HStack(alignment: .top, spacing: 12) {
                RequestIcon(spec: line.icon, size: 36, ring: Palette.paperRaised)
                VStack(alignment: .leading, spacing: 6) {
                    AskRunsText(runs: line.runs, size: TextRole.body.size)
                        .foregroundStyle(Palette.ink)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 8) {
                        Text(Self.time(entry.at, now: now)).textRole(.meta).foregroundStyle(Palette.inkMeta)
                        OutcomeMark(outcome)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.vertical, 12)
            .padding(.horizontal, 14)
            .accessibilityElement(children: .combine)
        } else {
            ActivityRowView(runs: line.runs, time: Self.time(entry.at, now: now), outcome: outcome) {
                RequestIcon(spec: line.icon, size: 36, ring: Palette.paperRaised)
            }
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// "now", "10m", "2h", "3d".
    static func time(_ at: Int64, now: Int64) -> String {
        let d = now - at
        return d < 60 ? "now" : compactDuration(d)
    }
}

/// What a running mode decided, as a stack (the Running board, C): the newest row on top with the next ones peeking
/// out under it and "+3"; tapping the edges or "+3" opens the list in place, tapping the top row opens that entry.
struct RunningStack: View {
    var running: TrustRunning
    var entries: [ActivityEntry]
    @Binding var open: Bool
    var onOpenEntry: (ActivityEntry) -> Void

    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var tint: Color { running == .lockdown ? Palette.warning : Palette.danger }

    var body: some View {
        VStack(spacing: 0) {
            SectionHeader(running.listTitle, glyph: running.kind.glyph) {
                HStack(spacing: 6) {
                    Text("\(entries.count)").foregroundStyle(tint)
                    if open && entries.count > 1 {
                        Button { toggle() } label: {
                            GlyphIcon(.chevronUp, size: 13).foregroundStyle(Palette.inkMeta)
                                .frame(width: 32, height: 32)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Show less")
                        .accessibilityIdentifier("runningShowLess")
                        .frame(height: 20)
                    }
                }
            }
            if open {
                RaisedGroup(inset: 62) {
                    ForEach(entries, id: \.id) { entry in row(entry) }
                }
            } else if let first = entries.first {
                folded(first)
            }
        }
    }

    private func row(_ entry: ActivityEntry) -> some View {
        Button { onOpenEntry(entry) } label: {
            RunningEntryRow(entry: entry, outcome: running.outcome)
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityIdentifier("runningEntry:\(entry.id)")
    }

    private func folded(_ first: ActivityEntry) -> some View {
        let peeks = RunningListFold.peeks(entries.count)
        return ZStack(alignment: .bottomTrailing) {
            // The cards under the top one: tapping them opens the stack.
            ForEach(Array((0..<peeks).reversed()), id: \.self) { i in
                UnevenRoundedRectangle(bottomLeadingRadius: 18, bottomTrailingRadius: 18, style: .continuous)
                    .fill(Palette.paperRaised)
                    .overlay(UnevenRoundedRectangle(bottomLeadingRadius: 18, bottomTrailingRadius: 18, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
                    .frame(height: 30)
                    .padding(.horizontal, CGFloat(8 * i + 8))
                    .opacity(1 - 0.25 * Double(i))
                    .offset(y: CGFloat(7 * i + 7))
                    .frame(maxHeight: .infinity, alignment: .bottom)
                    .onTapGesture { toggle() }
                    .accessibilityHidden(true)
            }
            RaisedGroup { row(first) }
                .padding(.horizontal, -16)
            if let hidden = RunningListFold.hidden(entries.count) {
                Button { toggle() } label: {
                    Text("+\(hidden)")
                        .font(RFont.fixedSans(12, .semibold))
                        .monospacedDigit()
                        .foregroundStyle(Palette.onInverse)
                        .padding(.horizontal, 7)
                        .frame(minWidth: 22, minHeight: 22)
                        .background(Palette.inverse, in: Capsule())
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(PressDimsStyle())
                .offset(x: -10, y: 33)
                .accessibilityLabel(RunningListFold.showAll(entries.count) ?? "")
                .accessibilityIdentifier("runningShowAll")
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, peeks > 0 ? 18 : 0)
    }

    private func toggle() {
        feedback.play(.expand(!open))
        withAnimation(reduceMotion ? .easeInOut(duration: Motion.fade) : .spring(duration: 0.4, bounce: 0.22)) { open.toggle() }
    }
}
