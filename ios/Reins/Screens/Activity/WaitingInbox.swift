import SwiftUI

// "Waiting for you" (decisions v2, 03 §5.2): requests that became tickets. Nobody waits on them, so they are not cards
// in the stack but rows above the river, the soonest due first, grouped by who asks. Each opens the approval sheet;
// several of one kind from one AI can be answered together.

/// The waiting list split into Instant cards and tickets, the tickets grouped and batched (pure, unit-tested).
enum TicketInbox {
    /// The most a note or a question carries (the core's limit).
    static let maxText = 500

    /// Several tickets of one kind from one AI ("all 4 emails to Acme"): `allow` are those that may be approved
    /// together, `all` those that may be denied together.
    struct Batch: Identifiable, Equatable {
        var connectionId: String
        /// Who asks, its own name.
        var label: String
        /// "Gmail · Send": what they are.
        var kind: String
        var key: String
        var all: [String]
        var allow: [String]

        var id: String { "\(connectionId)|\(key)" }
    }

    /// One AI's tickets, the soonest due first, and its batches.
    struct Group: Identifiable, Equatable {
        var connectionId: String
        var label: String
        var items: [PendingItem]
        var batches: [Batch]

        var id: String { connectionId }
    }

    /// What the stack shows: everything that is not a ticket, in the core's order.
    static func instant(_ pending: [PendingItem]) -> [PendingItem] { pending.filter { $0.ticket == nil } }

    /// The tickets, the soonest due first (then the oldest, then by id, so the order never jumps).
    static func tickets(_ pending: [PendingItem]) -> [PendingItem] {
        pending.compactMap { item in item.ticket.map { (item, $0.deadline) } }
            .sorted { a, b in
                if a.1 != b.1 { return a.1 < b.1 }
                if a.0.createdAt != b.0.createdAt { return a.0.createdAt < b.0.createdAt }
                return a.0.id < b.0.id
            }
            .map(\.0)
    }

    /// The tickets by who asks (one connection each, so two AIs of the same name never mix), the group with the soonest
    /// deadline first.
    static func groups(_ pending: [PendingItem]) -> [Group] {
        var order: [String] = []
        var by: [String: [PendingItem]] = [:]
        for item in tickets(pending) {
            if by[item.connectionId] == nil { order.append(item.connectionId) }
            by[item.connectionId, default: []].append(item)
        }
        return order.compactMap { id in
            guard let items = by[id], let first = items.first else { return nil }
            return Group(connectionId: id, label: first.connectionLabel, items: items, batches: batches(items))
        }
    }

    /// What makes tickets the same kind: the service, the action and the operation.
    static func kindKey(_ item: PendingItem) -> String { "\(item.service)|\(item.action)|\(item.op)" }

    /// One AI's tickets of the same kind, when there are at least two.
    static func batches(_ items: [PendingItem]) -> [Batch] {
        var order: [String] = []
        var by: [String: [PendingItem]] = [:]
        for item in items where item.kind == .request {
            let key = "\(item.connectionId)|\(kindKey(item))"
            if by[key] == nil { order.append(key) }
            by[key, default: []].append(item)
        }
        return order.compactMap { key in
            guard let same = by[key], same.count >= 2, let first = same.first else { return nil }
            return Batch(
                connectionId: first.connectionId, label: first.connectionLabel, kind: ActivityText.label(first), key: kindKey(first),
                all: same.map(\.id), allow: same.filter(allowsBatchApproval).map(\.id)
            )
        }
    }

    /// May be approved with others, without opening it: routine (`quick`, never what is asked every time; P4's stand-in
    /// for "never across L3"), not stale (what changed needs a look) and not waiting for the agent's answer to a Why?
    /// (the reviewer asked to know more first).
    static func allowsBatchApproval(_ item: PendingItem) -> Bool {
        guard item.quick, item.kind == .request else { return false }
        guard let t = item.ticket else { return true }
        return t.stale == nil && !["stale", "approved", "needs_info"].contains(t.state)
    }

    /// "Allow all 3", or "Allow 2" when only some may be approved together.
    static func allowTitle(_ b: Batch) -> String { b.allow.count == b.all.count ? "Allow all \(b.all.count)" : "Allow \(b.allow.count)" }

    static func denyTitle(_ b: Batch) -> String { "Deny all \(b.all.count)" }

    /// "Due in 3 h", "Overdue"; nil for what is not a ticket.
    static func due(_ item: PendingItem, now: Int64) -> String? { item.ticket.map { TicketText.due(deadline: $0.deadline, now: now) } }

    /// A note to the agent as typed: trimmed, at most `maxText` characters; nil when empty (a plain denial).
    static func note(_ text: String?) -> String? {
        let t = (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return t.isEmpty ? nil : String(t.prefix(maxText))
    }

    /// A Why? as typed; nil when empty (nothing to ask).
    static func question(_ text: String) -> String? { note(text) }

    /// Later is offered on what the server can park and is not parked yet.
    static func canLater(_ item: PendingItem) -> Bool { item.kind == .request && item.canDefer && item.ticket == nil }
}

/// "Waiting for you": one raised group above the river, its title row, then each AI's name and its tickets, and the
/// batch buttons under them. On a wallpaper the title is inside the pane, never on the art.
struct WaitingInbox: View {
    var groups: [TicketInbox.Group]

    private enum Line: Identifiable {
        case title(Int)
        case who(TicketInbox.Group)
        case ticket(PendingItem)
        case batch(TicketInbox.Batch)

        var id: String {
            switch self {
            case .title: "title"
            case let .who(g): "who:\(g.id)"
            case let .ticket(item): "ticket:\(item.id)"
            case let .batch(b): "batch:\(b.id)"
            }
        }

        /// The hairline above the row at `index`, and where it starts: after the icon column between one AI's rows,
        /// across the pane before the next AI.
        func hairline(at index: Int) -> (shown: Bool, inset: CGFloat) {
            switch self {
            case .title: (false, 62)
            case .who: (index > 1, 14)
            case .ticket: (index > 0, 62)
            case .batch: (true, 62)
            }
        }
    }

    private var lines: [Line] {
        var out: [Line] = [.title(groups.reduce(0) { $0 + $1.items.count })]
        for g in groups {
            out.append(.who(g))
            out += g.items.map(Line.ticket)
            out += g.batches.map(Line.batch)
        }
        return out
    }

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            let now = Int64(context.date.timeIntervalSince1970)
            let lines = self.lines
            VStack(spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.element.id) { i, line in
                    // The first ticket sits right under its AI's name, without a line between them.
                    let rule = line.hairline(at: i)
                    let underName: Bool = if case .who = lines[max(i - 1, 0)] { true } else { false }
                    row(line, now: now)
                        .modifier(GroupSlice(first: i == 0, last: i == lines.count - 1, hairline: rule.shown && !underName, inset: rule.inset))
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("waitingInbox")
    }

    @ViewBuilder private func row(_ line: Line, now: Int64) -> some View {
        switch line {
        case let .title(count):
            HStack(spacing: 6) {
                Text("Waiting for you")
                    .font(RFont.sans(13, .semibold))
                    .tracking(13 * TextRole.section.trackingEm)
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(1)
                Spacer(minLength: 8)
                Text("\(count)")
                    .font(RFont.sans(13, .semibold))
                    .monospacedDigit()
                    .foregroundStyle(Palette.ink2)
            }
            .padding(.horizontal, 14)
            .padding(.top, 12)
            .padding(.bottom, 4)
            .frame(minHeight: 40)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Waiting for you, \(count)")
            .accessibilityAddTraits(.isHeader)
        case let .who(group):
            HStack(spacing: 8) {
                ConnectionIcon(connectionId: group.connectionId, label: group.label, size: 22)
                Text(verbatim: untrustedName(group.label, limit: WaitCardIdentity.limit))
                    .font(RFont.sans(14, .semibold))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
            }
            .padding(.horizontal, 14)
            .padding(.top, 8)
            .padding(.bottom, 2)
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            .accessibilityIdentifier("waitingWho:\(group.connectionId)")
        case let .ticket(item):
            TicketRow(item: item, now: now)
        case let .batch(batch):
            BatchRow(batch: batch)
        }
    }
}

/// A ticket as a row: the request as its waiting card says it, when it is due, and what happened to it since (a Why?,
/// the agent's answer, what changed). A tap opens the approval sheet.
private struct TicketRow: View {
    var item: PendingItem
    var now: Int64
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        let line = AskLabels.pending(item, field: ActivityText.field(item, danger: false))
        Button {
            guard !SidewaysPan.swiping else { return }
            model.openSheet(item.sheetTarget)
            feedback.defaultTap()
        } label: {
            HStack(alignment: .top, spacing: 12) {
                RequestIcon(spec: line.icon, size: 36, ring: Palette.paperRaised)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 4) {
                    AskRunsText(runs: line.runs, size: TextRole.body.size)
                        .foregroundStyle(Palette.ink)
                        .lineLimit(3)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if let mono = line.mono ?? ActivityText.detail(item) {
                        Text(verbatim: mono)
                            .font(RFont.mono(12.5))
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(2)
                            .environment(\.layoutDirection, .leftToRight)
                    }
                    if let ticket = item.ticket {
                        HStack(spacing: 6) {
                            DueText(deadline: ticket.deadline, now: now)
                                .accessibilityIdentifier("ticketDue:\(item.id)")
                            if let state = TicketText.state(ticket) {
                                Pill(state, tone: state == "Stale" ? .warning : .violet)
                            }
                        }
                        TicketNotes(ticket: ticket, id: item.id, compact: true)
                    }
                }
            }
            .padding(.vertical, 10)
            .padding(.horizontal, 14)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowButtonStyle())
        .contextMenu {
            Button("Open", systemImage: "arrow.up.right") {
                model.openSheet(item.sheetTarget)
                feedback.defaultTap()
            }
            Button("Deny", systemImage: "nosign", role: .destructive) {
                Task { await Burst.answer(model, ids: [item.id], label: item.connectionLabel, approve: false) }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("ticket:\(item.id)")
    }
}

/// "Due in 3 h" in the meta colour; under an hour in amber, past the deadline in red.
struct DueText: View {
    var deadline: Int64
    var now: Int64

    var body: some View {
        let overdue = deadline <= now
        let urgent = TicketText.urgent(deadline: deadline, now: now)
        HStack(spacing: 4) {
            Image(systemName: overdue ? "clock.badge.exclamationmark" : "clock")
                .font(.system(size: 11.5, weight: .semibold))
            Text(TicketText.due(deadline: deadline, now: now)).monospacedDigit()
        }
        .font(RFont.sans(12.5, .semibold))
        .foregroundStyle(overdue ? Palette.danger : urgent ? Palette.warning : Palette.inkMeta)
        .lineLimit(1)
    }
}

/// What happened to a ticket since it was parked: the reviewer's Why?, the agent's answer (its own words, set apart
/// and labelled as the agent's, never a fact Reins states), what changed since it was approved.
struct TicketNotes: View {
    var ticket: TicketView
    var id: String
    /// In a row: a line or two each.
    var compact = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let stale = TicketText.stale(ticket) {
                WarnTag(text: stale, tint: Palette.warning, symbol: "arrow.triangle.2.circlepath", lines: compact ? 2 : 4)
                    .accessibilityLabel("Stale: \(stale)")
                    .accessibilityIdentifier("ticketStale:\(id)")
            }
            if let question = ticket.question.map(untrusted), !question.isEmpty {
                HStack(alignment: .top, spacing: 5) {
                    Image(systemName: "questionmark.bubble").font(.system(size: 12, weight: .semibold)).padding(.top, 1.5)
                    Text(verbatim: question).lineLimit(compact ? 2 : 6)
                }
                .font(RFont.sans(13))
                .foregroundStyle(Palette.ink2)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("You asked: \(question)")
                .accessibilityIdentifier("ticketQuestion:\(id)")
            }
            if let says = TicketText.agentSays(ticket) {
                AgentSays(text: says, lines: compact ? 3 : 12)
                    .accessibilityIdentifier("agentSays:\(id)")
            }
        }
    }
}

/// The agent's answer to a Why?: "Agent says" over its words in italics, on the sunken paper with a rule at the side,
/// so it never reads as Reins' own.
private struct AgentSays: View {
    var text: String
    var lines: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Label {
                Text(TicketText.agentSaysLabel)
            } icon: {
                Image(systemName: "quote.bubble").font(.system(size: 11, weight: .semibold))
            }
            .font(RFont.sans(11.5, .semibold))
            .foregroundStyle(Palette.inkMeta)
            Text(verbatim: text)
                .font(RFont.sans(13.5))
                .italic()
                .foregroundStyle(Palette.ink2)
                .lineLimit(lines)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.leading, 10)
        .padding(.vertical, 6)
        .padding(.trailing, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(alignment: .leading) {
            Capsule().fill(Palette.inkMeta.opacity(0.6)).frame(width: 2.5).padding(.vertical, 6)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(TicketText.agentSaysLabel): \(text)")
    }
}

/// Several of one kind from one AI: Allow (only what may be approved together, one check for the lot) and Deny.
private struct BatchRow: View {
    var batch: TicketInbox.Batch
    @Environment(AppModel.self) private var model
    @State private var busy = false

    /// One-tap approval needs who asks in sight, as on a waiting card.
    private var allows: Bool { !batch.allow.isEmpty && !untrustedName(batch.label, limit: WaitCardIdentity.limit).isEmpty }

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                kind
                Spacer(minLength: 4)
                buttons
            }
            VStack(alignment: .leading, spacing: 8) {
                kind
                HStack(spacing: 8) { buttons }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .disabled(busy)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("batch:\(batch.id)")
    }

    private var kind: some View {
        Text(batch.kind)
            .font(RFont.mono(12, .medium))
            .textCase(.uppercase)
            .foregroundStyle(Palette.inkMeta)
            .lineLimit(1)
    }

    @ViewBuilder private var buttons: some View {
        if allows {
            Button {
                answer(batch.allow, approve: true)
            } label: {
                Label { Text(TicketInbox.allowTitle(batch)) } icon: { BiometryMark(size: 15) }
            }
            .buttonStyle(SignalButtonStyle(.inverse, size: .small, fill: false))
            .fixedSize()
            .accessibilityIdentifier("batchAllow:\(batch.id)")
        }
        Button(TicketInbox.denyTitle(batch)) { answer(batch.all, approve: false) }
            .buttonStyle(SignalButtonStyle(.outline, size: .small, fill: false))
            .fixedSize()
            .accessibilityIdentifier("batchDeny:\(batch.id)")
    }

    private func answer(_ ids: [String], approve: Bool) {
        busy = true
        Task {
            await Burst.answer(model, ids: ids, label: batch.label, approve: approve)
            busy = false
        }
    }
}
