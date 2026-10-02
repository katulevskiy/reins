import os
import UIKit

/// What the Sounds & haptics screen needs beyond `Feedback`: the settings, auditioning a moment, and what this device's
/// haptics can do.
protocol FeedbackPreviewing: AnyObject {
    var store: FeedbackStore { get }
    /// The Settings page auditioning a moment: ignores rate limits and category switches, nothing else.
    func preview(haptic: Haptic?, cue: Cue?, step: Int)
    /// How this device renders haptics, in a few words for the Settings page.
    var hapticTier: String { get }
}

extension FeedbackPreviewing {
    func preview(_ event: FeedbackEvent, step: Int = 0) { preview(haptic: event.haptic, cue: event.cue, step: step) }
}

/// Haptics and sound for the whole app (the Android app's `AndroidFeedback`). `FeedbackGate` decides when something
/// plays, `HapticComposer` how a haptic is felt, `CueTable` which sound and how loud, and `FeedbackEvent` what each of
/// Reins's moments is made of. Nothing plays while the app is in the background: the events that matter there arrive
/// as notifications, which sound the same chimes.
///
/// Debug builds log one line per request (subsystem `dev.rewarden.ios`, category `feedback`): what played or why not.
final class FeedbackEngine: Feedback, FeedbackPreviewing, @unchecked Sendable {
    let store: FeedbackStore
    private let env: AppEnvironment
    private let gate: FeedbackGate
    private let claims: ClaimTracker
    private let haptics = HapticPlayer()
    private let sounds: SoundPlayer
    private let log = Logger(subsystem: "dev.rewarden.ios", category: "feedback")
    private var observers: [NSObjectProtocol] = []

    @MainActor
    init(store: FeedbackStore = .shared, clock: @escaping () -> Int64 = feedbackClock) {
        self.store = store
        env = AppEnvironment()
        gate = FeedbackGate(settings: { store.current }, env: env, clock: clock)
        claims = ClaimTracker(clock: clock)
        sounds = SoundPlayer(clock: clock)
        observeLifecycle()
        if store.current.hapticsOn { haptics.prewarm(strength: store.current.strength) }
    }

    // MARK: Feedback

    func haptic(_ haptic: Haptic) {
        play(haptic, preview: false)
    }

    func cue(_ cue: Cue, step: Int) {
        claims.claimCue()
        play(cue, step: step, preview: false)
    }

    func cueUnlessRecent(_ cue: Cue) {
        if claims.cueWithin(ClaimTracker.recentMs) {
            debug("cue \(cue) skip: another cue just played")
        } else {
            self.cue(cue, step: 0)
        }
    }

    /// A choice was made: the close that follows stays silent.
    func quietClose() { claims.quiet() }

    // MARK: FeedbackPreviewing

    func preview(haptic: Haptic?, cue: Cue?, step: Int) {
        // The sound first: it is the channel perceived late.
        if let cue {
            claims.claimCue()
            play(cue, step: step, preview: true)
        }
        if let haptic { play(haptic, preview: true) }
    }

    var hapticTier: String {
        if HapticPlayer.supportsCoreHaptics { return "Taptic Engine: precise clicks and ticks" }
        if UIDevice.current.userInterfaceIdiom == .phone { return "Basic haptics" }
        return "This device has no haptics"
    }

    // MARK: Playing

    private func play(_ haptic: Haptic, preview: Bool) {
        switch gate.haptic(haptic, preview: preview) {
        case let .skip(why):
            debug("haptic \(haptic) skip: \(why.rawValue)")
        case .play:
            let strength = store.current.strength
            haptics.play(haptic, strength: strength)
            debug("haptic \(haptic) play \(strength.rawValue)")
        }
    }

    private func play(_ cue: Cue, step: Int, preview: Bool) {
        switch gate.cue(cue, preview: preview) {
        case let .skip(why):
            debug("cue \(cue) skip: \(why.rawValue)")
        case .play:
            let spec = CueTable.spec(cue)
            let volume = CueTable.volume(spec, userGain: store.current.gain)
            let rate: Float = cue == .detent ? DetentLadder.rate(step) : 1
            #if DEBUG
            let line = "cue \(cue) vol=\(String(format: "%.2f", volume)) rate=\(String(format: "%.2f", rate))\(cue == .detent ? " step=\(step)" : "")"
            sounds.play(cue, volume: volume, rate: rate, priority: spec.priority) { [log] ok in
                log.debug("\(line, privacy: .public) \(ok ? "play" : "skip: \(Skipped.notLoaded.rawValue)", privacy: .public)")
            }
            #else
            sounds.play(cue, volume: volume, rate: rate, priority: spec.priority)
            #endif
        }
    }

    private func debug(_ message: @autoclosure () -> String) {
        #if DEBUG
        let text = message()
        log.debug("\(text, privacy: .public)")
        #endif
    }

    // MARK: Lifecycle

    @MainActor
    private func observeLifecycle() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.foreground(true) }
        })
        observers.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.foreground(false) }
        })
    }

    @MainActor
    private func foreground(_ on: Bool) {
        env.foreground = on
        if on {
            TouchDownRecognizer.install { [weak self] in
                guard let self, self.env.foreground, self.store.current.soundsOn else { return }
                self.sounds.touchDown()
            }
            if store.current.hapticsOn { haptics.prewarm(strength: store.current.strength) }
        } else {
            sounds.suspend()
            haptics.suspend()
        }
        debug("foreground=\(on)")
    }
}

/// The real `FeedbackEnvironment`: whether the app is in front (kept by the lifecycle notifications, so reading it
/// costs nothing), and whether this device has haptics at all.
final class AppEnvironment: FeedbackEnvironment, @unchecked Sendable {
    private let lock = NSLock()
    private var _foreground: Bool

    @MainActor
    init() {
        _foreground = UIApplication.shared.applicationState != .background
    }

    var foreground: Bool {
        get { lock.withLock { _foreground } }
        set { lock.withLock { _foreground = newValue } }
    }

    var active: Bool { foreground }

    /// A Taptic Engine, or an iPhone whose UIKit generators stand in for Core Haptics.
    let hasHaptics: Bool = HapticPlayer.supportsCoreHaptics || UIDevice.current.userInterfaceIdiom == .phone
}

/// Sees every finger that goes down in a window without taking part in any gesture: the sound output wakes on the
/// touch-down, so it is running by the time the tap's sound is asked for (the Android app's `onTouchDown`).
final class TouchDownRecognizer: UIGestureRecognizer {
    private var onDown: () -> Void = {}

    @MainActor
    static func install(_ onDown: @escaping () -> Void) {
        for scene in UIApplication.shared.connectedScenes {
            guard let scene = scene as? UIWindowScene else { continue }
            for window in scene.windows {
                if let existing = window.gestureRecognizers?.compactMap({ $0 as? TouchDownRecognizer }).first {
                    existing.onDown = onDown
                    continue
                }
                let recognizer = TouchDownRecognizer(target: nil, action: nil)
                recognizer.onDown = onDown
                recognizer.cancelsTouchesInView = false
                recognizer.delaysTouchesBegan = false
                recognizer.delaysTouchesEnded = false
                window.addGestureRecognizer(recognizer)
            }
        }
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        onDown()
        state = .failed
    }

    override func canPrevent(_ preventedGestureRecognizer: UIGestureRecognizer) -> Bool { false }
    override func canBePrevented(by preventingGestureRecognizer: UIGestureRecognizer) -> Bool { false }
}
