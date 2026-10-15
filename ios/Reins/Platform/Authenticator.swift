import LocalAuthentication

/// Confirms that the phone's owner is holding it before anything is approved (the Android app's `BiometricPrompt`
/// wrapper): Face ID or Touch ID, falling back to the passcode. Fails closed: no passcode set means no approval.
protocol Authenticating: AnyObject {
    /// True once the owner confirmed. `reason` is shown under the system prompt ("Approve: Claude: Send email").
    func confirm(_ reason: String) async -> Bool
}

final class Authenticator: Authenticating {
    func confirm(_ reason: String) async -> Bool {
        // A prompt that never came up is asked once more: the system's Face ID screen can fail to start in time (the
        // first prompt after a boot: "UI activation timed out"), which would otherwise look like a tap that did nothing.
        for attempt in 0..<2 {
            let context = LAContext()
            context.localizedFallbackTitle = "Use Passcode"
            var error: NSError?
            guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) else { return false }
            do {
                return try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)
            } catch {
                if attempt == 0 && Self.promptDidNotRun(error) { continue }
                return false
            }
        }
        return false
    }

    /// Failures that are not an answer (cancel, fallback, a wrong face, a lockout, the app leaving the screen): the prompt
    /// itself broke.
    static func promptDidNotRun(_ error: Error) -> Bool {
        let e = error as NSError
        guard e.domain == LAErrorDomain else { return false }
        let answered: [LAError.Code] = [
            .userCancel, .userFallback, .authenticationFailed, .appCancel, .systemCancel, .biometryLockout, .passcodeNotSet,
            .biometryNotEnrolled, .biometryNotAvailable, .invalidContext, .notInteractive,
        ]
        return !answered.map(\.rawValue).contains(e.code)
    }
}

/// The core's `OwnerCheck`: the core asks it itself, in its own words, before what only the owner may do (starting
/// Overnight or changing its length, changing what it does with what is too risky). It is the same check as a
/// bypass's, `Authenticator` in the app (Face ID or Touch ID, else the passcode), so it fails closed the same way.
final class AuthenticatorOwnerCheck: OwnerCheck, @unchecked Sendable {
    private let authenticator: Authenticating

    init(_ authenticator: Authenticating) {
        self.authenticator = authenticator
    }

    func confirm(reason: String) async throws -> Bool {
        await authenticator.confirm(reason)
    }
}

/// Always says yes: the `-demo` build and UI tests (the simulator has no enrolled face by default).
final class TrustingAuthenticator: Authenticating {
    func confirm(_ reason: String) async -> Bool { true }
}
