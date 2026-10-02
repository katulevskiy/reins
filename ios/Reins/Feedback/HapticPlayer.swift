import CoreHaptics
import os
import UIKit

/// Plays haptics through one running `CHHapticEngine`: one pattern per `Haptic` (`HapticComposer`), its player built
/// once per strength and reused, so a detent tick during a slider drag costs a `start` call. Where Core Haptics is not
/// supported the UIKit feedback generators play the spec's fallback.
///
/// Everything runs on one serial queue at user-interactive priority: starting the engine can take tens of ms, and the
/// caller (usually the main thread, mid-gesture) must never wait for it.
final class HapticPlayer: @unchecked Sendable {
    static let supportsCoreHaptics = CHHapticEngine.capabilitiesForHardware().supportsHaptics

    private let queue = DispatchQueue(label: "com.reins2fa.app.haptics", qos: .userInteractive)
    private let log = Logger(subsystem: "com.reins2fa.app", category: "feedback")
    private var engine: CHHapticEngine?
    private var running = false
    private var players: [PlayerKey: CHHapticPatternPlayer] = [:]

    private struct PlayerKey: Hashable {
        let haptic: Haptic
        let strength: HapticStrength
    }

    /// Creates and starts the engine ahead of the first haptic.
    func prewarm(strength: HapticStrength) {
        guard Self.supportsCoreHaptics else { return }
        queue.async {
            self.startIfNeeded()
            // The lightest, most frequent patterns are ready before the first slider drag.
            for haptic in [Haptic.tick, .select, .toggleOn, .toggleOff] { _ = self.player(haptic, strength) }
        }
    }

    /// The app went to the background: the system stops the engine anyway; letting go of it frees the Taptic Engine.
    func suspend() {
        guard Self.supportsCoreHaptics else { return }
        queue.async {
            self.engine?.stop()
            self.running = false
        }
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
        guard let engine, !running else { return }
        do {
            try engine.start()
            running = true
        } catch {
            log.error("haptic engine did not start: \(error.localizedDescription)")
        }
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

    static func pattern(_ notes: [HapticNote]) throws -> CHHapticPattern {
        var events: [CHHapticEvent] = []
        var curves: [CHHapticParameterCurve] = []
        for note in notes {
            let params = [
                CHHapticEventParameter(parameterID: .hapticIntensity, value: note.intensity),
                CHHapticEventParameter(parameterID: .hapticSharpness, value: note.sharpness),
            ]
            switch note.kind {
            case .transient:
                events.append(CHHapticEvent(eventType: .hapticTransient, parameters: params, relativeTime: note.time))
            case .continuous:
                events.append(CHHapticEvent(eventType: .hapticContinuous, parameters: params, relativeTime: note.time, duration: note.duration))
                if note.envelope.count >= 2 {
                    let points = note.envelope.map {
                        CHHapticParameterCurve.ControlPoint(relativeTime: $0.at * note.duration, value: $0.level)
                    }
                    curves.append(CHHapticParameterCurve(parameterID: .hapticIntensityControl, controlPoints: points, relativeTime: note.time))
                }
            }
        }
        return try CHHapticPattern(events: events, parameterCurves: curves)
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
