#if REINS_APP
import Foundation

/// Where the intents meet the app: the one `AppModel` of the process, set at launch by `AppHost`. An intent can be
/// the reason the app was launched (in the background), so each call first waits for the store to be open.
@MainActor
enum IntentBridge {
    static weak var model: AppModel?

    /// The model once the session was read; nil when it never came (the store failed to open).
    static func ready() async -> AppModel? {
        for _ in 0..<100 {
            if let model, model.session != .loading { return model }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return model
    }

    private static func signedIn() async throws -> AppModel {
        guard let model = await ready(), case .signedIn = model.session else { throw ReinsIntentError.signedOut }
        return model
    }

    /// Lockdown on, or off back to the mode it interrupted (Manual if that was Lockdown too, as the Android app's
    /// "End lockdown"). Turning it on denies what waits, so the list is read again.
    static func setLockdown(_ on: Bool) async throws {
        let model = try await signedIn()
        guard let settings = await model.refreshAutopilot() else {
            throw ReinsIntentError.failed("Autopilot could not be read. Try again in the app.")
        }
        if let target = lockdownTarget(on: on, mode: settings.mode, base: settings.baseMode) {
            do {
                try await model.core.setAutopilotMode(connectionId: nil, mode: target, minutes: nil)
            } catch {
                throw ReinsIntentError.failed(error.userMessage)
            }
        }
        await model.refreshAutopilot()
        await model.refreshPending()
    }

    /// The global mode to set for Lockdown `on` / off, nil when nothing changes.
    nonisolated static func lockdownTarget(on: Bool, mode: AutopilotMode, base: AutopilotMode) -> AutopilotMode? {
        if on { return mode == .lockdown ? nil : .lockdown }
        guard mode == .lockdown else { return nil }
        return base == .lockdown ? .manual : base
    }

    /// Lockdown off ("Resume Reins"); false when it was not on.
    static func endLockdown() async throws -> Bool {
        let model = try await signedIn()
        let on = await model.refreshAutopilot()?.mode == .lockdown
        if on { try await setLockdown(false) }
        return on
    }

    /// Ends every bypass; false when none ran.
    static func stopBypass() async throws -> Bool {
        let model = try await signedIn()
        let running = await model.refreshAutopilot()?.lastBypassEnd != nil
        if running { await model.stopBypasses() }
        return running
    }

    /// Denies a request. One that is gone (answered, expired) is not an error: there is nothing left to deny.
    static func deny(_ requestId: String) async throws {
        guard DeepLink.isId(requestId) else { return }
        let model = try await signedIn()
        do {
            try await model.core.deny(requestId: requestId)
        } catch CoreError.NotFound {
            // Already answered or expired.
        } catch {
            throw ReinsIntentError.failed(error.userMessage)
        }
        await model.refreshPending()
    }

    /// Opens a link in the app, as a tapped notification would.
    static func open(_ link: DeepLink) async {
        guard let model = await ready() else { return }
        await model.handle(link)
    }
}
#endif
