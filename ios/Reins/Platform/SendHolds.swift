import Foundation
import UIKit

/// The email send hold in the app (Decisions v2, 08 R10). The server's silent `hold` push, the app's own countdown and
/// the next sync all come here: the core sends what is due (never before its time on this phone's clock) and turns
/// what came due too long ago into "Still send?", which is an ask, so then (and only then) a notification says so
/// while the app is not in front.
@MainActor
enum SendHolds {
    /// Releases what is due, says "Still send?" for any that came too late (when nobody is looking at the app), and
    /// reads Activity again. Safe any time.
    @discardableResult
    static func release(_ model: AppModel, notifier: AppNotifier?) async throws -> HoldsReleased {
        let released = try await model.core.releaseHolds()
        if released.late > 0, !model.isActive { notifier?.stillSend(released.late) }
        await model.refreshPending()
        return released
    }

    /// A `hold` push (silent, `PushPayload.silent`): the release, as a background fetch's result. Never a
    /// notification of its own.
    static func receive(_ model: AppModel, notifier: AppNotifier?) async -> UIBackgroundFetchResult {
        do {
            let released = try await release(model, notifier: notifier)
            return released.sent + released.late > 0 ? .newData : .noData
        } catch CoreError.Network {
            // Transient: the next push, the countdown or the next sync releases it.
            return .failed
        } catch let CoreError.Server(status, _) where status >= 500 {
            return .failed
        } catch {
            return .noData
        }
    }

    /// Undo, Send (after the owner's check: it goes as the owner's approval) or Discard; a few words when it did not
    /// go through, else nil.
    static func perform(_ action: HoldAction, holdId: String, model: AppModel) async -> String? {
        let core = model.core
        do {
            switch action {
            case .undo, .discard:
                // It only takes something away: no check.
                model.feedback.play(.undo)
                try await core.undoSend(holdId: holdId)
            case .send:
                switch await OwnerPrompt.confirm(model, reason: "Send the email") {
                case .confirmed: break
                case .cancelled: return nil
                case .unavailable:
                    model.feedback.play(.error)
                    return OwnerPrompt.unavailableMessage
                }
                model.feedback.play(.approved)
                try await core.sendLate(holdId: holdId)
            }
            await model.refreshPending()
            return nil
        } catch {
            model.feedback.play(.error)
            await model.refreshPending()
            return message(action, error)
        }
    }

    /// What went wrong, in words: one already gone (sent, undone elsewhere, or its time ran out) says so.
    nonisolated static func message(_ action: HoldAction, _ error: Error) -> String {
        if case CoreError.NotFound = error { return action == .undo ? "Too late to undo: it is being sent." : "That email is no longer waiting." }
        return error.userMessage
    }
}

extension AppNotifier {
    static let stillSendId = "still-send"

    /// "Still send?": held emails a wake found too late to send. An ask, so it notifies; a tap opens Activity, where
    /// Send and Discard are.
    @MainActor
    func stillSend(_ count: UInt32) {
        post(Self.stillSendId, NotificationContent.status(
            title: HoldText.lateTitle,
            body: HoldText.lateBody(count),
            link: .home,
            settings: FeedbackSettings.load(),
            silent: false
        ))
    }
}
