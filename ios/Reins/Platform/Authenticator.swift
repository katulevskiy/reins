import LocalAuthentication

/// Confirms that the phone's owner is holding it before anything is approved (the Android app's `BiometricPrompt`
/// wrapper): Face ID or Touch ID, falling back to the passcode. Fails closed: no passcode set means no approval.
protocol Authenticating: AnyObject {
    /// True once the owner confirmed. `reason` is shown under the system prompt ("Approve: Claude: Send email").
    func confirm(_ reason: String) async -> Bool
}

final class Authenticator: Authenticating {
    func confirm(_ reason: String) async -> Bool {
        let context = LAContext()
        context.localizedFallbackTitle = "Use Passcode"
        var error: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) else { return false }
        do {
            return try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)
        } catch {
            return false
        }
    }
}

/// Always says yes: the `-demo` build and UI tests (the simulator has no enrolled face by default).
final class TrustingAuthenticator: Authenticating {
    func confirm(_ reason: String) async -> Bool { true }
}
