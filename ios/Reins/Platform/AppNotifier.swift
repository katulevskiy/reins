import Foundation

/// The core's `Notifier`: called from the core's own threads when something waits, was resolved, or Autopilot
/// acted. Hops to the main actor, refreshes the model, and plays the in-app cue when the app is in front.
final class AppNotifier: Notifier, @unchecked Sendable {
    @MainActor weak var model: AppModel?

    func itemPending(item: PendingItem) {
        Task { @MainActor in
            guard let model else { return }
            await model.refreshPending()
            if model.isActive { model.feedback.play(.requestArrived) }
        }
    }

    func itemResolved(id: String) {
        Task { @MainActor in await self.model?.refreshPending() }
    }

    func autoDecided(decision: AutoDecisionView) {
        Task { @MainActor in
            guard let model else { return }
            await model.refreshPending()
            if model.isActive { model.feedback.play(decision.verdict == .approve ? .autoApproved : .autoDenied) }
        }
    }

    func autopilotChanged(event: AutopilotEvent) {
        Task { @MainActor in await self.model?.refreshAutopilot() }
    }
}
