import Foundation

/// What a request arriving while the app is in front gets.
struct ArrivalOutcome: Equatable {
    var chime: Bool
    var haptic: Bool

    static let chimeAndHaptic = ArrivalOutcome(chime: true, haptic: true)
    static let chimeOnly = ArrivalOutcome(chime: true, haptic: false)
    static let hapticOnly = ArrivalOutcome(chime: false, haptic: true)
    static let nothing = ArrivalOutcome(chime: false, haptic: false)
}

/// Decides what a request that arrives while Reins is in front gets instead of its notification: the request chime and
/// the attention haptic (the Android app's `ArrivalPolicy`). Pure and clock-injected so its rules are unit tested.
///
/// Rules, in order:
///  - Only a request that waits for the user. What Autopilot, a bypass or a grant answered gets nothing, and its id is
///    remembered, so a late repeat stays quiet too.
///  - Once per request: one already announced (an update, the same request reaching the app twice) gets nothing.
///  - Once per burst: after a chime or haptic, nothing more for `coalesceMs`. The requests of a burst still count as
///    announced, so none of them chimes later.
///  - The chime: Sounds & haptics, Sounds, Requests and "Sound when open" on.
///  - The haptic: Sounds & haptics and Haptics on.
///
/// The phone's own settings act below this: the chime plays in the `.ambient` audio session (`SoundPlayer`), which the
/// ring/silent switch mutes and which never interrupts or ducks the user's music.
final class ArrivalPolicy: @unchecked Sendable {
    /// At most one chime this often, however many requests arrive.
    static let coalesceMs: Int64 = 3000
    /// Request ids kept to tell a new request from one already announced.
    static let remembered = 256

    private static let never = Int64.min / 2
    private let clock: () -> Int64
    private let lock = NSLock()
    private var announced: [String] = []
    private var announcedSet: Set<String> = []
    private var lastAt = never

    init(clock: @escaping () -> Int64) { self.clock = clock }

    func decide(id: String, needsUser: Bool, settings: FeedbackSettings) -> ArrivalOutcome {
        lock.lock()
        defer { lock.unlock() }
        let fresh = announcedSet.insert(id).inserted
        if fresh {
            announced.append(id)
            while announced.count > Self.remembered { announcedSet.remove(announced.removeFirst()) }
        }
        guard needsUser, fresh else { return .nothing }
        let now = clock()
        guard now - lastAt >= Self.coalesceMs else { return .nothing }
        let outcome = ArrivalOutcome(
            chime: settings.allows(.requests) && settings.soundWhenOpen,
            haptic: settings.hapticsOn
        )
        if outcome != .nothing { lastAt = now }
        return outcome
    }
}
