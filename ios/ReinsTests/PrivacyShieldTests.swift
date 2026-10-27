import XCTest
@testable import Reins

/// The privacy cover shows over the whole window only while a sensitive screen is up and the app is not in front or
/// is captured.
final class PrivacyShieldTests: XCTestCase {
    func testTheCoverShowsOnlyOverASensitiveScreenWhenNotInFrontOrCaptured() {
        XCTAssertFalse(PrivacyShield.shows(screens: 0, inactive: true, captured: true), "nothing sensitive: no cover")
        XCTAssertFalse(PrivacyShield.shows(screens: 1, inactive: false, captured: false), "in front, not captured")
        XCTAssertTrue(PrivacyShield.shows(screens: 1, inactive: true, captured: false), "the app switcher's snapshot")
        XCTAssertTrue(PrivacyShield.shows(screens: 2, inactive: false, captured: true), "recorded or mirrored")
    }
}
