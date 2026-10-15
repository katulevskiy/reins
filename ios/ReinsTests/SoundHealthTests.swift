import XCTest
@testable import Reins

private final class Sounded: @unchecked Sendable {
    var value: Bool?
}

/// A sound never holds the app up: skipped at once while the audio server does not answer, and after the engine failed
/// to start until a growing backoff has passed.
final class SoundHealthTests: XCTestCase {
    private var now: Int64 = 10_000

    func testAnAudioCallThatDoesNotComeBackSkipsSoundsUntilItDoes() {
        let health = SoundHealth()
        XCTAssertTrue(health.canTry(now: now))
        health.begin(now: now)
        XCTAssertTrue(health.canTry(now: now + SoundHealth.stuckMs - 1), "a call in progress is not yet stuck")
        XCTAssertFalse(health.canTry(now: now + SoundHealth.stuckMs))
        health.end()
        XCTAssertTrue(health.canTry(now: now + SoundHealth.stuckMs))
    }

    func testAFailedStartBacksOffLongerEachTimeUntilTheEngineStarts() {
        let health = SoundHealth()
        health.failed(now: now)
        XCTAssertFalse(health.canTry(now: now + SoundHealth.firstBackoffMs - 1))
        XCTAssertTrue(health.canTry(now: now + SoundHealth.firstBackoffMs))
        health.failed(now: now)
        XCTAssertFalse(health.canTry(now: now + 2 * SoundHealth.firstBackoffMs - 1), "the backoff doubles")
        for _ in 0..<10 { health.failed(now: now) }
        XCTAssertTrue(health.canTry(now: now + SoundHealth.maxBackoffMs), "never longer than the cap")
        health.started()
        XCTAssertTrue(health.canTry(now: now))
    }

    func testPlayingWhileTheAudioServerDoesNotAnswerReturnsAtOnce() {
        let player = SoundPlayer(clock: { [unowned self] in now })
        let release = DispatchSemaphore(value: 0)
        player.simulateStuckAudioCall(until: release)
        now += SoundHealth.stuckMs
        let sounded = Sounded()
        let started = Date()
        player.play(.tap, volume: 1, rate: 1, priority: 0) { sounded.value = $0 }
        player.touchDown()
        XCTAssertLessThan(Date().timeIntervalSince(started), 0.05, "returned at once")
        XCTAssertEqual(sounded.value, false, "skipped, not queued behind the stuck call")
        release.signal()
    }
}
