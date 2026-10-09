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

    /// Autopilot's mode for every AI ("Set Reins to Assisted").
    static func setMode(_ option: AutopilotModeOption) async throws {
        let model = try await signedIn()
        do {
            try await model.core.setAutopilotMode(connectionId: nil, mode: option.mode, minutes: nil)
        } catch {
            throw ReinsIntentError.failed(error.userMessage)
        }
        await model.refreshAutopilot()
        await model.refreshPending()
    }

    private static let focusBeforeKey = "focus.previousMode"
    private static let focusSetKey = "focus.setMode"

    /// A Focus filter: `option` while the Focus is on; nil (the Focus ended) puts back the mode from before it.
    ///
    /// Nobody confirms a Focus as it starts (a schedule, a place), so a Focus may only make Autopilot stricter: Manual,
    /// Assisted or Lockdown, never Auto. The mode from before is put back only if the mode is still the one the Focus
    /// set: one the user chose meanwhile (Lockdown, say) stays. A bypass that ran when the Focus began comes back as the
    /// mode under it, not as a bypass.
    static func applyFocus(_ option: AutopilotModeOption?, defaults: UserDefaults = AppGroup.defaults) async throws {
        let model = try await signedIn()
        if let option {
            guard option != .auto else {
                throw ReinsIntentError.failed("A Focus can make Autopilot stricter, not set it to Auto.")
            }
            let settings = await model.refreshAutopilot()
            let before = defaults.string(forKey: focusBeforeKey)
                ?? settings.flatMap { AutopilotModeOption($0.mode) ?? AutopilotModeOption($0.baseMode) }?.rawValue
            try await setMode(option)
            // Remembered only once the mode is set, so a failed start leaves nothing to put back.
            if let before { defaults.set(before, forKey: focusBeforeKey) }
            defaults.set(option.rawValue, forKey: focusSetKey)
        } else {
            let before = defaults.string(forKey: focusBeforeKey).flatMap(AutopilotModeOption.init(rawValue:))
            let set = defaults.string(forKey: focusSetKey).flatMap(AutopilotModeOption.init(rawValue:))
            defaults.removeObject(forKey: focusBeforeKey)
            defaults.removeObject(forKey: focusSetKey)
            guard let before, let set else { return }
            let now = await model.refreshAutopilot()?.mode
            if now == set.mode { try await setMode(before) }
        }
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

    /// Approves a routine request as the notification's Approve does. Gone (answered, expired) is no error; a request
    /// the core keeps for the sheet (asked every time) says so.
    static func approveQuick(_ requestId: String) async throws {
        guard DeepLink.isId(requestId) else { return }
        let model = try await signedIn()
        do {
            try await model.core.approveQuick(requestId: requestId)
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

extension AutopilotModeOption {
    /// The core's mode; nil for Bypass, which intents do not set.
    init?(_ mode: AutopilotMode) {
        switch mode {
        case .manual: self = .manual
        case .assisted: self = .assisted
        case .auto: self = .auto
        case .lockdown: self = .lockdown
        case .bypass: return nil
        }
    }

    var mode: AutopilotMode {
        switch self {
        case .manual: .manual
        case .assisted: .assisted
        case .auto: .auto
        case .lockdown: .lockdown
        }
    }
}
#endif
