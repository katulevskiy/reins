import Foundation

/// Whether the app is in front, as the notification extension sees it. The app writes a timestamp into the App Group
/// while its scene is active and keeps it fresh; the extension then leaves the push alone (the app pops the sheet and
/// chimes itself, as Android's `Foreground.focused`). A crashed app's mark goes stale within `freshness`.
enum AppPresence {
    static let key = "app.inFrontAt"
    /// The app beats every `interval`; a mark older than `freshness` counts as gone.
    static let interval: TimeInterval = 15
    static let freshness: TimeInterval = 40

    static func inFront(now: Date = Date(), defaults: UserDefaults = AppGroup.defaults) -> Bool {
        let at = defaults.double(forKey: key)
        guard at > 0 else { return false }
        let age = now.timeIntervalSince1970 - at
        return age >= -5 && age < freshness
    }

    #if REINS_APP
    @MainActor private static var beat: Task<Void, Never>?

    /// Called by `AppModel.setActive`.
    @MainActor static func setActive(_ active: Bool) {
        beat?.cancel()
        beat = nil
        let defaults = AppGroup.defaults
        guard active else {
            defaults.removeObject(forKey: key)
            return
        }
        beat = Task { @MainActor in
            while !Task.isCancelled {
                defaults.set(Date().timeIntervalSince1970, forKey: key)
                try? await Task.sleep(for: .seconds(interval))
            }
        }
    }
    #endif
}
