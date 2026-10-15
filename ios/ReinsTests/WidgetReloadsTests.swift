import XCTest
@testable import Reins

private final class Count: @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0

    var value: Int { lock.withLock { n } }

    func add() -> Int { lock.withLock { n += 1; return n } }
}

/// Each widget reload wakes the widget extension: a burst of snapshot changes reloads once at once and once more at
/// the end of the spacing, never once per change.
final class WidgetReloadsTests: XCTestCase {
    func testABurstReloadsAtOnceAndOnceMoreAtTheEnd() {
        let count = Count()
        let folded = expectation(description: "the folded reload")
        let reloads = WidgetReloads(spacing: 0.3) { if count.add() == 2 { folded.fulfill() } }
        for _ in 0..<5 { reloads.request() }
        XCTAssertEqual(count.value, 1, "the first request reloads at once")
        wait(for: [folded], timeout: 3)
        let quiet = expectation(description: "nothing more")
        quiet.isInverted = true
        wait(for: [quiet], timeout: 0.6)
        XCTAssertEqual(count.value, 2, "the other four became one reload")
    }

    func testRequestsSpacedApartEachReload() {
        let count = Count()
        let reloads = WidgetReloads(spacing: 0.1) { _ = count.add() }
        reloads.request()
        let later = expectation(description: "after the spacing")
        later.isInverted = true
        wait(for: [later], timeout: 0.2)
        reloads.request()
        XCTAssertEqual(count.value, 2)
    }
}
