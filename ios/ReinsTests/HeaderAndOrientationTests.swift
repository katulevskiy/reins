import XCTest
@testable import Reins

/// The header's mode chip keeps the title's room while a timed mode runs, and the iPhone app stays in portrait.
final class HeaderAndOrientationTests: XCTestCase {
    func testATimedModeShowsItsTimeAndSpeaksItsName() {
        XCTAssertEqual(ModeChip(mode: .bypass(left: "54:27")).spoken, "Bypass, 54:27 left")
        XCTAssertEqual(ModeChip(mode: .overnight(left: "5:59")).spoken, "Overnight, 5:59 left")
        XCTAssertEqual(ModeChip(mode: .lockdown).spoken, "Lockdown")
        XCTAssertEqual(ModeChip(mode: .manual).spoken, "Manual")
        XCTAssertEqual(ModeChip(mode: .bypass(left: "")).spoken, "Bypass", "no time known: the name")
    }

    func testTheIPhoneIsPortraitOnly() {
        let phone = Bundle.main.object(forInfoDictionaryKey: "UISupportedInterfaceOrientations") as? [String]
        XCTAssertEqual(phone, ["UIInterfaceOrientationPortrait"])
        let pad = Bundle.main.object(forInfoDictionaryKey: "UISupportedInterfaceOrientations~ipad") as? [String]
        XCTAssertEqual(pad?.count, 4, "the iPad keeps every orientation")
    }
}
