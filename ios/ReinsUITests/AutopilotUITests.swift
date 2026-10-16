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
        open(extra)
        _ = waitFor("modeHero")
    }

    /// With `-demoMode bypass|overnight|lockdown` Trust shows the running field where the mode hero was.
    private func launchRunning(_ extra: [String]) {
        open(extra)
        _ = waitFor("runningField")
    }

    private func open(_ extra: [String]) {
        app.launchArguments = ["-demo", "-noPopup", "-noLiveActivities"] + extra
        app.launch()
        // Autopilot is part of Trust (a tab on a phone, the sidebar on a regular width).
        XCTAssertTrue(app.openTab("trust"), "no Trust tab")
    }

    /// Waits for the running field to be open, folding or compact (its accessibility value).
    private func waitForFold(_ state: String, timeout: TimeInterval = 8) {
        let field = element("runningField")
        let reached = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", state), object: field)
        XCTAssertEqual(XCTWaiter().wait(for: [reached], timeout: timeout), .completed, "the field never became \(state): \(String(describing: field.value))")
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

    /// As long as `waitFor`: on a busy Mac a switch can take more than 5 s to show.
    private func selected(_ id: String, timeout: TimeInterval = 8) {
        expectation(for: NSPredicate(format: "isSelected == true"), evaluatedWith: element(id))
        waitForExpectations(timeout: timeout)
    }

    private func notSelected(_ id: String, timeout: TimeInterval = 8) {
        expectation(for: NSPredicate(format: "isSelected == false"), evaluatedWith: element(id))
        waitForExpectations(timeout: timeout)
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
            launchRunning(["-demoMode", mode])
            waitForFold("open")
            if let clock { waitFor(clock) }
            shot("trust-\(mode)-open")

            // A slow drag held before letting go stops without momentum, half way through the fold.
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.72))
            from.press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.64)), withVelocity: .slow, thenHoldForDuration: 0.4)
            waitForFold("folding")
            shot("trust-\(mode)-folding")

            app.swipeUp()
            app.swipeUp()
            waitForFold("compact")
            if let clock { waitFor(clock) }
            XCTAssertTrue(element("heroMode").exists, "the sentence keeps its label when compact")
            let stop = waitFor(button)
            // To the point: a frame measured between fractional positions reads 43.999999999999986.
            XCTAssertGreaterThanOrEqual(stop.frame.height.rounded(), 44, "\(button) stays a full touch target")
            XCTAssertLessThan(stop.frame.maxY, app.frame.minY + 200, "\(button) moved up into the bar")
            shot("trust-\(mode)-compact")

            for _ in 0..<4 { app.swipeDown() }
            waitForFold("open")

            // Compact again, a finger on the button stops the mode. (XCUITest's isHittable says no here: the faded
            // sentence, kept for VoiceOver and passing touches through, overlaps the button's frame. The tap proves it.)
            app.swipeUp()
            app.swipeUp()
            waitForFold("compact")
            let compactStop = waitFor(button)
            compactStop.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
            XCTAssertTrue(element("runningField").waitForNonExistence(timeout: 8), "\(button) in the compact bar stopped \(mode)")
            app.terminate()
        }
    }

    /// A bypass that approved 100 requests lists the latest three; "Show all 97" opens the rest in place and "Show less"
    /// folds them back. Trust opens folded again.
    func testABypassListFoldsToItsLatestThree() {
        launchRunning(["-demoMode", "bypass", "-demoRunningEntries", "100"])
        let showAll = waitFor("runningShowAll")
        // The mood line says what the bypass did (12 minutes of it; all of today unless the test runs just after midnight).
        XCTAssertTrue(waitFor("trustMood").label.hasPrefix("Bypass approved"), "the mood line counts the bypass, not Autopilot")
        XCTAssertEqual(app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'runningEntry:'")).count, 3)
        shot("trust-bypass-list-folded")
        // The 100 seeded and whatever else the demo's bypass decided in its 12 minutes, all but the three shown.
        let hidden = Int(showAll.label.replacingOccurrences(of: "Show all ", with: "")) ?? 0
        XCTAssertGreaterThanOrEqual(hidden, 97, showAll.label)
        showAll.tap()
        // Open, the list runs on (Show less is at its far end, not yet laid out): more rows than the three, and no Show all.
        XCTAssertTrue(showAll.waitForNonExistence(timeout: 5), "Show all opened the list")
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
        // The button itself: its list cell carries the same identifier and has no frame of its own.
        let add = app.buttons["addMcp"].firstMatch
        XCTAssertTrue(add.waitForExistence(timeout: 8), "Add MCP server never showed")
        XCTAssertLessThanOrEqual(add.frame.maxY, app.tab("connections").frame.minY, "Add MCP server clears the tab bar")
        XCTAssertTrue(add.isHittable)
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
