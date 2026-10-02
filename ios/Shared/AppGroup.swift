import Foundation

/// What the app, its notification extension and its widgets share: one App Group container and one keychain group.
enum AppGroup {
    static let id: String = Bundle.main.object(forInfoDictionaryKey: "ReinsAppGroup") as? String ?? "group.dev.rewarden.ios"

    /// `$(AppIdentifierPrefix)dev.rewarden.ios.shared`, or nil when the build has no team prefix (unsigned simulator
    /// builds), in which case keychain items stay in the process's default group.
    static let keychainGroup: String? = {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "ReinsKeychainGroup") as? String,
              !group.hasPrefix("."), !group.hasPrefix("$")
        else { return nil }
        return group
    }()

    /// The shared container; falls back to the process's own Application Support where App Groups are unavailable.
    static var container: URL {
        if let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: id) {
            return url
        }
        return FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    }

    /// Settings every process reads (feedback switches, device status, the widget snapshot).
    static let defaults: UserDefaults = UserDefaults(suiteName: id) ?? .standard

    /// A directory in the shared container, created on first use, kept out of backups and readable after the first
    /// unlock (pushes are handled while the phone is locked).
    static func directory(_ name: String) -> URL {
        var url = container.appendingPathComponent("Library/Application Support", isDirectory: true)
            .appendingPathComponent(name, isDirectory: true)
        let fm = FileManager.default
        if !fm.fileExists(atPath: url.path) {
            try? fm.createDirectory(
                at: url,
                withIntermediateDirectories: true,
                attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
            )
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? url.setResourceValues(values)
        }
        return url
    }
}
