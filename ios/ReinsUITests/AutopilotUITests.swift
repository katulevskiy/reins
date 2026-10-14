import XCTest

/// Walks Autopilot in the `-demo` app the way a user would: the modes, a bypass held to confirm (timed and until
/// turned off), Lock down as a plain tap, the model, a profile and its thresholds, "Try it". Screenshots go to the test's attachments and, when `REINS_SHOTS` names a folder, there too.
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

    private func selected(_ id: String) {
        expectation(for: NSPredicate(format: "isSelected == true"), evaluatedWith: element(id))
        waitForExpectations(timeout: 5)
    }

    func testBypassIsHeldAndLockDownIsAPlainTap() {
        launch()
        waitFor("mode:manual")
        selected("mode:manual")
        shot("autopilot-manual")
        tap("mode:auto")
        selected("mode:auto")
        tap("mode:bypass")
        waitFor("confirmBypass")
        shot("autopilot-bypass-sheet")
        tap("bypass:60")
        // A tap is not enough: held for two seconds.
        waitFor("confirmBypass").tap()
        XCTAssertTrue(element("confirmBypass").exists, "a tap does not start a bypass")
        waitFor("confirmBypass").press(forDuration: 2.6)
        waitFor("bypassClock")
        text("Approving without asking")
        shot("autopilot-bypass")
        tap("stopBypass")
        selected("mode:auto")
        // Lock down: no dialog.
        tap("mode:lockdown")
        waitFor("endLockdown")
        XCTAssertFalse(app.alerts.firstMatch.exists, "Lock down asks nothing")
        shot("autopilot-lockdown")
        tap("endLockdown")
        // Lockdown replaced the mode picked before it, so it ends in Manual (as on Android).
        waitFor("mode:lockdown")
        selected("mode:manual")
    }

    func testABypassCanRunUntilTurnedOff() {
        launch()
        tap("mode:bypass")
        tap("bypass:custom")
        waitFor("bypassForDialog")
        tap("bypassFor:untilOff")
        XCTAssertTrue(waitFor("untilOffWarning").label.contains("Everything is approved without asking until you stop it."))
        shot("autopilot-bypass-for")
        tap("closeBypassFor")
        // Back on the first sheet, Custom shows the choice.
        XCTAssertTrue(waitFor("bypass:custom").label.contains("Until off"))
        waitFor("confirmBypass").press(forDuration: 2.6)
        waitFor("bypassClock")
        text("On until you stop it")
        shot("autopilot-bypass-until-off")
        tap("stopBypass")
        waitFor("mode:bypass")
    }

    func testACustomLengthStartsFromItsOwnSheet() {
        launch()
        tap("mode:bypass")
        tap("bypass:custom")
        waitFor("bypassForDialog")
        let stepper = waitFor("bypassStepper")
        // iOS has no increment(): the stepper's "+" is its right end (one adjustable element to VoiceOver).
        stepper.coordinate(withNormalizedOffset: CGVector(dx: 0.92, dy: 0.5)).tap()
        XCTAssertEqual(stepper.value as? String, "4 days")
        waitFor("confirmBypassFor").press(forDuration: 2.6)
        waitFor("bypassClock")
        text("Approving without asking")
        tap("stopBypass")
    }

    func testTheModelDownloadsWithProgressAndCanBeDeleted() {
        launch(["-demoNoModel"])
        text("Get Autopilot's score")
        XCTAssertTrue(waitFor("mode:auto").label.contains("Needs the model"))
        shot("autopilot-no-model")
        tap("downloadModel")
        waitFor("modelProgress")
        shot("autopilot-downloading")
        waitFor("modelInstalled", timeout: 15)
        waitFor("stat:scored")
        tap("modelRow")
        waitFor("modelSheet")
        tap("deleteModel")
        XCTAssertTrue(app.alerts["Delete the model?"].waitForExistence(timeout: 5))
        app.alerts.buttons["Delete"].tap()
        waitFor("downloadModelDetails")
        tap("closeModel")
        waitFor("downloadModel")
    }

    func testAProfileShowsItsKindsAndThresholds() {
        launch()
        scrollTo("profile:personal")
        shot("autopilot-profiles")
        tap("profile:personal")
        waitFor("class:github/write/push")
        waitFor("statMemory")
        shot("profile")
        tap("class:gmail/read")
        tap("lock:off:gmail/read")
        XCTAssertTrue(app.alerts.firstMatch.waitForExistence(timeout: 5))
        app.alerts.buttons["Unlock"].tap()
        text("Unlocked by you")
        tap("preset:cautious")
        selected("preset:cautious")
        tap("preset:custom")
        waitFor("approveAt")
        waitFor("replay")
        text("decided")
        // iOS has no increment(): drag the threshold a little to the right.
        let approveAt = element("approveAt")
        approveAt.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: approveAt.coordinate(withNormalizedOffset: CGVector(dx: 0.6, dy: 0.5)))
        shot("profile-custom")
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
