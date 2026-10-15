import Foundation

/// The mode an Overnight interrupted, kept on this phone for its Stop. The core keeps it too and returns to it when
/// Overnight reaches its end, but while Overnight runs its settings read Overnight as the mode set (`baseMode`), so
/// stopping it early needs the mode from before: the one remembered here when it started, else the safe default
/// (Manual for every AI, following every AI for one).
enum OvernightReturn {
    private static let prefix = "overnight.return."

    private static func key(_ connectionId: String?) -> String { prefix + (connectionId ?? "") }

    /// Remembers what Overnight interrupts in `connectionId`'s scope (nil: every AI): `base`, the mode set there before
    /// it started (nil: the default, or following every AI). One that already runs keeps what it first interrupted;
    /// Lockdown is not returned to (the core ends a lockdown that Overnight replaces), so it is not kept either.
    static func remember(_ base: AutopilotMode?, connectionId: String?, defaults: UserDefaults = AppGroup.defaults) {
        if base == .overnight { return }
        guard let base, base != .lockdown else {
            defaults.removeObject(forKey: key(connectionId))
            return
        }
        defaults.set(base.key, forKey: key(connectionId))
    }

    /// The mode to go back to when Overnight is stopped in this scope: nil for a connection means following every AI.
    static func mode(_ connectionId: String?, defaults: UserDefaults = AppGroup.defaults) -> AutopilotMode? {
        let stored = defaults.string(forKey: key(connectionId))
        let mode = stored.flatMap { raw in AutopilotText.modes.first { AutopilotText.key($0) == raw } }
            .flatMap { [AutopilotMode.manual, .auto].contains($0) ? $0 : nil }
        if connectionId == nil { return mode ?? .manual }
        return mode
    }

    /// Done with: Overnight ended or was stopped in this scope.
    static func forget(_ connectionId: String?, defaults: UserDefaults = AppGroup.defaults) {
        defaults.removeObject(forKey: key(connectionId))
    }
}
