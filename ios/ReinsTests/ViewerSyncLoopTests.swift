import XCTest
@testable import Reins

/// The foreground loop on a phone that views the account: it runs (the core's `sync` follows the account's log there),
/// even on a phone that was the approval device once and was replaced, which is how a second phone usually starts.
final class ViewerSyncLoopTests: XCTestCase {
    func testAViewerFollowsTheLogEvenAfterItWasReplaced() {
        XCTAssertTrue(AppModel.foregroundLoopRuns(role: .viewer, deviceReplaced: true))
        XCTAssertTrue(AppModel.foregroundLoopRuns(role: .viewer, deviceReplaced: false))
    }

    func testTheApprovingPhoneRuns() {
        XCTAssertTrue(AppModel.foregroundLoopRuns(role: .primary, deviceReplaced: false))
        XCTAssertTrue(AppModel.foregroundLoopRuns(role: .legacy, deviceReplaced: false))
    }

    func testAReplacedPhoneThatViewsNothingWaits() {
        XCTAssertFalse(AppModel.foregroundLoopRuns(role: .legacy, deviceReplaced: true))
        XCTAssertFalse(AppModel.foregroundLoopRuns(role: .removed, deviceReplaced: true))
    }
}
