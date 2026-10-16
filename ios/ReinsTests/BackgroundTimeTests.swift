import UIKit
import XCTest
@testable import Reins

/// What keeps a background wake from being ended by iOS: background tasks ended exactly once, completion handlers called
/// exactly once within the budget, and the store lock never held without a background task (`SuspensionGuard`).
final class BackgroundTimeTests: XCTestCase {
    // MARK: BackgroundTaskToken

    private final class Ends: @unchecked Sendable {
        private let lock = NSLock()
        private var ids: [UIBackgroundTaskIdentifier] = []
        func record(_ id: UIBackgroundTaskIdentifier) { lock.withLock { ids.append(id) } }
        var all: [UIBackgroundTaskIdentifier] { lock.withLock { ids } }
    }

    func testABackgroundTaskIsEndedOnceWhoeverEndsItFirst() {
        let ends = Ends()
        let token = BackgroundTaskToken { ends.record($0) }
        token.started(UIBackgroundTaskIdentifier(rawValue: 7))
        token.end() // the expiration handler
        token.end() // the owner's defer
        XCTAssertEqual(ends.all, [UIBackgroundTaskIdentifier(rawValue: 7)])
        XCTAssertEqual(token.identifier, .invalid)
    }

    func testAnEndBeforeTheIdentifierIsKnownEndsItWhenItIs() {
        let ends = Ends()
        let token = BackgroundTaskToken { ends.record($0) }
        token.end()
        XCTAssertEqual(ends.all, [])
        token.started(UIBackgroundTaskIdentifier(rawValue: 9))
        token.end()
        XCTAssertEqual(ends.all, [UIBackgroundTaskIdentifier(rawValue: 9)])
    }

    func testATaskThatCouldNotBeBegunIsNeverEnded() {
        let ends = Ends()
        let token = BackgroundTaskToken { ends.record($0) }
        token.started(.invalid)
        token.end()
        XCTAssertEqual(ends.all, [])
    }

    func testConcurrentEndsEndOnce() {
        let ends = Ends()
        let token = BackgroundTaskToken { ends.record($0) }
        token.started(UIBackgroundTaskIdentifier(rawValue: 3))
        DispatchQueue.concurrentPerform(iterations: 64) { _ in token.end() }
        XCTAssertEqual(ends.all.count, 1)
    }

    // MARK: Budgeted

    @MainActor
    func testTheWorksResultCompletesOnceAndTheTimerStaysQuiet() async throws {
        var results: [Int] = []
        let budget = Budgeted(limit: .milliseconds(300), fallback: -1, work: { 1 }, finish: { results.append($0) })
        try await Task.sleep(for: .milliseconds(600))
        XCTAssertEqual(results, [1])
        budget.expire()
        XCTAssertEqual(results, [1], "an expiry after the result changes nothing")
    }

    @MainActor
    func testTheBudgetRunningOutCompletesWithTheFallbackAndCancelsTheWork() async throws {
        var results: [Int] = []
        var cancelled = false
        let budget = Budgeted(limit: .milliseconds(100), fallback: -1, work: {
            try? await Task.sleep(for: .seconds(5))
            cancelled = Task.isCancelled
            return 1
        }, finish: { results.append($0) })
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertEqual(results, [-1])
        XCTAssertTrue(budget.finished)
        XCTAssertTrue(cancelled, "the work was cancelled and its late result dropped")
    }

    @MainActor
    func testAnExpiryCompletesAtOnceExactlyOnce() async throws {
        var results: [Int] = []
        let budget = Budgeted(limit: .seconds(10), fallback: -1, work: {
            try? await Task.sleep(for: .seconds(5))
            return 1
        }, finish: { results.append($0) })
        budget.expire()
        budget.expire()
        XCTAssertEqual(results, [-1])
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(results, [-1])
    }

    // MARK: SuspensionGuard

    /// A `KeepAwake` that records its holds, and can refuse them.
    private final class FakeKeepAwake: KeepAwake, @unchecked Sendable {
        final class Token: KeepAwakeToken, @unchecked Sendable {
            private let lock = NSLock()
            private(set) var endCount = 0
            func end() { lock.withLock { endCount += 1 } }
            var ended: Bool { lock.withLock { endCount > 0 } }
        }

        private let lock = NSLock()
        var refuse = false
        private(set) var tokens: [Token] = []
        private(set) var expiring: [@Sendable () -> Void] = []

        func begin(expiring: @escaping @Sendable () -> Void) -> KeepAwakeToken? {
            lock.withLock {
                guard !refuse else { return nil }
                let t = Token()
                tokens.append(t)
                self.expiring.append(expiring)
                return t
            }
        }

        func expireLatest() { lock.withLock { expiring.last }?() }
    }

    func testOneHoldCoversABurstOfLocksAndEndsAfterTheGrace() {
        let keep = FakeKeepAwake()
        let guarding = SuspensionGuard(keepAwake: keep, grace: 0.2, drain: 0.2)
        for _ in 0..<5 {
            XCTAssertTrue(guarding.locking())
            guarding.unlocked()
        }
        XCTAssertEqual(keep.tokens.count, 1, "one background task for the burst")
        XCTAssertTrue(guarding.awake)
        let ended = expectation(description: "ended after the grace")
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.6) {
            if keep.tokens[0].endCount == 1, !guarding.awake { ended.fulfill() }
        }
        wait(for: [ended], timeout: 2)
    }

    func testAHoldNeverEndsWhileALockIsHeld() {
        let keep = FakeKeepAwake()
        let guarding = SuspensionGuard(keepAwake: keep, grace: 0.05, drain: 0.2)
        XCTAssertTrue(guarding.locking())
        Thread.sleep(forTimeInterval: 0.3)
        XCTAssertFalse(keep.tokens[0].ended, "still locked: the task stays")
        guarding.unlocked()
        Thread.sleep(forTimeInterval: 0.3)
        XCTAssertTrue(keep.tokens[0].ended)
    }

    func testNoBackgroundTaskMeansNoLock() {
        let keep = FakeKeepAwake()
        keep.refuse = true
        let guarding = SuspensionGuard(keepAwake: keep)
        XCTAssertFalse(guarding.locking(), "being suspended: the core's call fails instead of locking")
        XCTAssertEqual(guarding.holding, 0)
        keep.refuse = false
        XCTAssertTrue(guarding.locking())
        guarding.unlocked()
    }

    func testAnExpiryWaitsForTheLockInProgressThenEndsTheHold() {
        let keep = FakeKeepAwake()
        let guarding = SuspensionGuard(keepAwake: keep, grace: 10, drain: 2)
        XCTAssertTrue(guarding.locking())
        let expired = expectation(description: "the expiry returned")
        DispatchQueue.global().async {
            keep.expireLatest()
            expired.fulfill()
        }
        Thread.sleep(forTimeInterval: 0.2)
        XCTAssertFalse(keep.tokens[0].ended, "the lock in progress finishes first")
        XCTAssertFalse(guarding.locking(), "no new lock while time runs out")
        guarding.unlocked()
        wait(for: [expired], timeout: 2)
        XCTAssertEqual(keep.tokens[0].endCount, 1)
        XCTAssertFalse(guarding.awake)
        // Later (the next wake), a lock begins a new hold.
        XCTAssertTrue(guarding.locking())
        XCTAssertEqual(keep.tokens.count, 2)
        guarding.unlocked()
    }

    func testAnExpiryWithALockStuckPastTheDrainStillEndsTheHold() {
        let keep = FakeKeepAwake()
        let guarding = SuspensionGuard(keepAwake: keep, grace: 10, drain: 0.1)
        XCTAssertTrue(guarding.locking())
        keep.expireLatest()
        XCTAssertEqual(keep.tokens[0].endCount, 1, "iOS ends an app whose task outlives its time")
        guarding.unlocked()
    }
}
