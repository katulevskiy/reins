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

    /// Taps the switch itself (the row's trailing edge): a tap in the middle lands on its label.
    /// A lazy List can report an offscreen switch with an infinite frame. Scroll until its
    /// actual tap point is visible, then anchor to the app so XCTest cannot resolve it again
    /// to an infinite element-relative coordinate while synthesizing the event.
    private func flip(_ id: String, towardTop: Bool = false) {
        let screen = app.frame
        let navigation = app.navigationBars.firstMatch
        let top = navigation.exists ? max(screen.minY + 1, navigation.frame.maxY) : screen.minY + 1
        let visible = CGRect(x: screen.minX + 1, y: top, width: screen.width - 2,
                             height: max(0, screen.maxY - 34 - top))
        let list = app.collectionViews["soundsList"]
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        var lastFrame = CGRect.zero
        for attempt in 0...8 {
            lastFrame = .zero
            // SwiftUI may retain an obsolete AX node after scrolling. Inspect fresh matches.
            for control in app.switches.matching(identifier: id).allElementsBoundByIndex {
                lastFrame = control.frame
                let point = CGPoint(x: lastFrame.minX + lastFrame.width * 0.93, y: lastFrame.midY)
                if lastFrame.width > 1, lastFrame.height > 1,
                   point.x.isFinite, point.y.isFinite, visible.contains(point), control.isHittable {
                    app.coordinate(withNormalizedOffset: .zero)
                        .withOffset(CGVector(dx: point.x - screen.minX, dy: point.y - screen.minY)).tap()
                    return
                }
            }
            guard attempt < 8 else { break }
            let down = lastFrame.midY.isFinite && lastFrame.height > 1
                ? lastFrame.midY < visible.minY : towardTop
            // The center drag can land on the volume thumb and be consumed by
            // Slider. Use the inset List's empty leading gutter, also in split view.
            let scrollFrame = list.frame.intersection(visible)
            guard scrollFrame.minX.isFinite, scrollFrame.minY.isFinite,
                  scrollFrame.width.isFinite, scrollFrame.height.isFinite,
                  scrollFrame.width > 20, scrollFrame.height > 1 else {
                return XCTFail("Sounds list has no finite visible scroll area: \(scrollFrame)")
            }
            let origin = app.coordinate(withNormalizedOffset: .zero)
            let x = scrollFrame.minX + 10 - screen.minX
            let startY = scrollFrame.minY + scrollFrame.height * (down ? 0.2 : 0.75) - screen.minY
            let endY = scrollFrame.minY + scrollFrame.height * (down ? 0.75 : 0.2) - screen.minY
            origin.withOffset(CGVector(dx: x, dy: startY))
                .press(forDuration: 0.05,
                       thenDragTo: origin.withOffset(CGVector(dx: x, dy: endY)),
                       withVelocity: XCUIGestureVelocity(rawValue: 300), thenHoldForDuration: 0.4)
        }
        XCTFail("\(id) has no finite visible switch point (frame \(lastFrame), viewport \(visible))")
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
        flip("soundsMaster", towardTop: true)
        XCTAssertFalse(toggle("haptics").isEnabled, "the master turns everything off")
        flip("soundsMaster")
        XCTAssertTrue(toggle("haptics").isEnabled)
    }
}
