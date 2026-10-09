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
    /// The "More options" section is open.
    var moreOpen = false
    /// What Autopilot made of the request (Assisted and Auto), if it looked at it.
    private(set) var suggestion: SuggestionView?

    init(requestId: String) {
        self.requestId = requestId
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
        guard view == nil else { return }
        do {
            let v = try await app.core.approvalView(requestId: requestId)
            view = v
            draft = .initial(for: v)
            loading = false
            // A hint only: without one (Manual, no model, a failure) the sheet is simply as before.
            suggestion = try? await app.core.autopilotSuggestion(requestId: requestId)
        } catch {
            loading = false
            self.error = decisionErrorMessage(error)
        }
    }

    var preview: BuildResult? { view.map { buildChoice($0, draft) } }

    /// Approving needs the owner's Face ID, Touch ID or passcode first; anything but a confirmation does nothing. With
    /// `allow`, the core's "allow for a while" permission (same AI, same kind of request, same target) is made
    /// alongside, in place of whatever "More options" says.
    func approve(_ app: AppModel, allow quickAllow: Bool = false) async {
        guard let view, !busy else { return }
        let standing = view.quick?.allow
        if quickAllow && standing == nil { return }
        var choice: ApprovalChoice
        switch buildChoice(view, draft) {
        case let .invalid(message):
            app.feedback.play(.error)
            error = message
            return
        case let .ok(c):
            choice = c
        }
        if quickAllow { choice.standing = standing }
        busy = true
        error = nil
        let allow = view.kind == .grant || view.kind == .accounts
        let reason = "\(allow ? "Allow access for" : "Approve the request from") \(untrusted(view.connectionLabel))"
        switch await OwnerCheck.confirm(app, reason: reason) {
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
            error = OwnerCheck.unavailableMessage
        }
    }

    /// Denying needs no confirmation: it can only take something away.
    /// Closes at once; the answer goes out in the background (and the request comes back if it could not).
    func deny(_ app: AppModel) async {
        guard !busy else { return }
        app.feedback.play(.denied)
        let (core, id) = (app.core, requestId)
        app.answerInBackground(id, failed: "Not denied") { try await core.deny(requestId: id) }
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
