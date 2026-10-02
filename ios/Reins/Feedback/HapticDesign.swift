import Foundation

/// The Android design's building blocks (`VibrationEffect.Composition` primitives), kept as the vocabulary so both
/// apps are designed from one table. Each becomes Core Haptics events in `HapticComposer`.
enum HapticPrimitive: Equatable {
    /// A crisp, full click.
    case click
    /// A lighter, sharper tick.
    case tick
    /// A soft, dull tick.
    case lowTick
    /// A low, heavy knock with a short body.
    case thud
    /// A short swell.
    case quickRise

    /// How long the primitive itself lasts, in seconds: the next step's delay counts from its end.
    var duration: Double {
        switch self {
        case .tick: 0.008
        case .click: 0.012
        case .lowTick: 0.010
        case .thud: 0.035
        case .quickRise: 0.060
        }
    }

    /// Core Haptics sharpness: what tells a tick from a thud.
    var sharpness: Float {
        switch self {
        case .tick: 0.9
        case .click: 0.7
        case .lowTick: 0.3
        case .thud: 0.08
        case .quickRise: 0.45
        }
    }

    /// Intensity at scale 1: a tick is lighter than a click at the same scale, as on Android.
    var weight: Float {
        switch self {
        case .tick: 0.8
        case .lowTick: 0.7
        case .click, .thud, .quickRise: 1
        }
    }
}

/// One primitive in a composition: `scale` 0...1 is the Standard strength, `delayMs` precedes it.
struct HapticStep: Equatable {
    var primitive: HapticPrimitive
    var scale: Float
    var delayMs: Int = 0

    init(_ primitive: HapticPrimitive, _ scale: Float, _ delayMs: Int = 0) {
        self.primitive = primitive
        self.scale = scale
        self.delayMs = delayMs
    }
}

/// One Core Haptics event before it becomes a `CHHapticEvent` (kept plain so the design is unit tested).
struct HapticNote: Equatable {
    enum Kind: Equatable { case transient, continuous }

    /// A point of a continuous note's intensity envelope: `at` is 0...1 of the note's length, `level` multiplies its
    /// intensity.
    struct Point: Equatable {
        var at: Double
        var level: Float
    }

    var kind: Kind
    /// Seconds from the start of the pattern.
    var time: Double
    /// Continuous notes only.
    var duration: Double = 0
    var intensity: Float
    var sharpness: Float
    var envelope: [Point] = []
}

/// What UIKit plays where Core Haptics cannot (no Taptic Engine API on this device).
enum HapticFallback: Equatable {
    case selection
    case impact(Impact, Float)
    case notification(Notice)

    enum Impact: Equatable { case light, medium, heavy, soft, rigid }
    enum Notice: Equatable { case success, warning, error }
}

/// How a `Haptic` is felt: the Android composition (its richest variant, which every Taptic Engine renders), plus
/// continuous textures Core Haptics can add on top (a swell under the surge, the decay of a pop, the fall of a
/// refusal), and a UIKit fallback.
struct HapticSpec {
    let haptic: Haptic
    let steps: [HapticStep]
    /// Continuous textures at Standard strength, laid under the steps.
    var underlay: [HapticNote] = []
    let fallback: HapticFallback
    /// 0 light (detent-class), 1 action, 2 event, 3 alert.
    let priority: Int
    /// The same haptic never repeats inside this many ms.
    let minGapMs: Int64
}

enum HapticTable {
    static func spec(_ haptic: Haptic) -> HapticSpec { specs[haptic] ?? build(haptic) }

    private static let specs: [Haptic: HapticSpec] = Dictionary(uniqueKeysWithValues: Haptic.allCases.map { ($0, build($0)) })

    static var all: [HapticSpec] { Haptic.allCases.map(spec) }

    /// The rising ticks of `.surge`: scale and gap per step, ending in the crack.
    static let surgeRamp: [(scale: Float, gap: Int)] = [(0.20, 0), (0.30, 60), (0.42, 50), (0.56, 40), (0.70, 32)]

    private static func fade(_ from: Float, _ to: Float) -> [HapticNote.Point] { [.init(at: 0, level: from), .init(at: 1, level: to)] }

    private static func build(_ haptic: Haptic) -> HapticSpec {
        switch haptic {
        // The faintest detent: slider steps.
        case .tick:
            HapticSpec(haptic: haptic, steps: [HapticStep(.lowTick, 0.5)], fallback: .selection, priority: 0, minGapMs: 70)
        // A choice was made: crisper than Tick.
        case .select:
            HapticSpec(haptic: haptic, steps: [HapticStep(.tick, 0.7)], fallback: .selection, priority: 0, minGapMs: 50)
        // Switches: a rising pair for on, a falling pair for off.
        case .toggleOn:
            HapticSpec(haptic: haptic, steps: [HapticStep(.lowTick, 0.5), HapticStep(.tick, 0.8, 20)],
                       fallback: .impact(.light, 0.8), priority: 1, minGapMs: 80)
        case .toggleOff:
            HapticSpec(haptic: haptic, steps: [HapticStep(.tick, 0.7), HapticStep(.lowTick, 0.4, 20)],
                       fallback: .impact(.soft, 0.7), priority: 1, minGapMs: 80)
        // A committed action: approve, save, refresh.
        case .confirm:
            HapticSpec(haptic: haptic, steps: [HapticStep(.click, 0.65)], fallback: .impact(.medium, 0.8), priority: 1, minGapMs: 120)
        // Soft rise, then a small settle: a permission given, an AI connected.
        case .success:
            HapticSpec(haptic: haptic, steps: [HapticStep(.quickRise, 0.45), HapticStep(.lowTick, 0.55, 40)],
                       fallback: .notification(.success), priority: 2, minGapMs: 400)
        // Two crisp taps: something needs you.
        case .attention:
            HapticSpec(haptic: haptic, steps: [HapticStep(.tick, 0.85), HapticStep(.tick, 0.85, 90)],
                       fallback: .notification(.warning), priority: 2, minGapMs: 400)
        // A short, heavy double: failed.
        case .error:
            HapticSpec(haptic: haptic, steps: [HapticStep(.click, 1), HapticStep(.thud, 0.8, 60)],
                       fallback: .notification(.error), priority: 3, minGapMs: 400)
        // Revoke, delete, disconnect: a low thud with a click on top.
        case .heavy:
            HapticSpec(haptic: haptic, steps: [HapticStep(.thud, 0.85), HapticStep(.click, 0.5, 30)],
                       fallback: .impact(.heavy, 1), priority: 2, minGapMs: 300)
        // A small pop that decays: Autopilot answered for you.
        case .pop:
            HapticSpec(haptic: haptic, steps: [HapticStep(.tick, 0.8), HapticStep(.lowTick, 0.35, 25)],
                       underlay: [HapticNote(kind: .continuous, time: 0.004, duration: 0.05, intensity: 0.3, sharpness: 0.5, envelope: fade(1, 0))],
                       fallback: .impact(.soft, 0.6), priority: 1, minGapMs: 120)
        // A refusal: firmer than Confirm, and falling where Confirm is one click. Not an error: saying no is a normal
        // answer.
        case .deny:
            HapticSpec(haptic: haptic, steps: [HapticStep(.click, 0.85), HapticStep(.lowTick, 0.55, 40)],
                       underlay: [HapticNote(kind: .continuous, time: 0.010, duration: 0.07, intensity: 0.28, sharpness: 0.2, envelope: fade(1, 0))],
                       fallback: .impact(.rigid, 0.9), priority: 1, minGapMs: 200)
        // Bypass on: five ticks that firm up and bunch together over a swell, then a hard crack with a thud under it,
        // about 250 ms.
        case .surge:
            HapticSpec(haptic: haptic, steps: surge(finish: HapticStep(.click, 0.9, 24), closer: HapticStep(.thud, 1, 8)),
                       underlay: [HapticNote(kind: .continuous, time: 0, duration: 0.24, intensity: 0.4, sharpness: 0.25,
                                             envelope: [.init(at: 0, level: 0.1), .init(at: 0.7, level: 0.5), .init(at: 1, level: 1)])],
                       fallback: .impact(.heavy, 1), priority: 2, minGapMs: 500)
        // Bypass off: a very short, sharp double tick.
        case .zip:
            HapticSpec(haptic: haptic, steps: [HapticStep(.tick, 0.9), HapticStep(.tick, 0.9, 28)],
                       fallback: .impact(.rigid, 0.9), priority: 1, minGapMs: 100)
        // Autopilot up: an irregular crackle of micro-pulses (uneven strength and gaps) over a ragged buzz, ending in a
        // firm crack, about 180 ms.
        case .lightning:
            HapticSpec(
                haptic: haptic,
                steps: [HapticStep(.tick, 0.45), HapticStep(.tick, 0.80, 31), HapticStep(.lowTick, 0.35, 17),
                        HapticStep(.tick, 0.90, 44), HapticStep(.click, 1, 52)],
                underlay: [HapticNote(kind: .continuous, time: 0, duration: 0.16, intensity: 0.2, sharpness: 0.95,
                                      envelope: [.init(at: 0, level: 0.6), .init(at: 0.2, level: 0.1), .init(at: 0.35, level: 0.9),
                                                 .init(at: 0.55, level: 0.2), .init(at: 0.85, level: 1), .init(at: 1, level: 0)])],
                fallback: .impact(.heavy, 0.9), priority: 2, minGapMs: 400
            )
        }
    }

    private static func surge(finish: HapticStep, closer: HapticStep) -> [HapticStep] {
        surgeRamp.map { HapticStep(.tick, $0.scale, $0.gap) } + [finish, closer]
    }
}

/// Turns a spec into the notes of one Core Haptics pattern at a strength.
enum HapticComposer {
    /// Intensity at `strength`, never imperceptible and never above full.
    static func scaled(_ scale: Float, _ strength: HapticStrength) -> Float { min(max(scale * strength.scale, 0.05), 1) }

    static func notes(_ haptic: Haptic, _ strength: HapticStrength) -> [HapticNote] {
        let spec = HapticTable.spec(haptic)
        var out: [HapticNote] = []
        var t = 0.0
        for step in spec.steps {
            t += Double(step.delayMs) / 1000
            let p = step.primitive
            let intensity = scaled(step.scale * p.weight, strength)
            switch p {
            case .click, .tick, .lowTick:
                out.append(HapticNote(kind: .transient, time: t, intensity: intensity, sharpness: p.sharpness))
            case .thud:
                // A dull knock, then its body dying away.
                out.append(HapticNote(kind: .transient, time: t, intensity: intensity, sharpness: p.sharpness))
                out.append(HapticNote(kind: .continuous, time: t, duration: p.duration, intensity: scaled(step.scale * 0.6, strength),
                                      sharpness: 0.05, envelope: [.init(at: 0, level: 1), .init(at: 1, level: 0)]))
            case .quickRise:
                out.append(HapticNote(kind: .continuous, time: t, duration: p.duration, intensity: intensity, sharpness: p.sharpness,
                                      envelope: [.init(at: 0, level: 0.15), .init(at: 1, level: 1)]))
            }
            t += p.duration
        }
        for var note in spec.underlay {
            note.intensity = scaled(note.intensity, strength)
            out.append(note)
        }
        return out.sorted { $0.time < $1.time }
    }

    /// Seconds from the first event to the end of the last one.
    static func length(_ notes: [HapticNote]) -> Double {
        notes.map { $0.time + ($0.kind == .continuous ? $0.duration : 0.01) }.max() ?? 0
    }
}
