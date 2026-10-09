import XCTest
@testable import Reins

@MainActor
final class ArrivalTests: XCTestCase {
    private final class Box {
        let name: String
        init(_ name: String) { self.name = name }
    }

    func testWaitersResumeInOrderAfterTheHooksOnceItComes() async {
        let arrival = Arrival<Box>()
        var seen: [String] = []
        arrival.whenSettled { seen.append("hook 1: \($0.name)") }
        arrival.whenSettled { seen.append("hook 2: \($0.name)") }
        var waiters: [Task<Void, Never>] = []
        var started = 0
        for i in 1...3 {
            waiters.append(Task {
                // On the main actor, nothing runs between this and the wait beginning.
                started += 1
                let box = await arrival.wait()
                seen.append("waiter \(i): \(box?.name ?? "none")")
            })
            // Each begins to wait before the next starts.
            while started < i { await Task.yield() }
        }
        XCTAssertEqual(seen, [], "nothing runs before it comes")
        arrival.settle(Box("model"))
        for waiter in waiters { await waiter.value }
        XCTAssertEqual(seen, ["hook 1: model", "hook 2: model", "waiter 1: model", "waiter 2: model", "waiter 3: model"])
    }

    func testOnceItCameHooksRunAndWaitsReturnAtOnce() async {
        let arrival = Arrival<Box>()
        arrival.settle(Box("model"))
        var hooked: String?
        arrival.whenSettled { hooked = $0.name }
        XCTAssertEqual(hooked, "model")
        let waited = await arrival.wait()
        XCTAssertEqual(waited?.name, "model")
    }

    func testNeverComingResumesTheWaitersWithNothingAndRunsNoHook() async {
        let arrival = Arrival<Box>()
        var hooked = false
        arrival.whenSettled { _ in hooked = true }
        var started = false
        let waiter = Task {
            started = true
            return await arrival.wait()
        }
        while !started { await Task.yield() }
        arrival.settle(nil)
        let waited = await waiter.value
        XCTAssertNil(waited)
        XCTAssertFalse(hooked)
        arrival.whenSettled { _ in hooked = true }
        XCTAssertFalse(hooked)
    }

    func testOnlyTheFirstSettleCounts() async {
        let arrival = Arrival<Box>()
        arrival.settle(Box("first"))
        arrival.settle(Box("second"))
        let waited = await arrival.wait()
        XCTAssertEqual(waited?.name, "first")
    }
}
