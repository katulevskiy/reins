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

    /// Approving needs the owner's Face ID, Touch ID or passcode first; anything but a confirmation does nothing.
    func approve(_ app: AppModel) async {
        guard let view, !busy else { return }
        let choice: ApprovalChoice
        switch buildChoice(view, draft) {
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
        switch await OwnerCheck.confirm(app, reason: reason) {
        case .confirmed:
            // A permission that stays is a bigger step than one answer, and sounds like it.
            let standing = choice.standing != nil || view.kind == .grant
            app.feedback.play(standing ? .grantCreated : .approved)
            do {
                try await app.core.approve(requestId: requestId, choice: choice)
                await app.refreshPending()
                busy = false
                finished = true
            } catch {
                app.feedback.play(.error)
                busy = false
                self.error = decisionErrorMessage(error)
            }
        case .cancelled:
            busy = false
        case .unavailable:
            app.feedback.play(.error)
            busy = false
            error = OwnerCheck.unavailableMessage
        }
    }

    /// Denying needs no confirmation: it can only take something away.
    func deny(_ app: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        app.feedback.play(.denied)
        do {
            try await app.core.deny(requestId: requestId)
            await app.refreshPending()
            busy = false
            finished = true
        } catch {
            app.feedback.play(.error)
            busy = false
            self.error = decisionErrorMessage(error)
        }
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

    /// "Search Gmail", "Push with git", "Create issue".
    var headline: String {
        let title = mcp.map { untrusted($0.title) } ?? opTitle
        return operationTitle(action: actionKind.rawValue, count: shownCount, service: service, title: title, op: op)
    }

    var isQuestion: Bool { ask != nil }
}
