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
        app.launchArguments = ["-demo", "-noPopup", "-noLiveActivities"] + extra
        app.launch()
        // Autopilot is part of Trust (a tab on a phone, the sidebar on a regular width).
        XCTAssertTrue(app.openTab("trust"), "no Trust tab")
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

    private func notSelected(_ id: String) {
        expectation(for: NSPredicate(format: "isSelected == false"), evaluatedWith: element(id))
        waitForExpectations(timeout: 5)
    }

    /// Manual → Auto → Manual → Auto: each time the card in force is selected, the other is not, and both stay
    /// enabled (nothing is left dimmed or pressed).
    func testManualAndAutoSwitchBackAndForth() {
        launch()
        waitFor("mode:manual")
        selected("mode:manual")
        for (picked, other) in [("auto", "manual"), ("manual", "auto"), ("auto", "manual")] {
            tap("mode:\(picked)")
            selected("mode:\(picked)")
            notSelected("mode:\(other)")
            XCTAssertTrue(element("mode:\(picked)").isEnabled, "\(picked) stays enabled")
            XCTAssertTrue(element("mode:\(other)").isEnabled, "\(other) stays enabled")
            XCTAssertTrue(element("mode:\(other)").isHittable, "\(other) can be tapped again")
        }
        shot("autopilot-switched")
    }

    /// "Mode" has its (?) beside it, and what the modes do opens under it.
    func testTheModeHeaderExplainsTheModes() {
        launch()
        let header = waitFor("modeHeader")
        header.buttons["help"].firstMatch.tap()
        XCTAssertTrue(waitFor("modeHelp").label.contains("Lock down denies everything that would ask"))
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

    /// Overnight opens the bypass sheet in its own look: its (?) says it decides like Auto when the model does not rate
    /// risk (the demo's does not), it takes what to do with what is too risky, it is held to start, runs with its ring
    /// and stops back to the mode it interrupted.
    func testOvernightIsHeldRunsAndStopsBackToTheModeItInterrupted() {
        launch()
        tap("mode:auto")
        selected("mode:auto")
        tap("mode:overnight")
        let sheet = waitFor("overnightDialog")
        waitFor("overnight:480")
        selected("overnight:480")
        sheet.buttons["help"].firstMatch.tap()
        XCTAssertTrue(waitFor("helpText").label.contains("decides like Auto"))
        selected("whenRisky:wait")
        tap("whenRisky:deny")
        selected("whenRisky:deny")
        shot("autopilot-overnight-sheet")
        // A tap is not enough: held for two seconds.
        waitFor("confirmOvernight").tap()
        XCTAssertTrue(element("confirmOvernight").exists, "a tap does not start Overnight")
        waitFor("confirmOvernight").press(forDuration: 2.6)
        waitFor("overnightClock")
        text("Holds what is too risky")
        selected("whenRisky:deny")
        shot("autopilot-overnight")
        tap("stopOvernight")
        selected("mode:auto")
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
        // Installed: the week's numbers take the offer's place; the model's row is further down Trust, under the grants.
        waitFor("stat:scored", timeout: 15)
        scrollTo("modelInstalled")
        waitFor("modelInstalled")
        tap("modelRow")
        waitFor("modelSheet")
        tap("deleteModel")
        XCTAssertTrue(app.alerts["Delete the model?"].waitForExistence(timeout: 5))
        app.alerts.buttons["Delete"].tap()
        waitFor("downloadModelDetails")
        tap("closeModel")
        app.swipeDown()
        app.swipeDown()
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

    /// A running bypass, Overnight or lockdown folds into a compact bar as Trust scrolls: the label line and the timer
    /// stay, the button moves to the timer's right and stays a full target; back at the top it opens again.
    func testTheRunningFieldFoldsIntoACompactBarOnScroll() {
        for (mode, button, clock) in [("bypass", "stopBypass", "bypassClock"), ("overnight", "stopOvernight", "overnightClock"), ("lockdown", "endLockdown", nil)] {
            launch(["-demoMode", mode])
            waitFor("runningFieldOpen")
            if let clock { waitFor(clock) }
            shot("trust-\(mode)-open")

            // A slow drag held before letting go stops without momentum, half way through the fold.
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.72))
            from.press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.64)), withVelocity: .slow, thenHoldForDuration: 0.4)
            waitFor("runningFieldFolding")
            shot("trust-\(mode)-folding")

            app.swipeUp()
            app.swipeUp()
            waitFor("runningFieldCompact")
            if let clock { waitFor(clock) }
            XCTAssertTrue(element("heroMode").exists, "the sentence keeps its label when compact")
            let stop = waitFor(button)
            XCTAssertTrue(stop.isHittable, "\(button) is reachable in the compact bar")
            XCTAssertGreaterThanOrEqual(stop.frame.height, 44, "\(button) stays a full touch target")
            XCTAssertLessThan(stop.frame.maxY, app.frame.minY + 200, "\(button) moved up into the bar")
            shot("trust-\(mode)-compact")

            for _ in 0..<4 { app.swipeDown() }
            waitFor("runningFieldOpen")
            app.terminate()
        }
    }

    /// A bypass that approved 100 requests lists the latest three; "Show all 97" opens the rest in place and "Show less"
    /// folds them back. Trust opens folded again.
    func testABypassListFoldsToItsLatestThree() {
        launch(["-demoMode", "bypass", "-demoRunningEntries", "100"])
        let showAll = waitFor("runningShowAll")
        XCTAssertEqual(showAll.label, "Show all 97")
        XCTAssertEqual(app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'runningEntry:'")).count, 3)
        shot("trust-bypass-list-folded")
        showAll.tap()
        waitFor("runningShowLess")
        XCTAssertGreaterThan(app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'runningEntry:'")).count, 3)
        shot("trust-bypass-list-open")
        XCTAssertTrue(app.openTab("activity"))
        XCTAssertTrue(app.openTab("trust"))
        waitFor("runningShowAll")
    }

    /// Scrolled to the end, the last row of a root page sits fully above the floating tab bar: Trust's starting rule,
    /// Connections' "Add MCP server".
    func testNothingHidesUnderTheTabBar() {
        launch()
        for _ in 0..<8 { app.swipeUp() }
        let bar = app.tab("trust")
        XCTAssertTrue(bar.exists)
        XCTAssertLessThanOrEqual(waitFor("startingRule").frame.maxY, bar.frame.minY, "Trust's last row clears the tab bar")
        shot("trust-bottom")

        XCTAssertTrue(app.openTab("connections"))
        for _ in 0..<8 { app.swipeUp() }
        XCTAssertLessThanOrEqual(waitFor("addMcp").frame.maxY, app.tab("connections").frame.minY, "Add MCP server clears the tab bar")
        XCTAssertTrue(element("addMcp").isHittable)
        shot("connections-bottom")
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
