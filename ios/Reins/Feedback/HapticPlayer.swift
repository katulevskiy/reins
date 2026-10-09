import CoreHaptics
import os
import UIKit

/// Plays haptics through one `CHHapticEngine`: one pattern per `Haptic` (`HapticComposer`), its player built once per
/// strength and reused, so a detent tick during a slider drag costs a `start` call. Where Core Haptics is not supported
/// the UIKit feedback generators play the spec's fallback.
///
/// The engine is kept awake like the sound output (`WarmPolicy`): a touch-down starts it, so it is running by the time
/// the tap's haptic is asked for, and it stops after a quiet spell so the Taptic Engine costs no power while nobody
/// touches the app.
///
/// Everything runs on one serial queue at user-interactive priority: starting the engine can take tens of ms, and the
/// caller (usually the main thread, mid-gesture) must never wait for it.
final class HapticPlayer: @unchecked Sendable {
    static let supportsCoreHaptics = CHHapticEngine.capabilitiesForHardware().supportsHaptics

    private let queue = DispatchQueue(label: "com.reins2fa.app.haptics", qos: .userInteractive)
    private let log = Logger(subsystem: "com.reins2fa.app", category: "feedback")
    private let warm: WarmPolicy
    private var engine: CHHapticEngine?
    private var running = false
    private var players: [PlayerKey: CHHapticPatternPlayer] = [:]
    private var idleCheck: DispatchWorkItem?

    private struct PlayerKey: Hashable {
        let haptic: Haptic
        let strength: HapticStrength
    }

    init(clock: @escaping () -> Int64 = feedbackClock) {
        warm = WarmPolicy(clock: clock)
    }

    /// Creates and starts the engine ahead of the first haptic.
    func prewarm(strength: HapticStrength) {
        guard Self.supportsCoreHaptics else { return }
        warm.touch()
        queue.async {
            self.startIfNeeded()
            // The lightest, most frequent patterns are ready before the first slider drag.
            for haptic in [Haptic.tick, .select, .toggleOn, .toggleOff] { _ = self.player(haptic, strength) }
        }
    }

    /// A finger went down somewhere in the app: wake the engine now so it is running by the time the tap is felt.
    func touchDown() {
        guard Self.supportsCoreHaptics else { return }
        warm.touch()
        queue.async {
            guard !self.running else { return }
            self.startIfNeeded()
        }
    }

    /// The app went to the background: the system stops the engine anyway; letting go of it frees the Taptic Engine.
    func suspend() {
        guard Self.supportsCoreHaptics else { return }
        warm.stop()
        queue.async { self.stopEngine() }
    }

    func play(_ haptic: Haptic, strength: HapticStrength) {
        if Self.supportsCoreHaptics {
            queue.async { self.playPattern(haptic, strength) }
        } else {
            Task { @MainActor in Self.playFallback(HapticTable.spec(haptic).fallback, strength) }
        }
    }

    // MARK: Core Haptics

    private func makeEngine() -> CHHapticEngine? {
        do {
            let engine = try CHHapticEngine()
            // Haptics only: no audio graph to bring up, the shortest path to the actuator.
            engine.playsHapticsOnly = true
            // Stopped by `WarmPolicy` instead: the system's own shutdown would leave the next tap to wake it.
            engine.isAutoShutdownEnabled = false
            engine.stoppedHandler = { [weak self] reason in
                guard let self else { return }
                self.queue.async {
                    self.running = false
                    self.log.debug("haptic engine stopped: \(reason.rawValue)")
                }
            }
            // The haptic server restarted: every player made before is dead; the engine must be started again.
            engine.resetHandler = { [weak self] in
                guard let self else { return }
                self.queue.async {
                    self.players.removeAll()
                    self.running = false
                    self.startIfNeeded()
                }
            }
            return engine
        } catch {
            log.error("haptic engine unavailable: \(error.localizedDescription)")
            return nil
        }
    }

    private func startIfNeeded() {
        if engine == nil { engine = makeEngine() }
        guard let engine else { return }
        if !running {
            do {
                try engine.start()
                running = true
            } catch {
                log.error("haptic engine did not start: \(error.localizedDescription)")
                return
            }
        }
        scheduleIdleCheck()
    }

    private func stopEngine() {
        idleCheck?.cancel()
        idleCheck = nil
        guard running else { return }
        // Its players stay valid: they play again once it is started.
        engine?.stop()
        running = false
    }

    /// Stops the engine once nothing has wanted a haptic for `WarmPolicy.idleMs`.
    private func scheduleIdleCheck() {
        guard idleCheck == nil else { return }
        let item = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.idleCheck = nil
            if self.warm.wanted {
                self.scheduleIdleCheck()
            } else {
                self.stopEngine()
            }
        }
        idleCheck = item
        queue.asyncAfter(deadline: .now() + .milliseconds(Int(warm.remainingMs) + 50), execute: item)
    }

    private func player(_ haptic: Haptic, _ strength: HapticStrength) -> CHHapticPatternPlayer? {
        let key = PlayerKey(haptic: haptic, strength: strength)
        if let player = players[key] { return player }
        guard let engine else { return nil }
        do {
            let player = try engine.makePlayer(with: Self.pattern(HapticComposer.notes(haptic, strength)))
            players[key] = player
            return player
        } catch {
            log.error("haptic \(String(describing: haptic)) has no pattern: \(error.localizedDescription)")
            return nil
        }
    }

    private func playPattern(_ haptic: Haptic, _ strength: HapticStrength) {
        warm.touch()
        startIfNeeded()
        guard running, let player = player(haptic, strength) else { return }
        do {
            try player.start(atTime: CHHapticTimeImmediate)
        } catch {
            // A stale player (the engine was reset under it): rebuild once.
            players[PlayerKey(haptic: haptic, strength: strength)] = nil
            running = false
            startIfNeeded()
            try? self.player(haptic, strength)?.start(atTime: CHHapticTimeImmediate)
        }
    }

    /// One Core Haptics event before it is made (kept plain so the layout is unit tested).
    struct Event: Equatable {
        var kind: HapticNote.Kind
        var time: Double
        var duration: Double = 0
        var intensity: Float
        var sharpness: Float
    }

    /// How long each step of a shaped texture lasts.
    static let textureStep = 0.01

    /// The events of a pattern. A texture whose strength or sharpness changes along it is played as short steps, each
    /// with its own intensity and sharpness, not shaped with a control curve: a curve scales every event of the pattern
    /// while it runs and keeps its last value after, so the clicks over and after a texture lost most of their
    /// strength (Heavy's click and Lightning's final crack were silent).
    static func events(_ notes: [HapticNote]) -> [Event] {
        var out: [Event] = []
        for note in notes {
            switch note.kind {
            case .transient:
                out.append(Event(kind: .transient, time: note.time, intensity: note.intensity, sharpness: note.sharpness))
            case .continuous:
                let shaped = note.envelope.count >= 2
                let sharpening = note.sharpnessPath.count >= 2
                guard shaped || sharpening else {
                    out.append(Event(kind: .continuous, time: note.time, duration: note.duration, intensity: note.intensity, sharpness: note.sharpness))
                    continue
                }
                let count = max(2, Int((note.duration / textureStep).rounded(.up)))
                let length = note.duration / Double(count)
                for i in 0..<count {
                    // Each step takes the shape's value at its middle.
                    let at = (Double(i) + 0.5) / Double(count)
                    let intensity = note.intensity * (shaped ? HapticNote.value(note.envelope, at: at) : 1)
                    guard intensity > 0 else { continue }
                    out.append(Event(
                        kind: .continuous,
                        time: note.time + Double(i) * length,
                        duration: length,
                        intensity: intensity,
                        sharpness: sharpening ? HapticNote.value(note.sharpnessPath, at: at) : note.sharpness
                    ))
                }
            }
        }
        return out
    }

    static func pattern(_ notes: [HapticNote]) throws -> CHHapticPattern {
        let events = events(notes).map { e in
            let params = [
                CHHapticEventParameter(parameterID: .hapticIntensity, value: e.intensity),
                CHHapticEventParameter(parameterID: .hapticSharpness, value: e.sharpness),
            ]
            return switch e.kind {
            case .transient: CHHapticEvent(eventType: .hapticTransient, parameters: params, relativeTime: e.time)
            case .continuous: CHHapticEvent(eventType: .hapticContinuous, parameters: params, relativeTime: e.time, duration: e.duration)
            }
        }
        return try CHHapticPattern(events: events, parameters: [])
    }

    // MARK: UIKit fallback

    @MainActor
    private static func playFallback(_ fallback: HapticFallback, _ strength: HapticStrength) {
        switch fallback {
        case .selection:
            UISelectionFeedbackGenerator().selectionChanged()
        case let .impact(kind, intensity):
            let style: UIImpactFeedbackGenerator.FeedbackStyle = switch kind {
            case .light: .light
            case .medium: .medium
            case .heavy: .heavy
            case .soft: .soft
            case .rigid: .rigid
            }
            UIImpactFeedbackGenerator(style: style).impactOccurred(intensity: CGFloat(min(intensity * strength.scale, 1)))
        case let .notification(notice):
            let type: UINotificationFeedbackGenerator.FeedbackType = switch notice {
            case .success: .success
            case .warning: .warning
            case .error: .error
            }
            UINotificationFeedbackGenerator().notificationOccurred(type)
        }
    }
}
