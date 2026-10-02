import Foundation
import Observation

/// The user's `FeedbackSettings`, read once from the app group (where the notification extension reads them too) and
/// then served from memory. Screens observe `settings`; the engine reads `current` from any thread on every event.
@Observable
final class FeedbackStore: @unchecked Sendable {
    /// The app's one store: the engine and the Sounds & haptics screen share it.
    static let shared = FeedbackStore()

    /// For SwiftUI. Change it with `update`.
    private(set) var settings: FeedbackSettings

    @ObservationIgnored private let defaults: UserDefaults?
    @ObservationIgnored private let lock = NSLock()
    @ObservationIgnored private var snapshot: FeedbackSettings

    /// `defaults` nil keeps everything in memory (tests, previews).
    init(defaults: UserDefaults? = AppGroup.defaults) {
        let loaded = defaults.map { FeedbackSettings.load(from: $0) } ?? FeedbackSettings()
        self.defaults = defaults
        settings = loaded
        snapshot = loaded
    }

    /// The settings now, from any thread.
    var current: FeedbackSettings {
        lock.lock()
        defer { lock.unlock() }
        return snapshot
    }

    /// Applies `change` and saves; a change that changes nothing is not written. Call on the main thread.
    func update(_ change: (inout FeedbackSettings) -> Void) {
        var next = current
        change(&next)
        next.volume = next.volume.isFinite ? min(max(next.volume, 0), 1) : FeedbackSettings.defaultVolume
        guard next != current else { return }
        lock.lock()
        snapshot = next
        lock.unlock()
        settings = next
        if let defaults { next.save(to: defaults) }
    }
}
