import UserNotifications
import XCTest
@testable import Reins

final class NotificationAccessTests: XCTestCase {
    func testTheSystemsAnswerMapsToWhatTheAppSays() {
        XCTAssertEqual(NotificationAccessState.of(.notDetermined, alert: .notSupported), .notAsked)
        XCTAssertEqual(NotificationAccessState.of(.denied, alert: .disabled), .off)
        XCTAssertEqual(NotificationAccessState.of(.authorized, alert: .enabled), .on)
        XCTAssertEqual(NotificationAccessState.of(.ephemeral, alert: .enabled), .on)
        // Allowed but silent: a request would not sound or show, so it counts as needing attention.
        XCTAssertEqual(NotificationAccessState.of(.provisional, alert: .enabled), .quiet)
        XCTAssertEqual(NotificationAccessState.of(.authorized, alert: .disabled), .quiet)
    }

    func testOnlyAWorkingStateNeedsNoAttention() {
        XCTAssertFalse(NotificationAccessState.on.needsAttention)
        XCTAssertFalse(NotificationAccessState.unknown.needsAttention)
        for state in [NotificationAccessState.notAsked, .quiet, .off] {
            XCTAssertTrue(state.needsAttention, "\(state)")
        }
        XCTAssertEqual(NotificationAccessState.off.summary, "Off")
    }
}
