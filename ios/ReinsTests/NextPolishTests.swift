import XCTest
@testable import Reins

/// TestFlight polish: the header chip's clock, and where the shared glass draws the art.
final class NextPolishTests: XCTestCase {
    func testTheChipDropsSecondsFromAnHourUp() {
        XCTAssertEqual(AutopilotText.chipClock(until: 1_000 + 47 * 60 + 12, now: 1_000), "47:12")
        XCTAssertEqual(AutopilotText.chipClock(until: 1_000 + 23 * 3_600 + 39 * 60 + 19, now: 1_000), "23:39")
        XCTAssertEqual(AutopilotText.chipClock(until: 1_000 + 3_600, now: 1_000), "1:00")
        XCTAssertEqual(AutopilotText.chipClock(until: 1_000 + 2 * 86_400 + 4 * 3_600, now: 1_000), "2d 4h")
        XCTAssertEqual(ModeChip.Mode.left(1_000 + 23 * 3_600 + 39 * 60 + 19, now: 1_000), "23:39")
        XCTAssertEqual(AutopilotText.clock(until: 1_000 + 23 * 3_600 + 39 * 60 + 19, now: 1_000), "23:39:19", "the running field keeps its seconds")
    }

    func testTheGlassDrawsTheArtWhereItIsDrawn() {
        // The welcome's art: 780 × 1000 filled into 393 × 620 from the top, centred across (its sides cropped).
        let rect = Backdrop.fillRect(image: CGSize(width: 780, height: 1000), in: CGRect(x: 0, y: 0, width: 393, height: 620))
        XCTAssertEqual(rect.height, 620, accuracy: 0.01)
        XCTAssertEqual(rect.width, 483.6, accuracy: 0.01)
        XCTAssertEqual(rect.midX, 196.5, accuracy: 0.01)
        XCTAssertEqual(rect.minY, 0, accuracy: 0.01)
        // Activity's wallpaper keeps its aspect, so its rect is its frame (wherever the scroll has moved it).
        let wall = CGRect(x: 0, y: -120, width: 390, height: 500)
        XCTAssertEqual(Backdrop.fillRect(image: CGSize(width: 780, height: 1000), in: wall), wall)
    }
}
