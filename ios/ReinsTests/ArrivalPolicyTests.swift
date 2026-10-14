import XCTest
@testable import Reins

/// What a request arriving while the app is in front gets: the chime and haptic once per request and per burst (the
/// Android app's `ArrivalPolicyTest`).
final class ArrivalPolicyTests: XCTestCase {
    private var now: Int64 = 100_000
    private lazy var policy = ArrivalPolicy(clock: { [unowned self] in now })

    private func with(_ change: (inout FeedbackSettings) -> Void) -> FeedbackSettings {
        var s = FeedbackSettings()
        change(&s)
        return s
    }

    private func arrive(_ id: String, needsUser: Bool = true, settings: FeedbackSettings = FeedbackSettings()) -> ArrivalOutcome {
        policy.decide(id: id, needsUser: needsUser, settings: settings)
    }

    func testANewRequestThatNeedsTheUserChimesAndBuzzes() {
        XCTAssertEqual(arrive("req1"), .chimeAndHaptic)
    }

    func testWhatAutopilotABypassOrAGrantAnsweredGetsNothingThenOrLater() {
        XCTAssertEqual(arrive("auto1", needsUser: false), .nothing)
        now += 10_000
        XCTAssertEqual(arrive("auto1"), .nothing)
        XCTAssertEqual(arrive("req2"), .chimeAndHaptic, "it does not hold back the next real one")
    }

    func testABurstChimesOnceAndItsRequestsStayAnnounced() {
        XCTAssertEqual(arrive("a"), .chimeAndHaptic)
        for id in ["b", "c", "d"] {
            now += 400
            XCTAssertEqual(arrive(id), .nothing, id)
        }
        now += ArrivalPolicy.coalesceMs
        XCTAssertEqual(arrive("c"), .nothing)
        XCTAssertEqual(arrive("e"), .chimeAndHaptic)
    }

    func testAtMostOneChimePerWindowCountedFromTheLastOne() {
        XCTAssertEqual(arrive("a"), .chimeAndHaptic)
        now += ArrivalPolicy.coalesceMs - 1
        XCTAssertEqual(arrive("b"), .nothing)
        now += 1
        XCTAssertEqual(arrive("c"), .chimeAndHaptic)
    }

    func testAnUpdateToARequestAlreadyShownStaysQuiet() {
        XCTAssertEqual(arrive("req1"), .chimeAndHaptic)
        now += 60_000
        XCTAssertEqual(arrive("req1"), .nothing)
    }

    func testSoundWhenOpenOffKeepsTheHapticOnly() {
        XCTAssertEqual(arrive("a", settings: with { $0.soundWhenOpen = false }), .hapticOnly)
        now += ArrivalPolicy.coalesceMs
        XCTAssertEqual(arrive("b", settings: with { $0.soundWhenOpen = false; $0.haptics = false }), .nothing)
    }

    func testTheSoundSwitchesAboveItAndTheMasterDecideToo() {
        XCTAssertEqual(arrive("a", settings: with { $0.requestSounds = false }), .hapticOnly)
        now += ArrivalPolicy.coalesceMs
        XCTAssertEqual(arrive("b", settings: with { $0.sounds = false }), .hapticOnly)
        now += ArrivalPolicy.coalesceMs
        XCTAssertEqual(arrive("c", settings: with { $0.haptics = false }), .chimeOnly)
        now += ArrivalPolicy.coalesceMs
        XCTAssertEqual(arrive("d", settings: with { $0.master = false }), .nothing)
    }

    func testASilentArrivalDoesNotStartABurstWindow() {
        XCTAssertEqual(arrive("a", settings: with { $0.master = false }), .nothing)
        XCTAssertEqual(arrive("b"), .chimeAndHaptic)
    }

    func testOldIdsAreForgottenAfterAWhileSoMemoryStaysBounded() {
        XCTAssertEqual(arrive("first"), .chimeAndHaptic)
        for i in 0..<ArrivalPolicy.remembered { _ = arrive("other\(i)") }
        now += ArrivalPolicy.coalesceMs
        XCTAssertEqual(arrive("first"), .chimeAndHaptic)
    }

    // MARK: The switch

    func testSoundWhenOpenDefaultsOnAndSurvivesOlderStoredSettings() throws {
        XCTAssertTrue(FeedbackSettings().soundWhenOpen)
        // What a version without the switch stored: the switch reads as on, and nothing else is reset.
        let old = #"{"master":true,"sounds":true,"interfaceSounds":false,"requestSounds":true,"autopilotSounds":true,"alertSounds":true,"volume":0.3,"haptics":false,"strength":"strong"}"#
        let s = try JSONDecoder().decode(FeedbackSettings.self, from: Data(old.utf8))
        XCTAssertTrue(s.soundWhenOpen)
        XCTAssertFalse(s.interfaceSounds)
        XCTAssertFalse(s.haptics)
        XCTAssertEqual(s.volume, 0.3, accuracy: 1e-6)
        XCTAssertEqual(s.strength, .strong)
    }

    func testSoundWhenOpenRoundTrips() throws {
        var s = FeedbackSettings()
        s.soundWhenOpen = false
        let back = try JSONDecoder().decode(FeedbackSettings.self, from: JSONEncoder().encode(s))
        XCTAssertEqual(back, s)
    }
}
