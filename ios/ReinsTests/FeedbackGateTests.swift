import XCTest
@testable import Reins

private final class FakeEnv: FeedbackEnvironment {
    var active = true
    var hasHaptics = true
}

private final class CountingEnv: FeedbackEnvironment {
    var reads = 0
    var active: Bool { reads += 1; return true }
    var hasHaptics: Bool { reads += 1; return true }
}

/// When things play (the Android app's `FeedbackGateTest`, `TapFallbackTest` and `LatencyTest`'s warm policy), and
/// which voice a cue takes.
final class FeedbackGateTests: XCTestCase {
    private var now: Int64 = 10_000
    private var settings = FeedbackSettings()
    private let env = FakeEnv()
    private lazy var gate = FeedbackGate(settings: { [unowned self] in settings }, env: env, clock: { [unowned self] in now })

    private func with(_ change: (inout FeedbackSettings) -> Void) -> FeedbackSettings {
        var s = FeedbackSettings()
        change(&s)
        return s
    }

    func testByDefaultEverythingPlays() {
        XCTAssertEqual(gate.haptic(.confirm), .play)
        XCTAssertEqual(gate.cue(.send), .play)
    }

    func testHapticsNeedTheirSwitchAndATapticEngine() {
        settings = with { $0.haptics = false }
        XCTAssertEqual(gate.haptic(.confirm).skipped, .hapticsOff)
        settings = FeedbackSettings()
        env.hasHaptics = false
        XCTAssertEqual(gate.haptic(.confirm).skipped, .noHaptics)
    }

    func testTheMasterOffSilencesEverythingWhateverElseIsOn() {
        settings = with { $0.master = false }
        XCTAssertEqual(gate.haptic(.confirm).skipped, .masterOff)
        XCTAssertEqual(gate.haptic(.error, preview: true).skipped, .masterOff)
        for cue in Cue.allCases { XCTAssertEqual(gate.cue(cue).skipped, .masterOff, "\(cue)") }
        XCTAssertEqual(gate.cue(.request, preview: true).skipped, .masterOff)
        XCTAssertFalse(settings.soundsOn)
        XCTAssertFalse(settings.hapticsOn)
    }

    func testWithTheMasterOnTheSwitchesBelowDecide() {
        settings = with { $0.sounds = false }
        XCTAssertEqual(gate.cue(.tap).skipped, .soundsOff)
        XCTAssertEqual(gate.haptic(.confirm), .play, "haptics are their own switch")
        settings = with { $0.haptics = false }
        XCTAssertEqual(gate.haptic(.confirm).skipped, .hapticsOff)
        XCTAssertEqual(gate.cue(.tap), .play)
    }

    func testNothingPlaysInTheBackground() {
        env.active = false
        XCTAssertEqual(gate.haptic(.error).skipped, .inactive)
        XCTAssertEqual(gate.cue(.request).skipped, .inactive)
    }

    func testEachKindOfSoundHasItsOwnSwitch() {
        settings = with { $0.interfaceSounds = false }
        XCTAssertEqual(gate.cue(.tap).skipped, .categoryOff)
        XCTAssertEqual(gate.cue(.send).skipped, .categoryOff)
        XCTAssertEqual(gate.cue(.request), .play, "requests are independent of interface sounds")

        now += 1000
        settings = with { $0.requestSounds = false }
        XCTAssertEqual(gate.cue(.request).skipped, .categoryOff)
        XCTAssertEqual(gate.cue(.attention), .play)
        now += 1000
        settings = with { $0.alertSounds = false }
        XCTAssertEqual(gate.cue(.attention).skipped, .categoryOff)
        XCTAssertEqual(gate.cue(.autoApproved), .play)
        now += 1000
        settings = with { $0.autopilotSounds = false }
        XCTAssertEqual(gate.cue(.autoApproved).skipped, .categoryOff)
        XCTAssertEqual(gate.cue(.autoDenied).skipped, .categoryOff)
        XCTAssertEqual(gate.cue(.tap), .play)
    }

    func testAPreviewIgnoresCategorySwitchesAndRateLimitsButNotTheMasterOrSoundsSwitch() {
        settings = with {
            $0.interfaceSounds = false
            $0.requestSounds = false
        }
        XCTAssertEqual(gate.cue(.request, preview: true), .play)
        XCTAssertEqual(gate.cue(.request, preview: true), .play)
        settings = with { $0.sounds = false }
        XCTAssertEqual(gate.cue(.tap, preview: true).skipped, .soundsOff)
    }

    func testTheSameCueNeverStacks() {
        XCTAssertEqual(gate.cue(.tap), .play)
        now += 30
        XCTAssertEqual(gate.cue(.tap).skipped, .rate)
        now += 40
        XCTAssertEqual(gate.cue(.tap), .play)
    }

    func testTheSameHapticIsCoalesced() {
        XCTAssertEqual(gate.haptic(.tick), .play)
        now += 20
        XCTAssertEqual(gate.haptic(.tick).skipped, .rate)
        now += 60
        XCTAssertEqual(gate.haptic(.tick), .play)
    }

    func testLightHapticsDoNotChatterAcrossKinds() {
        XCTAssertEqual(gate.haptic(.tick), .play)
        now += 10
        XCTAssertEqual(gate.haptic(.select).skipped, .rate, "inside the global gap")
        now += 30
        XCTAssertEqual(gate.haptic(.select), .play)
    }

    func testImportantHapticsCutThroughLightOnes() {
        XCTAssertEqual(gate.haptic(.tick), .play)
        now += 5
        XCTAssertEqual(gate.haptic(.error), .play)
        now += 5
        XCTAssertEqual(gate.haptic(.tick).skipped, .rate)
    }

    func testABurstOfLightHapticsIsCapped() {
        var played = 0
        for i in 0..<60 {
            if gate.haptic(i % 2 == 0 ? .tick : .select) == .play { played += 1 }
            now += 30
        }
        // 60 events over 1.8 s, 12 per second at most.
        XCTAssertLessThanOrEqual(played, FeedbackGate.lightHapticsPerWindow * 2)
        XCTAssertGreaterThan(played, 0)
    }

    func testVoicesAreCappedAndAMoreImportantCueStillGetsIn() {
        for cue in [Cue.request, .done, .uploadReady, .reconnected] {
            XCTAssertEqual(gate.cue(cue), .play, "\(cue)")
            now += 30
        }
        XCTAssertEqual(gate.cue(.tap).skipped, .voices)
        XCTAssertEqual(gate.cue(.attention), .play)
        now += 2000
        XCTAssertEqual(gate.cue(.tap), .play)
    }

    func testTheGateReadsTheEnvironmentABoundedNumberOfTimesPerDecision() {
        let counting = CountingEnv()
        let g = FeedbackGate(settings: { FeedbackSettings() }, env: counting, clock: { [unowned self] in now })
        _ = g.cue(.tap)
        XCTAssertLessThanOrEqual(counting.reads, 2)
        counting.reads = 0
        now += 1000
        _ = g.haptic(.confirm)
        XCTAssertLessThanOrEqual(counting.reads, 2)
    }

    // MARK: Claims (cueUnlessRecent)

    func testACloseRightAfterACueOrAChoiceStaysQuiet() {
        let claims = ClaimTracker { [unowned self] in now }
        XCTAssertFalse(claims.cueWithin(ClaimTracker.recentMs))
        claims.claimCue()
        now += 100
        XCTAssertTrue(claims.cueWithin(250))
        now += 200
        XCTAssertFalse(claims.cueWithin(250))
        claims.quiet()
        now += 100
        XCTAssertTrue(claims.cueWithin(250))
    }

    // MARK: Keeping the output awake

    func testTheOutputStaysAwakeForAWhileAfterTheLastTouch() {
        let warm = WarmPolicy(clock: { [unowned self] in now }, idleMs: 1000)
        XCTAssertFalse(warm.wanted)
        warm.touch()
        XCTAssertTrue(warm.wanted)
        now += 600
        XCTAssertEqual(warm.remainingMs, 400)
        warm.touch()
        now += 900
        XCTAssertTrue(warm.wanted)
        now += 200
        XCTAssertFalse(warm.wanted)
        XCTAssertEqual(warm.remainingMs, 0)
        warm.touch()
        warm.stop()
        XCTAssertFalse(warm.wanted)
    }

    // MARK: Voices

    func testANewCueTakesAFreeVoiceThenStealsTheLeastImportant() {
        var pool = VoicePool(count: 3)
        XCTAssertEqual(pool.claim(now: 0, priority: 2, lengthMs: 500), 0)
        XCTAssertEqual(pool.claim(now: 10, priority: 0, lengthMs: 500), 1)
        XCTAssertEqual(pool.claim(now: 20, priority: 1, lengthMs: 500), 2)
        // All busy: the tap (priority 0) is the one to go.
        XCTAssertEqual(pool.claim(now: 30, priority: 1, lengthMs: 500), 1)
        // Now 2, 1, 1: the older of the two priority-1 voices.
        XCTAssertEqual(pool.claim(now: 40, priority: 1, lengthMs: 500), 2)
        // Nothing gives way to a cue less important than everything sounding.
        XCTAssertNil(pool.claim(now: 50, priority: 0, lengthMs: 500))
        // A voice that finished is free again.
        XCTAssertEqual(pool.claim(now: 600, priority: 0, lengthMs: 100), 0)
        pool.reset()
        XCTAssertEqual(pool.claim(now: 601, priority: 0, lengthMs: 100), 0)
    }
}
