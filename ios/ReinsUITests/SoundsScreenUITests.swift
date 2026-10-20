import XCTest

/// Settings > Sounds & haptics on the demo core: the switches cascade, the slider moves in tenths, the previews run.
/// Screenshots of each stage are kept in the result bundle (`xcresulttool export attachments`).
final class SoundsScreenUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        // From the default settings: a run cut short can leave the Sounds switch off for the next one.
        app.launchArguments = ["-demo", "-noPopup", "-noLiveActivities", "-resetFeedback", "-show", "sounds"]
        app.launch()
    }

    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func toggle(_ id: String) -> XCUIElement { app.switches[id].firstMatch }

    /// Taps the switch itself (the row's trailing edge): a tap in the middle of a SwiftUI toggle row lands on its label.
    private func flip(_ id: String) {
        var point: CGPoint?
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            // SwiftUI can replace a switch's AX node after scrolling. Resolve each attempt afresh;
            // an invalid element's infinite frame must never reach XCTest's touch synthesizer.
            for target in self.app.switches.matching(identifier: id).allElementsBoundByIndex {
                let frame = target.frame
                guard !frame.isNull, !frame.isInfinite, frame.width > 1, frame.height > 1,
                      frame.minX.isFinite, frame.minY.isFinite, frame.maxX.isFinite, frame.maxY.isFinite else { continue }
                let candidate = CGPoint(x: frame.minX + frame.width * 0.93, y: frame.midY)
                if self.app.frame.contains(candidate), target.isHittable {
                    point = candidate
                    return true
                }
            }
            return false
        }, object: nil)
        XCTAssertEqual(XCTWaiter().wait(for: [ready], timeout: 8), .completed, "\(id) did not become rendered and hittable")
        guard let point else { return }
        // Use the validated frame from the successful query; element-bound coordinates re-query a potentially
        // retired node while synthesizing the event. The target remains the switch's trailing edge.
        app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: point.x, dy: point.y)).tap()
    }

    /// The volume reads a whole tenth, one step from `percent` at most: `adjust(toNormalizedSliderPosition:)` drags to
    /// about that point, and where it lands depends on the slider's width.
    private func assertTenth(near percent: Int, file: StaticString = #filePath, line: UInt = #line) {
        let label = app.staticTexts["volumeValue"].label
        guard let value = Int(label.replacingOccurrences(of: "%", with: "")) else {
            return XCTFail("volume reads \(label)", file: file, line: line)
        }
        XCTAssertEqual(value % 10, 0, "volume moves in tenths, reads \(label)", file: file, line: line)
        XCTAssertLessThanOrEqual(abs(value - percent), 10, "volume reads \(label), wanted about \(percent)%", file: file, line: line)
    }

    func testTheSwitchesCascadeAndTheVolumeMovesInSteps() {
        let master = toggle("soundsMaster")
        XCTAssertTrue(master.waitForExistence(timeout: 15))
        shot("sounds-top")

        let interface = toggle("interfaceSounds")
        XCTAssertTrue(interface.isEnabled)
        flip("sounds")
        XCTAssertFalse(interface.isEnabled, "categories follow the Sounds switch")
        flip("sounds")
        XCTAssertTrue(interface.isEnabled)

        let volume = app.sliders["volume"]
        XCTAssertTrue(volume.exists)
        volume.adjust(toNormalizedSliderPosition: 0.8)
        assertTenth(near: 80)
        volume.adjust(toNormalizedSliderPosition: 0.5)
        assertTenth(near: 50)

        app.swipeUp()
        shot("sounds-middle")
        app.swipeUp()
        app.buttons["try:Slider"].firstMatch.tap()
        shot("sounds-bottom")
        app.swipeDown()
        app.swipeDown()
        app.swipeDown()

        flip("soundsMaster")
        XCTAssertFalse(toggle("haptics").isEnabled, "the master turns everything off")
        flip("soundsMaster")
        XCTAssertTrue(toggle("haptics").isEnabled)
    }
}
