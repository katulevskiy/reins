import CoreHaptics
import XCTest
@testable import Reins

/// The haptic table and how it becomes Core Haptics patterns (the Android app's `HapticDesignTest`).
final class HapticDesignTests: XCTestCase {
    private func steps(_ h: Haptic) -> [HapticStep] { HapticTable.spec(h).steps }
    private func notes(_ h: Haptic, _ s: HapticStrength = .standard) -> [HapticNote] { HapticComposer.notes(h, s) }
    private func sum(_ steps: [HapticStep]) -> Float { steps.map(\.scale).reduce(0, +) }
    /// Delays plus about ten milliseconds per primitive, as the Android test counts.
    private func duration(_ steps: [HapticStep]) -> Int { steps.map(\.delayMs).reduce(0, +) + steps.count * 10 }

    func testEveryHapticHasASpec() {
        XCTAssertEqual(HapticTable.all.count, Haptic.allCases.count)
        for h in Haptic.allCases {
            let s = HapticTable.spec(h)
            XCTAssertEqual(s.haptic, h)
            XCTAssertFalse(s.steps.isEmpty, "\(h) has steps")
            XCTAssertGreaterThan(s.minGapMs, 0, "\(h) gap")
        }
    }

    func testEveryHapticBecomesAValidPatternAtEveryStrength() throws {
        for h in Haptic.allCases {
            for strength in HapticStrength.allCases {
                let n = notes(h, strength)
                XCTAssertFalse(n.isEmpty)
                for note in n {
                    XCTAssertTrue((0.05...1).contains(note.intensity), "\(h) \(strength) intensity \(note.intensity)")
                    XCTAssertTrue((0...1).contains(note.sharpness), "\(h) sharpness")
                    XCTAssertGreaterThanOrEqual(note.time, 0)
                    if note.kind == .continuous { XCTAssertGreaterThan(note.duration, 0) }
                    for p in note.envelope { XCTAssertTrue((0...1).contains(p.at) && (0...1).contains(p.level)) }
                }
                XCTAssertEqual(n.map(\.time), n.map(\.time).sorted(), "\(h) notes in time order")
                XCTAssertNoThrow(try HapticPlayer.pattern(n), "\(h) \(strength)")
            }
        }
    }

    func testToggleOnRisesAndToggleOffFalls() {
        let on = steps(.toggleOn)
        let off = steps(.toggleOff)
        XCTAssertGreaterThan(on.last!.scale, on.first!.scale)
        XCTAssertLessThan(off.last!.scale, off.first!.scale)
        XCTAssertNotEqual(notes(.toggleOn), notes(.toggleOff))
    }

    func testCompoundPatternsHaveSeveralBeats() {
        for h in [Haptic.success, .attention, .heavy, .deny, .error] {
            XCTAssertGreaterThanOrEqual(steps(h).count, 2, "\(h)")
        }
    }

    func testDenyIsAFirmClickThatFallsAwayFirmerThanApproveLighterThanAnError() {
        let deny = steps(.deny)
        XCTAssertEqual(deny.first?.primitive, .click)
        XCTAssertLessThan(deny.last!.scale, deny.first!.scale)
        XCTAssertGreaterThan(deny.first!.scale, steps(.confirm).first!.scale)
        XCTAssertLessThan(sum(deny), sum(steps(.error)))
        XCTAssertLessThan(HapticTable.spec(.deny).priority, HapticTable.spec(.error).priority)
        // Its tail fades out.
        let tail = notes(.deny).first { $0.kind == .continuous }
        XCTAssertEqual(tail?.envelope.last?.level, 0)
    }

    func testStrengthScalesIntensity() {
        func first(_ s: HapticStrength) -> Float { notes(.select, s).first!.intensity }
        XCTAssertLessThan(first(.subtle), first(.standard))
        XCTAssertLessThan(first(.standard), first(.strong))
        XCTAssertEqual(HapticComposer.scaled(0.7, .subtle), 0.35, accuracy: 1e-6)
        XCTAssertEqual(HapticComposer.scaled(0.9, .strong), 1)
        XCTAssertEqual(HapticComposer.scaled(0.01, .subtle), 0.05)
    }

    func testPrioritiesOrderLightActionAndAlert() {
        XCTAssertLessThan(HapticTable.spec(.tick).priority, HapticTable.spec(.confirm).priority)
        XCTAssertLessThan(HapticTable.spec(.confirm).priority, HapticTable.spec(.error).priority)
    }

    // MARK: The Autopilot haptics

    func testSurgeIsAQuarterSecondCrescendoEndingInAHardCrack() {
        let s = steps(.surge)
        let ramp = Array(s.dropLast(2))
        XCTAssertGreaterThanOrEqual(ramp.count, 5)
        XCTAssertTrue(ramp.allSatisfy { $0.primitive == .tick })
        XCTAssertEqual(ramp.map(\.scale), ramp.map(\.scale).sorted(), "firms up")
        let gaps = ramp.dropFirst().map(\.delayMs)
        XCTAssertEqual(gaps, gaps.sorted(by: >), "and bunches together")
        XCTAssertEqual(s.last?.primitive, .thud)
        XCTAssertEqual(s.last?.scale, 1)
        XCTAssertTrue((200...320).contains(duration(s)), "about 250 ms, was \(duration(s))")
        // The swell under it rises to the crack.
        let swell = notes(.surge).first { $0.kind == .continuous && $0.time == 0 }
        XCTAssertEqual(swell?.envelope.map(\.level), swell?.envelope.map(\.level).sorted())
        XCTAssertTrue((0.2...0.32).contains(HapticComposer.length(notes(.surge))))
    }

    func testZipIsAVeryShortSharpDoubleTick() {
        let s = steps(.zip)
        XCTAssertEqual(s.count, 2)
        XCTAssertTrue(s.allSatisfy { $0.primitive == .tick && $0.scale >= 0.85 })
        XCTAssertLessThanOrEqual(HapticComposer.length(notes(.zip)), 0.05)
    }

    func testLightningIsAnIrregularCrackleWithAFinalCrack() {
        let s = steps(.lightning)
        let pulses = Array(s.dropLast())
        XCTAssertTrue((3...5).contains(pulses.count))
        XCTAssertEqual(s.last?.primitive, .click)
        XCTAssertEqual(s.last?.scale, 1)
        XCTAssertEqual(Set(pulses.map(\.scale)).count, pulses.count, "uneven strength")
        XCTAssertEqual(Set(s.dropFirst().map(\.delayMs)).count, s.count - 1, "and spacing")
        XCTAssertTrue((150...230).contains(duration(s)))
    }

    func testAnAutomaticAnswerIsFeltMoreLightlyThanTheUsersOwn() {
        XCTAssertLessThanOrEqual(steps(.pop).first!.scale, 0.8)
        XCTAssertLessThan(sum(steps(.pop)), sum(steps(.deny)))
        XCTAssertEqual(FeedbackEvent.autoDenied.haptic.map { HapticTable.spec($0).priority }, 0)
    }

    // MARK: Textures and clicks

    /// The pattern as Core Haptics holds it: each entry keyed by the raw `CHHapticPattern.Key` names.
    private func exported(_ h: Haptic, _ s: HapticStrength) throws -> [[String: Any]] {
        let dict = try HapticPlayer.pattern(notes(h, s)).exportDictionary()
        let entries = dict[.pattern] as? [Any] ?? []
        return entries.compactMap(keyed)
    }

    private func keyed(_ any: Any) -> [String: Any]? {
        if let d = any as? [CHHapticPattern.Key: Any] { return Dictionary(uniqueKeysWithValues: d.map { ($0.key.rawValue, $0.value) }) }
        return any as? [String: Any]
    }

    /// The intensity control at `time`: 1 unless a curve set it (a curve keeps its last value after its end).
    private func intensityControl(_ entries: [[String: Any]], at time: Double) -> Double {
        var level = 1.0
        for entry in entries {
            guard let curve = entry[CHHapticPattern.Key.parameterCurve.rawValue].flatMap(keyed),
                  curve[CHHapticPattern.Key.parameterID.rawValue] as? String == CHHapticDynamicParameter.ID.hapticIntensityControl.rawValue,
                  let start = curve[CHHapticPattern.Key.time.rawValue] as? Double else { continue }
            let points = (curve[CHHapticPattern.Key.parameterCurveControlPoints.rawValue] as? [Any] ?? []).compactMap(keyed).compactMap { p -> (Double, Double)? in
                guard let t = p[CHHapticPattern.Key.time.rawValue] as? Double, let v = p[CHHapticPattern.Key.parameterValue.rawValue] as? Double else { return nil }
                return (start + t, v)
            }
            guard let first = points.first, time >= first.0 else { continue }
            level = points.last { $0.0 <= time }?.1 ?? level
        }
        return level
    }

    func testTheControlCurveCheckSeesACurve() throws {
        // A thud's body shaped the way it used to be, with a control curve, and a click after it: the click is muted.
        let body = CHHapticEvent(eventType: .hapticContinuous, parameters: [], relativeTime: 0, duration: 0.035)
        let click = CHHapticEvent(eventType: .hapticTransient, parameters: [], relativeTime: 0.065)
        let fade = CHHapticParameterCurve(parameterID: .hapticIntensityControl,
                                          controlPoints: [.init(relativeTime: 0, value: 1), .init(relativeTime: 0.035, value: 0)],
                                          relativeTime: 0)
        let dict = try CHHapticPattern(events: [body, click], parameterCurves: [fade]).exportDictionary()
        let entries = (dict[.pattern] as? [Any] ?? []).compactMap(keyed)
        XCTAssertEqual(intensityControl(entries, at: 0), 1, accuracy: 1e-6)
        XCTAssertEqual(intensityControl(entries, at: 0.065), 0, accuracy: 1e-6)
    }

    func testTexturesLeaveTheIntensityControlAtFullWhereverAClickLands() throws {
        for h in Haptic.allCases {
            for strength in HapticStrength.allCases {
                let entries = try exported(h, strength)
                XCTAssertEqual(entries.filter { $0[CHHapticPattern.Key.event.rawValue] != nil }.count,
                               HapticPlayer.events(notes(h, strength)).count, "\(h) \(strength) exported")
                for note in notes(h, strength) where note.kind == .transient {
                    XCTAssertEqual(intensityControl(entries, at: note.time), 1, accuracy: 1e-6, "\(h) \(strength) at \(note.time)")
                }
            }
        }
    }

    func testClicksKeepTheirDesignedStrengthNextToATexture() {
        for h in Haptic.allCases {
            let n = notes(h)
            let transients = HapticPlayer.events(n).filter { $0.kind == .transient }
            XCTAssertEqual(transients.map(\.intensity), n.filter { $0.kind == .transient }.map(\.intensity), "\(h)")
        }
        // The ones that were lost: Heavy's click after its thud, Lightning's last crack, Surge's first tick.
        XCTAssertEqual(HapticPlayer.events(notes(.heavy)).last { $0.kind == .transient }?.intensity, HapticComposer.scaled(0.5, .standard))
        XCTAssertEqual(HapticPlayer.events(notes(.lightning)).last { $0.kind == .transient }?.intensity, 1)
        XCTAssertEqual(HapticPlayer.events(notes(.surge)).first { $0.kind == .transient }?.intensity, HapticComposer.scaled(0.2 * 0.8, .standard))
    }

    func testAShapedTextureIsPlayedAsShortStepsFollowingItsShape() {
        let swell = notes(.surge).first { $0.kind == .continuous && $0.time == 0 }!
        let steps = HapticPlayer.events([swell])
        XCTAssertGreaterThan(steps.count, 10)
        XCTAssertTrue(steps.allSatisfy { $0.kind == .continuous && $0.duration <= HapticPlayer.textureStep + 1e-9 })
        XCTAssertEqual(steps.first!.time, 0)
        XCTAssertEqual(steps.last!.time + steps.last!.duration, swell.duration, accuracy: 1e-9)
        // It grows and sharpens towards the crack.
        XCTAssertEqual(steps.map(\.intensity), steps.map(\.intensity).sorted())
        XCTAssertEqual(steps.map(\.sharpness), steps.map(\.sharpness).sorted())
        XCTAssertLessThan(steps.first!.sharpness, steps.last!.sharpness)
        // Lightning's buzz flickers in sharpness, not only in strength.
        let buzz = HapticPlayer.events(notes(.lightning).filter { $0.kind == .continuous })
        let sharpness = buzz.map(\.sharpness)
        XCTAssertNotEqual(sharpness, sharpness.sorted())
        XCTAssertNotEqual(sharpness, sharpness.sorted(by: >))
        // A flat texture stays one event.
        let flat = HapticNote(kind: .continuous, time: 0.1, duration: 0.2, intensity: 0.5, sharpness: 0.3)
        XCTAssertEqual(HapticPlayer.events([flat]), [HapticPlayer.Event(kind: .continuous, time: 0.1, duration: 0.2, intensity: 0.5, sharpness: 0.3)])
    }

    func testAShapeIsStraightBetweenItsPointsAndLevelBeyondThem() {
        let shape: [HapticNote.Point] = [.init(at: 0.2, level: 0.2), .init(at: 0.6, level: 1), .init(at: 1, level: 0)]
        XCTAssertEqual(HapticNote.value(shape, at: 0), 0.2)
        XCTAssertEqual(HapticNote.value(shape, at: 0.4), 0.6, accuracy: 1e-6)
        XCTAssertEqual(HapticNote.value(shape, at: 0.8), 0.5, accuracy: 1e-6)
        XCTAssertEqual(HapticNote.value(shape, at: 1), 0)
    }

    func testDetentTicksAreOneCheapEvent() {
        // A slider drag plays this many times a second: one transient, nothing continuous.
        XCTAssertEqual(notes(.tick).count, 1)
        XCTAssertEqual(notes(.tick).first?.kind, .transient)
    }
}
