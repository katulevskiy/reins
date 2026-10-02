import LocalAuthentication
import SwiftUI

/// What the grant screens do through the core. Each returns the reason it did not happen, or nil when it did (or the
/// user changed their mind at the Face ID prompt), and plays the same feedback as the Android app.
extension AppModel {
    /// Resuming gives an AI access again, so it needs the same authentication as approving. `standing` carries the
    /// changes made under "More options"; without it the grant comes back exactly as it was.
    func resumeGrant(_ grantId: String, seconds: Int64, standing: StandingGrant?) async -> String? {
        if let refusal = await confirmOwner("Resume grant") { return refusal.message }
        do {
            feedback.play(.grantCreated)
            if let standing {
                try await core.resumeGrantEdited(grantId: grantId, standing: standing)
            } else {
                try await core.resumeGrant(grantId: grantId, durationSecs: UInt64(max(seconds, 0)))
            }
            await refreshPending()
            return nil
        } catch {
            feedback.play(.error)
            return error.userMessage
        }
    }

    /// Removes an ended grant for good.
    func deleteGrant(_ grantId: String) async -> String? {
        do {
            feedback.play(.revoked)
            try await core.deleteGrant(grantId: grantId)
            await refreshPending()
            return nil
        } catch {
            feedback.play(.error)
            return error.userMessage
        }
    }

    /// Ends a running grant now; it can be resumed later from the Expired list.
    func revokeGrant(_ grantId: String) async -> String? {
        do {
            feedback.play(.revoked)
            try await core.revokeGrant(grantId: grantId)
            await refreshPending()
            return nil
        } catch {
            feedback.play(.error)
            return error.userMessage
        }
    }

    /// Why the owner could not confirm: nil when they did. A cancelled prompt is not an error (`message` nil).
    struct OwnerRefusal {
        var message: String?
    }

    /// Face ID / Touch ID / passcode before giving access. No passcode on the phone fails closed, with the reason.
    func confirmOwner(_ reason: String) async -> OwnerRefusal? {
        if await authenticator.confirm(reason) { return nil }
        // Only the real authenticator can be unavailable; fakes (demo, tests) mean "cancelled".
        if authenticator is Authenticator, !LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: nil) {
            feedback.play(.error)
            return OwnerRefusal(message: "Set a passcode on this iPhone first.")
        }
        return OwnerRefusal(message: nil)
    }

    /// The icon pick stored for a connection: by id, or, for entries logged before ids were recorded (an empty id), by
    /// the label it had then when exactly one connection carries it.
    func iconPick(connectionId: String, label: String) -> String? {
        if let c = connections.first(where: { $0.id == connectionId }) { return c.icon }
        guard connectionId.isEmpty else { return nil }
        let named = connections.filter { $0.label == label }
        return named.count == 1 ? named[0].icon : nil
    }
}
