import Foundation
import UserNotifications

/// The core's `Notifier` in the app process. The core calls it from its own threads when something waits, was
/// resolved, or Autopilot acted; it hops to the main actor and refreshes the model. Someone looking at the app sees the
/// change (the sheet pops up, the list moves), so in front it only plays the cue; otherwise (a background push wake,
/// a background refresh, a notification action) it posts the same notification the extension would, with the same
/// wording, category, sound and lock-screen privacy (the Android app's `AppNotifier`).
final class AppNotifier: Notifier, @unchecked Sendable {
    @MainActor weak var model: AppModel?
    private let center: UNUserNotificationCenter
    private let settings: @Sendable () -> FeedbackSettings

    init(center: UNUserNotificationCenter = .current(), settings: @escaping @Sendable () -> FeedbackSettings = { FeedbackSettings.load() }) {
        self.center = center
        self.settings = settings
    }

    func itemPending(item: PendingItem) {
        Task { @MainActor in
            guard let model else { return }
            await model.refreshPending()
            if model.isActive {
                model.feedback.play(.requestArrived)
                return
            }
            self.post(item.id, NotificationContent.pending(item, settings: self.settings()))
        }
    }

    func itemResolved(id: String) {
        // Answered here, on another device, by a grant or by its time running out: its notification has nothing left
        // to offer.
        let ids = [id, Self.decisionId(id)]
        center.removeDeliveredNotifications(withIdentifiers: [id])
        center.removePendingNotificationRequests(withIdentifiers: ids)
        Task { @MainActor in await self.model?.refreshPending() }
    }

    func autoDecided(decision: AutoDecisionView) {
        Task { @MainActor in
            guard let model else { return }
            await model.refreshPending()
            if model.isActive {
                // In front the list changes before the user's eyes; a quiet sound says why.
                model.feedback.play(decision.verdict == .approve ? .autoApproved : .autoDenied)
                return
            }
            self.post(Self.decisionId(decision.requestId), NotificationContent.decision(decision, settings: self.settings()))
        }
    }

    func autopilotChanged(event: AutopilotEvent) {
        Task { @MainActor in
            guard let model else { return }
            await model.refreshAutopilot()
            switch event {
            case .bypassEnded:
                if model.isActive { model.feedback.play(.bypassOff) }
            case let .paused(connectionId, connectionLabel, reason):
                // Auto-approvals stopped for a connection (unusual volume); its requests wait for the user.
                let inFront = model.isActive
                if inFront { model.feedback.play(.alert) }
                self.post("paused:\(connectionId)", NotificationContent.status(
                    title: NotificationText.pausedTitle(connectionLabel),
                    body: NotificationText.pausedBody(reason),
                    link: .autopilot,
                    settings: self.settings(),
                    silent: inFront
                ))
            case .modeChanged:
                break
            }
        }
    }

    /// "This phone is no longer your approval device." Posted in front too (it explains itself), silently there.
    @MainActor
    func deviceReplaced() {
        let inFront = model?.isActive ?? false
        if inFront { model?.feedback.play(.alert) }
        post(Self.replacedId, NotificationContent.status(
            title: NotificationText.replacedTitle,
            body: NotificationText.replacedBody,
            link: .home,
            settings: settings(),
            silent: inFront
        ))
    }

    /// How Autopilot's model download ended, when the user is not looking at the app (for the model download).
    @MainActor
    func modelDownloadFinished(failure: String?) {
        guard model?.isActive != true else { return }
        let c = NotificationContent.status(
            title: failure == nil ? "Autopilot's model is ready" : "The model could not be downloaded",
            body: failure ?? "It runs on this phone; nothing leaves it.",
            link: .autopilot,
            settings: settings(),
            silent: true
        )
        c.interruptionLevel = .passive
        post("model-download", c)
    }

    /// A Deny from a notification that did not go through.
    @MainActor
    func denyFailed(_ message: String) {
        post("deny-failed", NotificationContent.status(
            title: "The request was not denied",
            body: message,
            link: .home,
            settings: settings(),
            silent: false
        ))
    }

    func post(_ id: String, _ content: UNNotificationContent) {
        center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil)) { _ in
            // Not allowed or not possible: the app shows it all the next time it is opened.
        }
    }

    static let replacedId = "device-replaced"
    static func decisionId(_ requestId: String) -> String { "auto:\(requestId)" }
}
