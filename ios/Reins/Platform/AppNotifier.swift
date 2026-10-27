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
            // It waits for the owner: whatever the extension said about it before it knew ("Something is waiting",
            // "Autopilot has still to decide") gives way to the ask itself.
            self.withdraw(item.id, keeping: item.id)
            guard let model else { return }
            if model.isActive {
                await model.refreshPending()
                // The chime still sounds (once per request and per burst), for a phone left open on the desk.
                model.feedback.requestArrived(item.id, needsUser: true)
                return
            }
            // Posted before the refresh: in the background the app may be suspended any moment, and the notification
            // is what the user needs; it never waits on reading the lists.
            self.post(item.id, NotificationContent.pending(item, settings: self.settings()))
            await model.refreshPending()
        }
    }

    func itemResolved(id: String) {
        // Answered here, on another device, by a grant or by its time running out: its notification has nothing left
        // to offer.
        let ids = [id, Self.decisionId(id)]
        withdraw(id)
        center.removePendingNotificationRequests(withIdentifiers: ids)
        Task { @MainActor in await self.model?.refreshPending() }
    }

    func autoDecided(decision: AutoDecisionView) {
        // Decided without the owner: nothing about it may stay actionable. Its ask (posted while it waited, before a
        // bypass or Lockdown started) and the extension's texts go.
        withdraw(decision.requestId)
        Task { @MainActor in
            guard let model else { return }
            // An ask open on it closes, and a list read a moment ago does not bring it back.
            model.decidedAutomatically(decision.requestId)
            // A decision needs nothing from the owner, so it is never a notification; it is in Activity. In front a
            // denial (Lockdown, a "never" rule) gets its quiet sound so the list's change is explained; an approval
            // stays silent (a bypass approves hundreds). Never the request chime.
            if model.isActive {
                model.feedback.requestArrived(decision.requestId, needsUser: false)
                if decision.verdict != .approve { model.feedback.play(.autoDenied) }
            }
            await model.refreshPending()
        }
    }

    func autopilotChanged(event: AutopilotEvent) {
        Task { @MainActor in
            guard let model else { return }
            await model.refreshAutopilot()
            switch event {
            case .bypassEnded:
                if model.isActive { model.feedback.play(.bypassOff) }
            case let .overnightEnded(connectionId):
                // Back to the mode it interrupted, by the core: felt as a step back, as a bypass's end is.
                OvernightReturn.forget(connectionId)
                if model.isActive { model.feedback.play(.autopilotModeChanged(moreAutonomy: false)) }
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

    /// An Approve from the notification did not go through: the request still waits, one tap away.
    func approveFailed(_ message: String, requestId: String) {
        post("approve-failed", NotificationContent.status(
            title: "Not approved yet",
            body: "\(message) Open it to try again.",
            link: .item(kind: .request, id: requestId),
            settings: settings(),
            silent: false
        ))
    }

    /// Takes away every delivered notification about item `id`, by its identifier or the item it names (the
    /// extension's are filed under the push's own identifier), but `keeping`.
    func withdraw(_ id: String, keeping: String? = nil) {
        if id != keeping { center.removeDeliveredNotifications(withIdentifiers: [id]) }
        center.getDeliveredNotifications { delivered in
            let about = NotificationSweep.about(id, delivered: delivered.map { n in
                (n.request.identifier, n.request.content.userInfo[NotificationKey.id] as? String)
            }, keeping: keeping)
            if !about.isEmpty { UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: about) }
        }
    }

    func post(_ id: String, _ content: UNNotificationContent) {
        center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil)) { _ in
            // Not allowed or not possible: the app shows it all the next time it is opened.
        }
    }

    static let replacedId = "device-replaced"
    static func decisionId(_ requestId: String) -> String { "auto:\(requestId)" }
}
