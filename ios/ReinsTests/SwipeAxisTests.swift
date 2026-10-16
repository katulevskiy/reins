import CoreGraphics
import XCTest
@testable import Reins

/// When a touch on a row is a sideways swipe and when it is the list's scroll (`SwipeAxis`, behind SidewaysPan).
final class SwipeAxisTests: XCTestCase {
    private func sideways(_ tx: CGFloat, _ ty: CGFloat, v vx: CGFloat = 0, _ vy: CGFloat = 0) -> Bool {
        SwipeAxis.sideways(translation: CGPoint(x: tx, y: ty), velocity: CGPoint(x: vx, y: vy))
    }

    func testStraightUpOrDownIsAScroll() {
        XCTAssertFalse(sideways(0, -12, v: 0, -900))
        XCTAssertFalse(sideways(0, 12, v: 0, 900))
        XCTAssertFalse(sideways(2, -14, v: 40, -1200), "a thumb's slight drift is still a scroll")
    }

    func testStraightSidewaysIsASwipeEitherWay() {
        XCTAssertTrue(sideways(-12, 0, v: -800, 0))
        XCTAssertTrue(sideways(12, 0, v: 800, 0))
        XCTAssertTrue(sideways(-14, -3, v: -900, -150), "a little drift is still sideways")
    }

    func testADiagonalIsAScroll() {
        XCTAssertFalse(sideways(10, 10, v: 600, 600), "45° goes to the list")
        XCTAssertFalse(sideways(-10, 9, v: -500, 450))
        XCTAssertFalse(sideways(12, 10, v: 600, 500), "40° from horizontal is not clear enough")
    }

    func testTheMarginIsClear() {
        // 1.5 : 1 is the edge: at it a swipe, under it the list's.
        XCTAssertTrue(sideways(15, 10))
        XCTAssertFalse(sideways(14, 10))
        XCTAssertEqual(SwipeAxis.margin, 1.5)
    }

    func testVelocityCanSettleWhatTheFirstPointsLeaveOpen() {
        // Moved a little steeply so far, but going clearly sideways now.
        XCTAssertTrue(sideways(12, 9, v: 900, 200))
        // Moved sideways so far, with no speed to tell.
        XCTAssertTrue(sideways(-16, 2, v: 0, 0))
    }

    func testEitherLeaningUpOrDownGivesItToTheList() {
        XCTAssertFalse(sideways(16, 2, v: 100, 700), "turning to scroll")
        XCTAssertFalse(sideways(6, 12, v: 900, 100), "it moved up first")
        XCTAssertFalse(sideways(10, 10, v: 900, 0), "it moved diagonally first")
    }

    func testNoMovementIsNoSwipe() {
        XCTAssertFalse(sideways(0, 0, v: 0, 0))
    }
}
