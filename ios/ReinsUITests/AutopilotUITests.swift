import XCTest

/// Walks Autopilot in the `-demo` app the way a user would: the modes with their warnings, the model, a profile,
/// "Try it". Screenshots go to the test's attachments and, when `REINS_SHOTS` names a folder, there too.
final class AutopilotUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
    }

    private func launch(_ extra: [String] = []) {
        app.launchArguments = ["-demo", "-noPopup"] + extra
        app.launch()
        let tab = app.tabBars.buttons["Autopilot"].firstMatch
        if tab.waitForExistence(timeout: 10) {
            tab.tap()
        } else {
            // A regular width: the sidebar.
            app.buttons["Autopilot"].firstMatch.tap()
        }
        _ = waitFor("modeHero")
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let folder = ProcessInfo.processInfo.environment["REINS_SHOTS"] {
            try? screenshot.pngRepresentation.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
        }
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any)[id].firstMatch }

    @discardableResult
    private func waitFor(_ id: String, timeout: TimeInterval = 8) -> XCUIElement {
        let e = element(id)
        XCTAssertTrue(e.waitForExistence(timeout: timeout), "\(id) never showed")
        return e
    }

    private func text(_ s: String, timeout: TimeInterval = 8) {
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", s)).firstMatch.waitForExistence(timeout: timeout), "no text \(s)")
    }

    private func tap(_ id: String) {
        let e = waitFor(id)
        if !e.isHittable { app.swipeUp() }
        e.tap()
    }

    private func scrollTo(_ id: String) {
        let e = element(id)
        var tries = 0
        while !(e.exists && e.isHittable) && tries < 8 {
            app.swipeUp()
            tries += 1
        }
    }

    func testModesWarnBeforeBypassAndLockdown() {
        launch()
        text("Assisted")
        shot("autopilot-assisted")
        tap("mode:auto")
        text("Autopilot answers what it is sure of")
        tap("mode:bypass")
        waitFor("confirmBypass")
        shot("autopilot-bypass-sheet")
        tap("bypass:30")
        tap("confirmBypass")
        waitFor("bypassClock")
        text("Stop bypass")
        shot("autopilot-bypass")
        tap("stopBypass")
        text("Autopilot answers what it is sure of")
        tap("mode:lockdown")
        XCTAssertTrue(app.alerts["Lock down?"].waitForExistence(timeout: 5))
        app.alerts.buttons["Lock down"].tap()
        waitFor("endLockdown")
        shot("autopilot-lockdown")
        tap("endLockdown")
        text("Autopilot answers what it is sure of")
    }

    func testTheModelDownloadsWithProgressAndCanBeDeleted() {
        launch(["-demoNoModel"])
        text("NEEDS MODEL")
        scrollTo("downloadModel")
        shot("autopilot-no-model")
        tap("downloadModel")
        text("Downloading")
        shot("autopilot-downloading")
        waitFor("modelInstalled", timeout: 15)
        tap("deleteModel")
        XCTAssertTrue(app.alerts["Delete the model?"].waitForExistence(timeout: 5))
        app.alerts.buttons["Delete"].tap()
        waitFor("downloadModel")
    }

    func testAProfileShowsItsKindsAndUnlockingWarns() {
        launch()
        scrollTo("profile:personal")
        shot("autopilot-profiles")
        tap("profile:personal")
        waitFor("class:github/write/push")
        shot("profile")
        tap("class:gmail/read")
        tap("lock:off:gmail/read")
        XCTAssertTrue(app.alerts.firstMatch.waitForExistence(timeout: 5))
        app.alerts.buttons["Unlock"].tap()
        text("Unlocked by you")
        app.buttons["Cautious"].firstMatch.tap()
        text("Approves when 98% sure")
    }

    func testTryItJudgesAnExample() {
        launch()
        scrollTo("openTryIt")
        tap("openTryIt")
        waitFor("situation")
        tap("example:Injection attempt")
        tap("evaluate")
        waitFor("verdictWord", timeout: 8)
        text("Deny")
        app.swipeUp()
        shot("try-it")
    }
}
