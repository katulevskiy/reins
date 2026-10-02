import Foundation

/// What the device is doing right now. Read per event, so implementations answer from cached values: the gate sits on
/// the tap-to-sound path. It deliberately knows nothing of the phone's own sound settings: the in-app switches decide,
/// and the audio session (`.ambient`) lets the ring/silent switch and the volume buttons shape what is heard.
protocol FeedbackEnvironment: AnyObject {
    /// The app is in the foreground and active.
    var active: Bool { get }
    /// This device can play haptics at all (a Taptic Engine; iPads have none).
    var hasHaptics: Bool { get }
}

/// Why a haptic or cue did not play. Logged by debug builds.
enum Skipped: String {
    case masterOff = "sounds & haptics master off"
    case hapticsOff = "haptics switch off"
    case noHaptics = "no Taptic Engine"
    case inactive = "app not active"
    case soundsOff = "sounds switch off"
    case categoryOff = "category switch off"
    case rate = "rate limited"
    case voices = "too many voices"
    case notLoaded = "sound not loaded"
}

enum Decision: Equatable {
    case play
    case skip(Skipped)

    var skipped: Skipped? {
        if case let .skip(why) = self { return why }
        return nil
    }
}

/// Decides whether a haptic or cue plays: the in-app switches, whether the app is active, and rate limiting (the
/// Android app's `FeedbackGate`). Pure and clock-injected so its rules are unit tested; the engine acts on the result.
///
/// Rules, in order:
///  - The Sounds & haptics master (`FeedbackSettings.master`) is absolute: off, nothing plays.
///  - Haptics: the Haptics switch, a Taptic Engine, an active app.
///  - Sounds: the Sounds switch and the category switch, an active app.
///  - Rate: the same haptic / cue never repeats inside its minimum gap, any two light events keep a short global gap,
///    a burst of light haptics is capped per second, and sounding voices are capped.
final class FeedbackGate: @unchecked Sendable {
    static let globalHapticGapMs: Int64 = 25
    static let globalCueGapMs: Int64 = 20
    static let windowMs: Int64 = 1000
    static let lightHapticsPerWindow = 12
    static let maxVoices = 4

    private let settings: () -> FeedbackSettings
    private let env: FeedbackEnvironment
    private let clock: () -> Int64
    private let lock = NSLock()

    private static let never = Int64.min / 2
    private var lastHaptic: [Haptic: Int64] = [:]
    private var lastAnyHaptic = never
    private var lastAnyHapticPriority = 0
    private var lightHaptics: [Int64] = []

    private var lastCue: [Cue: Int64] = [:]
    private var lastAnyCue = never
    private var lastAnyCuePriority = 0
    /// (end time, priority) of every voice still sounding.
    private var voices: [(Int64, Int)] = []

    init(settings: @escaping () -> FeedbackSettings, env: FeedbackEnvironment, clock: @escaping () -> Int64) {
        self.settings = settings
        self.env = env
        self.clock = clock
    }

    /// `preview`: the Settings page auditioning a moment; the rate limits do not apply (a tap on a row is intent).
    func haptic(_ haptic: Haptic, preview: Bool = false) -> Decision {
        lock.lock()
        defer { lock.unlock() }
        let s = settings()
        if !s.master { return .skip(.masterOff) }
        if !s.haptics { return .skip(.hapticsOff) }
        if !env.hasHaptics { return .skip(.noHaptics) }
        if !env.active { return .skip(.inactive) }
        if preview { return .play }
        let spec = HapticTable.spec(haptic)
        let now = clock()
        if now - (lastHaptic[haptic] ?? Self.never) < spec.minGapMs { return .skip(.rate) }
        if spec.priority <= lastAnyHapticPriority && now - lastAnyHaptic < Self.globalHapticGapMs { return .skip(.rate) }
        if spec.priority == 0 {
            lightHaptics.removeAll { now - $0 > Self.windowMs }
            if lightHaptics.count >= Self.lightHapticsPerWindow { return .skip(.rate) }
            lightHaptics.append(now)
        }
        lastHaptic[haptic] = now
        lastAnyHaptic = now
        lastAnyHapticPriority = spec.priority
        return .play
    }

    /// `preview`: as for `haptic`, and the category switches do not apply (hearing a muted category is the point).
    func cue(_ cue: Cue, preview: Bool = false) -> Decision {
        lock.lock()
        defer { lock.unlock() }
        let s = settings()
        let spec = CueTable.spec(cue)
        if !s.master { return .skip(.masterOff) }
        if !s.sounds { return .skip(.soundsOff) }
        if !preview && !s.allows(spec.category) { return .skip(.categoryOff) }
        if !env.active { return .skip(.inactive) }
        if preview { return .play }
        let now = clock()
        if now - (lastCue[cue] ?? Self.never) < spec.minGapMs { return .skip(.rate) }
        if spec.priority <= lastAnyCuePriority && now - lastAnyCue < Self.globalCueGapMs { return .skip(.rate) }
        voices.removeAll { $0.0 <= now }
        if voices.count >= Self.maxVoices, let lowest = voices.map({ $0.1 }).min(), lowest >= spec.priority { return .skip(.voices) }
        voices.append((now + spec.approxMs, spec.priority))
        lastCue[cue] = now
        lastAnyCue = now
        lastAnyCuePriority = spec.priority
        return .play
    }
}

/// Remembers when the last cue sounded, so a sheet or dialog closing right after the action it hosted stays quiet
/// (the Android app's `ClaimTracker`, without the default-tap half: SwiftUI controls ask for their feedback
/// explicitly).
final class ClaimTracker: @unchecked Sendable {
    private static let never = Int64.min / 2
    private let clock: () -> Int64
    private let lock = NSLock()
    private var cueAt = never
    private var quietAt = never

    /// The window `Feedback.cueUnlessRecent` uses.
    static let recentMs: Int64 = 250

    init(clock: @escaping () -> Int64) { self.clock = clock }

    func claimCue() {
        lock.lock()
        cueAt = clock()
        lock.unlock()
    }

    /// A choice was made in a dialog: the dialog closing right after is the choice's doing, not a sound of its own.
    func quiet() {
        lock.lock()
        quietAt = clock()
        lock.unlock()
    }

    /// Was any cue requested (or a close silenced) in the last `windowMs`?
    func cueWithin(_ windowMs: Int64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return clock() - max(cueAt, quietAt) < windowMs
    }
}

/// When the sound output is kept awake. The audio engine's output idles after the last sound, and waking it costs tens
/// of milliseconds: the "slightly late" first sound after a pause. The engine keeps running while the user is touching
/// the app (a touch-down starts it, so it is up before the tap that sounds) and for `idleMs` after the last touch or
/// cue, then stops so it costs no power.
final class WarmPolicy: @unchecked Sendable {
    static let idleMs: Int64 = 12_000

    private let clock: () -> Int64
    private let idleMs: Int64
    private let lock = NSLock()
    private var until = Int64.min / 2

    init(clock: @escaping () -> Int64, idleMs: Int64 = WarmPolicy.idleMs) {
        self.clock = clock
        self.idleMs = idleMs
    }

    /// Something happened that may soon want a sound (a touch-down, a cue).
    func touch() {
        lock.lock()
        until = clock() + idleMs
        lock.unlock()
    }

    var wanted: Bool {
        lock.lock()
        defer { lock.unlock() }
        return clock() < until
    }

    /// Milliseconds until `wanted` turns false (0 when it already is).
    var remainingMs: Int64 {
        lock.lock()
        defer { lock.unlock() }
        return max(until - clock(), 0)
    }

    func stop() {
        lock.lock()
        until = Int64.min / 2
        lock.unlock()
    }
}

/// Which player voice a new cue takes: a free one, or, when all are sounding, the least important (oldest among
/// equals) as long as it matters no more than the newcomer. The gate already turned away cues that could not win.
struct VoicePool {
    struct Voice: Equatable {
        var endsAt: Int64
        var priority: Int
        var startedAt: Int64
    }

    private(set) var voices: [Voice]

    init(count: Int) {
        voices = Array(repeating: Voice(endsAt: Int64.min / 2, priority: 0, startedAt: Int64.min / 2), count: count)
    }

    /// The voice to use (and claims it), or nil when every voice plays something more important.
    mutating func claim(now: Int64, priority: Int, lengthMs: Int64) -> Int? {
        let pick = voices.indices.first { voices[$0].endsAt <= now }
            ?? voices.indices
            .filter { voices[$0].priority <= priority }
            .min { (voices[$0].priority, voices[$0].startedAt) < (voices[$1].priority, voices[$1].startedAt) }
        guard let index = pick else { return nil }
        voices[index] = Voice(endsAt: now + lengthMs, priority: priority, startedAt: now)
        return index
    }

    /// Everything stopped (the engine was interrupted or reset).
    mutating func reset() {
        for i in voices.indices { voices[i].endsAt = Int64.min / 2 }
    }
}

/// Milliseconds on a monotonic clock (uptime), for the gate and the policies.
func feedbackClock() -> Int64 {
    Int64(DispatchTime.now().uptimeNanoseconds / 1_000_000)
}
