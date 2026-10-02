import XCTest

/// Settings > Sounds & haptics on the demo core: the switches cascade, the slider moves in tenths, the previews run.
/// Screenshots of each stage are kept in the result bundle (`xcresulttool export attachments`).
final class SoundsScreenUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-demo", "-noPopup", "-show", "sounds"]
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
        toggle(id).coordinate(withNormalizedOffset: CGVector(dx: 0.93, dy: 0.5)).tap()
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
        XCTAssertEqual(app.staticTexts["volumeValue"].label, "80%")
        volume.adjust(toNormalizedSliderPosition: 0.5)
        XCTAssertEqual(app.staticTexts["volumeValue"].label, "50%")

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
