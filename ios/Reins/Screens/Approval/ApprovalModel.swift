import Foundation
import Observation

/// The approval sheet's state (the Android app's ApprovalViewModel): the request as the core shows it, what the user
/// changed, and the answer on its way.
@Observable
@MainActor
final class ApprovalModel {
    let requestId: String
    private(set) var loading = true
    private(set) var view: ApprovalView?
    var draft = ApprovalDraft() {
        didSet { if draft != oldValue { error = nil } }
    }
    private(set) var busy = false
    var error: String?
    private(set) var finished = false
    /// What Autopilot made of the request (Manual and Auto), if it looked at it.
    private(set) var suggestion: SuggestionView?
    /// An Activity entry's request shown again as it was decided: nothing can be changed or answered.
    private(set) var readOnly = false

    init(requestId: String) {
        self.requestId = requestId
    }

    /// An Activity entry's request as its approval showed it (`ReinsCoreProtocol.activityApproval`), read-only, with
    /// `released` (the emails and addresses that were shared) ticked.
    init(recorded view: ApprovalView, released: Set<String>) {
        requestId = view.requestId
        self.view = view
        var shown = ApprovalDraft.initial(for: view)
        shown.selected = released
        shown.allMail = nil
        draft = shown
        loading = false
        readOnly = true
    }

    /// A model already holding `view` (previews and tests).
    init(view: ApprovalView, suggestion: SuggestionView? = nil) {
        requestId = view.requestId
        self.view = view
        self.suggestion = suggestion
        draft = .initial(for: view)
        loading = false
    }

    func load(_ app: AppModel) async {
        guard view == nil, !readOnly else { return }
        do {
            let v = try await app.core.approvalView(requestId: requestId)
            view = v
            draft = .initial(for: v)
            loading = false
            // A hint only: without one (Lockdown, Bypass, a failure) the sheet is simply as before.
            suggestion = try? await app.core.autopilotSuggestion(requestId: requestId)
        } catch {
            loading = false
            self.error = decisionErrorMessage(error)
        }
    }

    var preview: BuildResult? { view.map { buildDecision($0, draft) } }

    /// Asks the core again for Autopilot's score (after its model was installed from the approval screen).
    func reloadSuggestion(_ app: AppModel) async {
        suggestion = try? await app.core.autopilotSuggestion(requestId: requestId)
    }

    /// Approving needs the owner's Face ID, Touch ID or passcode first; anything but a confirmation does nothing. What
    /// is remembered alongside is what the dock and its "Remember for" sheet say (`buildDecision`).
    func approve(_ app: AppModel) async {
        guard let view, !busy, !readOnly else { return }
        let choice: ApprovalChoice
        switch buildDecision(view, draft) {
        case let .invalid(message):
            app.feedback.play(.error)
            error = message
            return
        case let .ok(c):
            choice = c
        }
        busy = true
        error = nil
        let allow = view.kind == .grant || view.kind == .accounts
        let reason = "\(allow ? "Allow access for" : "Approve the request from") \(untrusted(view.connectionLabel))"
        switch await OwnerPrompt.confirm(app, reason: reason) {
        case .confirmed:
            // A permission that stays is a bigger step than one answer, and sounds like it.
            let standing = choice.standing != nil || view.kind == .grant
            app.feedback.play(standing ? .grantCreated : .approved)
            // The sheet closes now; the action (an email going out, a push) finishes in the background.
            let (core, id, answer) = (app.core, requestId, choice)
            app.answerInBackground(id, failed: "Not approved") { try await core.approve(requestId: id, choice: answer) }
            busy = false
            finished = true
        case .cancelled:
            busy = false
        case .unavailable:
            app.feedback.play(.error)
            busy = false
            error = OwnerPrompt.unavailableMessage
        }
    }

    /// Denying needs no confirmation: it can only take something away.
    /// Closes at once; the answer goes out in the background (and the request comes back if it could not).
    func deny(_ app: AppModel) async {
        guard !busy, !readOnly else { return }
        app.feedback.play(.denied)
        let (core, id) = (app.core, requestId)
        app.answerInBackground(id, failed: "Not denied") { try await core.deny(requestId: id) }
        finished = true
    }

    /// Denies with a note the agent receives ("Use the staging bucket"); an empty note is a plain denial.
    func deny(_ app: AppModel, note: String) async {
        guard !busy, !readOnly else { return }
        guard let note = TicketInbox.note(note) else { return await deny(app) }
        app.feedback.play(.denied)
        let (core, id) = (app.core, requestId)
        app.answerInBackground(id, failed: "Not denied") { try await core.denyWithNote(requestId: id, note: note) }
        finished = true
    }

    /// Later: the request becomes a ticket under "Waiting for you"; the agent is told at once and carries on. Nothing
    /// is approved, so no check.
    func decideLater(_ app: AppModel) async {
        guard !busy, !readOnly else { return }
        app.feedback.play(.tap)
        let (core, id) = (app.core, requestId)
        app.ticketInBackground(id, failed: "Not moved to later", leaveStack: true) { try await core.decideLater(requestId: id) }
        finished = true
    }

    /// Why?: the question goes to the agent; the request waits as a ticket for its answer. An empty question does
    /// nothing.
    func askWhy(_ app: AppModel, question: String) async {
        guard !busy, !readOnly, let question = TicketInbox.question(question) else { return }
        app.feedback.play(.tap)
        let (core, id) = (app.core, requestId)
        let instant = app.pending.first { $0.id == id }?.ticket == nil
        app.ticketInBackground(id, failed: "Not asked", leaveStack: instant) { try await core.askWhy(requestId: id, question: question) }
        finished = true
    }
}

extension ApprovalView {
    /// The operation's kind for its icon: what the core says it is, else what the request kind implies.
    var actionKind: ActionKind {
        switch kind {
        case .search: return .search
        case .read: return .read
        case .send: return .send
        case .grant: return .grant
        case .accounts: return .accounts
        case .fetch:
            let k = ActionKind.of(action)
            return k == .other ? .read : k
        case .write:
            let k = ActionKind.of(action)
            return k == .other ? .write : k
        }
    }

    /// What the request covers: the emails found for a search, else what the core says was asked for.
    var shownCount: Int {
        switch kind {
        case .search, .fetch: messages.count
        case .write: 1
        default: Int(count)
        }
    }

    /// "Search Gmail", "Push with git", "Create issue" (the core's `headline` is the sentence of what approving does).
    var titleLine: String {
        let title = mcp.map { untrusted($0.title) } ?? opTitle
        return operationTitle(action: actionKind.rawValue, count: shownCount, service: service, title: title, op: op)
    }

    var isQuestion: Bool { ask != nil }
}
