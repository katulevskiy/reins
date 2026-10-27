import SwiftUI

// Tasks (Decisions v2, 01 §12, 08 R3): a bounded envelope inside which what Auto would ask runs by itself, never past
// its ceiling. An agent proposes one; its card shows the envelope as chips the owner can only take away from, a ceiling
// that can only go down, a length only shorter and a budget only smaller. Approving asks the owner (the core asks it
// itself: a task loosens); Deny is the ordinary deny. Live tasks show in Activity as a chip with Stop; those that ended
// today as a quiet line. Nothing here notifies.

/// Tasks in words (pure, unit-tested).
enum TaskText {
    static let taskWord = "Task"
    static let stop = "Stop"

    /// The ceilings a task may have: 0 undoable, 1 recoverable, 2 reaches people (L3 never).
    static let ceilings: [UInt8] = [0, 1, 2]

    static func ceilingName(_ c: UInt8) -> String {
        switch c {
        case 0: "Undoable"
        case 1: "Recoverable"
        default: "Reaches people"
        }
    }

    /// "Up to: Recoverable".
    static func upTo(_ c: UInt8) -> String { "Up to: \(ceilingName(c))" }

    /// A kind of effect in a word or two ("git_push" → "Push").
    static func operationName(_ op: String) -> String {
        switch op {
        case "file_read": "Read files"
        case "file_write": "Write files"
        case "file_delete": "Delete files"
        case "git_push": "Push"
        case "git_ref_delete": "Delete branches"
        case "message": "Messages"
        case "publish": "Publish"
        case "permission": "Permissions"
        case "money": "Payments"
        case "secret_out", "secret_use": "Secrets"
        case "network": "Network"
        case "infra": "Infrastructure"
        case "exec": "Run commands"
        case "data_read": "Read data"
        case "data_write": "Write data"
        default:
            untrustedName(op.replacingOccurrences(of: "_", with: " "), limit: 30).capitalized
        }
    }

    /// "45 min", "2 h", "1 day".
    static func length(minutes: Int64) -> String {
        if minutes < 60 { return "\(minutes) min" }
        if minutes % (24 * 60) == 0 { let d = minutes / (24 * 60); return d == 1 ? "1 day" : "\(d) days" }
        if minutes % 60 == 0 { return "\(minutes / 60) h" }
        return "\(minutes / 60) h \(minutes % 60) min"
    }

    /// "20 uses", "No limit".
    static func budget(_ b: UInt32?) -> String { b.map { $0 == 1 ? "1 use" : "\($0) uses" } ?? "No limit" }

    /// The task's title as the agent named it: one short clean line.
    static func title(_ t: TaskView) -> String { untrustedName(t.title, limit: 60) }

    /// "Task · Fix CI".
    static func chip(_ t: TaskView) -> String { "\(taskWord) · \(title(t))" }

    /// "Fix CI ended · 7 auto · 1 asked · 2 parked".
    static func endedLine(_ t: TaskView) -> String { "\(title(t)) ended · \(t.auto) auto · \(t.asked) asked · \(t.parked) parked" }

    /// Live tasks (not ended, not past their end), in the core's order.
    static func live(_ tasks: [TaskView], now: Int64) -> [TaskView] { tasks.filter { $0.endedAt == nil && $0.endsAt > now } }

    /// Tasks that ended in the last day, the latest first.
    static func ended(_ tasks: [TaskView], now: Int64) -> [TaskView] {
        tasks.compactMap { t -> (TaskView, Int64)? in
            guard t.endedAt != nil || t.endsAt <= now else { return nil }
            let end = t.endedAt ?? t.endsAt
            return end > now - 86_400 ? (t, end) : nil
        }
        .sorted { $0.1 > $1.1 }
        .map(\.0)
    }

    /// The agent's reason, cleaned; nil when empty. Shown as the agent's, never read for anything.
    static func reason(_ t: TaskView) -> String? {
        let clean = untrusted(t.reason ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return clean.isEmpty ? nil : String(clean.prefix(300))
    }
}

/// A proposed task as the owner narrows it before approving (pure, unit-tested): every change only takes away.
struct TaskDraft: Equatable {
    enum Field: String, CaseIterable, Identifiable {
        case repos, branches, paths, people, operations

        var id: String { rawValue }

        /// The SF Symbol in front of its chips.
        var symbol: String {
            switch self {
            case .repos: "shippingbox"
            case .branches: "arrow.triangle.branch"
            case .paths: "folder"
            case .people: "person"
            case .operations: "square.stack.3d.up"
            }
        }
    }

    /// What the agent asked for (never changed).
    let proposed: TaskView
    /// What the owner approves.
    private(set) var edited: TaskView

    init(_ proposed: TaskView) {
        self.proposed = proposed
        edited = proposed
    }

    func values(_ field: Field) -> [String] {
        switch field {
        case .repos: edited.repos
        case .branches: edited.branches
        case .paths: edited.paths
        case .people: edited.people
        case .operations: edited.operations
        }
    }

    /// A chip as shown: kinds in words, everything else as the agent wrote it, cleaned and never cut (it is what is
    /// approved; the chip truncates in the middle instead).
    static func label(_ field: Field, _ value: String) -> String {
        field == .operations ? TaskText.operationName(value) : untrusted(value)
    }

    /// People can all go (no one is reached); every other list keeps one, since an empty list would mean "any".
    func canRemove(_ field: Field, _ value: String) -> Bool {
        let list = values(field)
        guard list.contains(value) else { return false }
        return field == .people || list.count > 1
    }

    @discardableResult
    mutating func remove(_ field: Field, _ value: String) -> Bool {
        guard canRemove(field, value) else { return false }
        switch field {
        case .repos: edited.repos.removeAll { $0 == value }
        case .branches: edited.branches.removeAll { $0 == value }
        case .paths: edited.paths.removeAll { $0 == value }
        case .people: edited.people.removeAll { $0 == value }
        case .operations: edited.operations.removeAll { $0 == value }
        }
        return true
    }

    /// The ceilings offered: the proposal's and lower.
    var ceilingChoices: [UInt8] { TaskText.ceilings.filter { $0 <= min(proposed.ceiling, 2) } }

    /// Only lower than (or as) proposed.
    @discardableResult
    mutating func setCeiling(_ c: UInt8) -> Bool {
        guard ceilingChoices.contains(c) else { return false }
        edited.ceiling = c
        return true
    }

    var proposedMinutes: Int64 { max((proposed.endsAt - proposed.startedAt) / 60, 0) }
    var minutes: Int64 { max((edited.endsAt - edited.startedAt) / 60, 0) }

    /// The lengths offered: 15 min up to the proposal's.
    var lengthChoices: [Int64] {
        let p = proposedMinutes
        return Array(Set([15, 30, 60, 120, 240, 480, 720, 1_440].filter { $0 < p } + [p])).filter { $0 >= 15 }.sorted()
    }

    /// Only shorter, and 15 minutes at least.
    @discardableResult
    mutating func setMinutes(_ m: Int64) -> Bool {
        guard m >= 15, m <= proposedMinutes else { return false }
        edited.endsAt = edited.startedAt + m * 60
        return true
    }

    /// The budgets offered: none (when the proposal has none) and smaller ones, up to the proposal's.
    var budgetChoices: [UInt32?] {
        let steps: [UInt32] = [5, 10, 20, 50, 100]
        guard let p = proposed.budget else { return [nil] + steps.map { Optional($0) } }
        return (steps.filter { $0 < p } + [p]).map { Optional($0) }
    }

    /// Only smaller, never none once the agent asked for a limit.
    @discardableResult
    mutating func setBudget(_ b: UInt32?) -> Bool {
        guard budgetChoices.contains(b), b != 0 else { return false }
        edited.budget = b
        return true
    }

    /// Something was taken away.
    var narrowed: Bool { edited != proposed }
}

/// What the cards do with the core (testable with the demo core).
@MainActor
enum TaskActions {
    /// Approves the proposal `requestId` as narrowed. The core asks the owner first (a task loosens) and refuses
    /// anything wider than proposed. Returns nil when approved, else what to say.
    static func approve(_ app: AppModel, requestId: String, draft: TaskDraft) async -> String? {
        do {
            try await app.core.approveTask(requestId: requestId, draft: draft.edited, owner: app.ownerCheck)
        } catch {
            app.feedback.play(.error)
            await app.refreshPending()
            return AutopilotModel.sentence(decisionErrorMessage(error))
        }
        app.feedback.play(.approved)
        await app.refreshPending()
        return nil
    }

    /// Deny: the ordinary denial (no check: it only takes away).
    static func deny(_ app: AppModel, item: PendingItem) async {
        await Burst.answer(app, ids: [item.id], label: item.connectionLabel, approve: false)
    }

    /// Stop: ends the task now; never asked (it only takes away). Returns nil when it ended, else what to say.
    static func end(_ app: AppModel, id: String) async -> String? {
        do {
            try await app.core.endTask(id: id)
        } catch {
            app.feedback.play(.error)
            return AutopilotModel.sentence(error.userMessage)
        }
        app.feedback.play(.selection)
        await app.refreshTasks()
        return nil
    }
}

// MARK: - The proposal card

/// The agent's proposals, one card each, with what waits.
struct TaskProposalCards: View {
    var items: [PendingItem]

    var body: some View {
        VStack(spacing: 10) {
            ForEach(items, id: \.id) { item in
                if let proposal = item.taskProposal {
                    TaskProposalCard(item: item, proposal: proposal)
                }
            }
        }
        .padding(.horizontal, 16)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("taskProposals")
    }
}

/// "Task Release v2.1": who proposes it, the agent's words (labelled as the agent's), the envelope as removable chips,
/// Up to / For / Budget that only go down, then Deny and Approve.
struct TaskProposalCard: View {
    var item: PendingItem
    var proposal: TaskView
    @Environment(AppModel.self) private var model
    @State private var draft: TaskDraft?
    @State private var busy = false
    @State private var error: String?
    @State private var more = false

    private var current: TaskDraft { draft ?? TaskDraft(proposal) }

    var body: some View {
        WaitCardShell(
            kind: .grant, label: TaskText.taskWord,
            ask: AskText([AskText.Run(text: "\(TaskText.taskWord) "), AskText.Run(text: TaskText.title(proposal), target: true)], role: .cardAsk),
            askIdentifier: "taskTitle:\(item.id)"
        ) {
            ConnectionIcon(connectionId: item.connectionId, label: item.connectionLabel, size: 34)
        } timer: {
            Button {
                withAnimation(.smooth(duration: 0.25)) { more.toggle() }
            } label: {
                Image(systemName: more ? "chevron.up" : "ellipsis")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Palette.onField)
                    .frame(width: 32, height: 32)
                    .background(Palette.onFieldWash, in: Circle())
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(PressDimsStyle())
            .accessibilityLabel(more ? "Less" : "More")
            .accessibilityIdentifier("taskMore:\(item.id)")
        } actions: {
            VStack(alignment: .leading, spacing: 12) {
                let who = untrustedName(item.connectionLabel, limit: WaitCardIdentity.limit)
                if !who.isEmpty {
                    Text(verbatim: "“\(who)”")
                        .font(RFont.sans(13.5, .medium))
                        .foregroundStyle(Palette.onField2)
                        .lineLimit(1)
                        .padding(.top, -4)
                        .accessibilityLabel("From \(who)")
                }
                if let reason = TaskText.reason(proposal) {
                    AgentReason(text: reason, lines: more ? 8 : 2)
                        .accessibilityIdentifier("taskReason:\(item.id)")
                }
                ForEach(TaskDraft.Field.allCases) { field in
                    let values = current.values(field)
                    if !values.isEmpty {
                        chips(field, values)
                    }
                }
                // The ceiling is never cut ("Up to: Reaches people"): it takes a line of its own when the row is full.
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 6) {
                        ceilingMenu
                        lengthMenu
                        if more || proposal.budget != nil { budgetMenu }
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        ceilingMenu
                        HStack(spacing: 6) {
                            lengthMenu
                            if more || proposal.budget != nil { budgetMenu }
                        }
                    }
                }
                if let error {
                    Text(error).font(RFont.sans(13.5, .medium)).foregroundStyle(Palette.onField).lineLimit(3)
                        .accessibilityIdentifier("taskError:\(item.id)")
                }
                HStack(spacing: 8) {
                    Button("Deny") { Task { await TaskActions.deny(model, item: item) } }
                        .buttonStyle(SignalButtonStyle(.onFieldWash, size: .small))
                        .accessibilityIdentifier("taskDeny:\(item.id)")
                    Button {
                        Task { await approve() }
                    } label: {
                        Label { Text("Approve") } icon: { BiometryMark(size: 15) }
                    }
                    .buttonStyle(SignalButtonStyle(.onField, size: .small))
                    .accessibilityIdentifier("taskApprove:\(item.id)")
                }
            }
        }
        .disabled(busy)
        .accessibilityIdentifier("taskProposal:\(item.id)")
    }

    private func chips(_ field: TaskDraft.Field, _ values: [String]) -> some View {
        ChipFlow(spacing: 6) {
            Image(systemName: field.symbol)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Palette.onField2)
                .frame(height: 28)
                .accessibilityHidden(true)
            ForEach(values, id: \.self) { value in
                let removable = current.canRemove(field, value)
                HStack(spacing: 4) {
                    Text(verbatim: TaskDraft.label(field, value))
                        .font(field == .operations ? RFont.sans(13, .medium) : RFont.mono(12.5, .medium))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .environment(\.layoutDirection, .leftToRight)
                    if removable {
                        Button {
                            edit { $0.remove(field, value) }
                        } label: {
                            Image(systemName: "xmark").font(.system(size: 10, weight: .bold))
                                .frame(width: 22, height: 22)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(PressDimsStyle())
                        .accessibilityLabel("Remove \(TaskDraft.label(field, value))")
                        .accessibilityIdentifier("taskRemove:\(field.rawValue):\(value)")
                    }
                }
                .foregroundStyle(Palette.onField)
                .padding(.leading, 10)
                .padding(.trailing, removable ? 3 : 10)
                .frame(minHeight: 28)
                .background(Palette.onFieldWash, in: Capsule())
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("taskChips:\(field.rawValue)")
    }

    private var ceilingMenu: some View {
        Menu {
            ForEach(current.ceilingChoices, id: \.self) { c in
                Button(TaskText.ceilingName(c)) { edit { $0.setCeiling(c) } }
            }
        } label: {
            menuLabel(TaskText.upTo(current.edited.ceiling))
        }
        .accessibilityIdentifier("taskCeiling:\(item.id)")
    }

    private var lengthMenu: some View {
        Menu {
            ForEach(current.lengthChoices, id: \.self) { m in
                Button(TaskText.length(minutes: m)) { edit { $0.setMinutes(m) } }
            }
        } label: {
            menuLabel(TaskText.length(minutes: current.minutes))
        }
        .accessibilityIdentifier("taskLength:\(item.id)")
    }

    private var budgetMenu: some View {
        Menu {
            ForEach(current.budgetChoices, id: \.self) { b in
                Button(TaskText.budget(b)) { edit { $0.setBudget(b) } }
            }
        } label: {
            menuLabel(TaskText.budget(current.edited.budget))
        }
        .accessibilityIdentifier("taskBudget:\(item.id)")
    }

    private func menuLabel(_ text: String) -> some View {
        HStack(spacing: 3) {
            Text(text).lineLimit(1)
            Image(systemName: "chevron.down").font(.system(size: 10, weight: .bold))
        }
        .font(RFont.sans(13, .semibold))
        .foregroundStyle(Palette.onField)
        .padding(.horizontal, 10)
        .frame(minHeight: 30)
        .background(Palette.onFieldWash, in: Capsule())
    }

    private func edit(_ change: (inout TaskDraft) -> Bool) {
        var d = current
        if change(&d) {
            draft = d
            model.feedback.play(.selection)
        }
    }

    private func approve() async {
        busy = true
        error = await TaskActions.approve(model, requestId: item.id, draft: current)
        busy = false
    }
}

/// The agent's reason on the field: "Agent says" over its words in italics, never Reins' own.
private struct AgentReason: View {
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
            .foregroundStyle(Palette.onField2)
            Text(verbatim: text)
                .font(RFont.sans(13.5))
                .italic()
                .foregroundStyle(Palette.onField)
                .lineLimit(lines)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.onFieldWash, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(TicketText.agentSaysLabel): \(text)")
    }
}

// MARK: - Live and ended tasks

/// Activity's task chips: each live task as "Task · Fix CI" with Stop, each that ended today as a quiet line.
struct TaskChips: View {
    var tasks: [TaskView]
    @Environment(AppModel.self) private var model

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            let now = Int64(context.date.timeIntervalSince1970)
            let live = TaskText.live(tasks, now: now)
            let ended = TaskText.ended(tasks, now: now)
            if !live.isEmpty || !ended.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(live, id: \.id) { t in LiveTaskChip(task: t) }
                    ForEach(ended, id: \.id) { t in
                        HStack(spacing: 6) {
                            Image(systemName: "checkmark.circle").font(.system(size: 12, weight: .semibold))
                            Text(verbatim: TaskText.endedLine(t)).lineLimit(1)
                        }
                        .font(RFont.sans(12.5, .medium))
                        .foregroundStyle(Palette.inkMeta)
                        .padding(.horizontal, 4)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("taskEnded:\(t.id)")
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("tasks")
            }
        }
    }
}

/// "Task · Fix CI" in a capsule, who it is for, and Stop (never asked: it only takes away).
private struct LiveTaskChip: View {
    var task: TaskView
    @Environment(AppModel.self) private var model
    @State private var busy = false

    var body: some View {
        HStack(spacing: 8) {
            ConnectionIcon(connectionId: task.connectionId, label: task.connectionLabel, size: 20)
            Image(systemName: "scope").font(.system(size: 12, weight: .semibold)).accessibilityHidden(true)
            Text(verbatim: TaskText.chip(task))
                .font(RFont.sans(13.5, .semibold))
                .lineLimit(1)
            Spacer(minLength: 4)
            Text(TaskText.upTo(task.ceiling))
                .font(RFont.sans(12, .medium))
                .foregroundStyle(Palette.inkMeta)
                .lineLimit(1)
            Button(TaskText.stop) {
                busy = true
                Task {
                    if let message = await TaskActions.end(model, id: task.id) { model.notice = message }
                    busy = false
                }
            }
            .buttonStyle(SignalButtonStyle(.outline, size: .small, fill: false))
            .fixedSize()
            .accessibilityIdentifier("taskStop:\(task.id)")
        }
        .foregroundStyle(Palette.ink)
        .padding(.leading, 10)
        .padding(.trailing, 6)
        .padding(.vertical, 6)
        .background(Palette.paperRaised, in: Capsule())
        .overlay(Capsule().stroke(Palette.rule, lineWidth: 1))
        .disabled(busy)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("task:\(task.id)")
    }
}
