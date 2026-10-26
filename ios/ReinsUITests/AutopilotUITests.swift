import XCTest

/// Walks Autopilot in the `-demo` app the way a user would: Manual and Auto, "For a while" (a bypass held to confirm,
/// timed and until turned off; Overnight and Lock down on a tap), the running field's collapse, the model, a profile
/// and its thresholds, "Try it". Screenshots go to the test's attachments and, when `REINS_SHOTS` names a folder, there too.
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

    private func element(_ id: String) -> XCUIElement {
        // The mode's stop control is a Button in both open and compact fields.
        // Avoid an Any query traversing hidden stacks while that button is visible.
        if ["stopBypass", "stopOvernight", "endLockdown"].contains(id) {
            return app.buttons.matching(identifier: id).firstMatch
        }
        return app.descendants(matching: .any)[id].firstMatch
    }

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

    /// Hidden tab stacks stay mounted. Gesture only on the list accepting touches.
    private func visibleList() -> XCUIElement {
        let list = app.collectionViews.allElementsBoundByIndex.first { $0.isHittable }
        XCTAssertNotNil(list, "the current tab has no hittable list")
        return list ?? app.collectionViews.firstMatch
    }

    /// Lazy rows may briefly return an invalid AX frame even when their pixels are visible.
    /// Reacquire the actual control and require finite geometry and a usable target before measuring it.
    @discardableResult
    private func scrollTo(_ id: String, type: XCUIElement.ElementType = .any, toEnd: Bool = false) -> CGRect {
        var previousFrame: CGRect?
        var unchangedScrolls = 0
        for attempt in 0...12 {
            var renderedFrame: CGRect?
            for target in app.descendants(matching: type).matching(identifier: id).allElementsBoundByIndex {
                let frame = target.frame
                if !frame.isNull, !frame.isInfinite, frame.width > 1, frame.height > 1,
                   frame.minX.isFinite, frame.minY.isFinite, frame.maxX.isFinite, frame.maxY.isFinite,
                   app.frame.contains(CGPoint(x: frame.midX, y: frame.midY)), target.isHittable {
                    if !toEnd { return frame }
                    renderedFrame = frame
                    break
                }
            }
            if let frame = renderedFrame {
                // Partly hittable can still mean behind the floating tab bar.
                // Reach the actual end before the separate inset assertion:
                // two more real list gestures must leave fresh bounds unchanged.
                if let previousFrame,
                   abs(frame.minY - previousFrame.minY) < 0.5,
                   abs(frame.maxY - previousFrame.maxY) < 0.5 {
                    unchangedScrolls += 1
                } else {
                    unchangedScrolls = 0
                }
                if unchangedScrolls >= 2 { return frame }
                previousFrame = frame
            } else {
                previousFrame = nil
                unchangedScrolls = 0
            }
            if attempt < 12 { visibleList().swipeUp() }
        }
        XCTFail("\(id) did not become rendered and hittable\(toEnd ? " at the list end" : "")")
        return .null
    }

    /// Reach the top through real scrolling, including any small inset offset after the field appears.
    private func scrollRunningFieldToTop() {
        for attempt in 0...12 {
            if element("runningField").value as? String == "open" { return }
            if attempt < 12 { visibleList().swipeDown() }
        }
        waitForFold("open")
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

    /// Opens "For a while" and picks one of its modes (Overnight, Lockdown or Bypass) from the iOS menu.
    private func pickTimed(_ mode: String) {
        tap("forAWhile")
        let byId = app.buttons.matching(identifier: "mode:\(mode)").firstMatch
        let item = byId.waitForExistence(timeout: 5) ? byId : app.buttons[mode.prefix(1).uppercased() + mode.dropFirst()].firstMatch
        XCTAssertTrue(item.waitForExistence(timeout: 5), "no \(mode) in the menu")
        item.tap()
    }

    /// Manual → Auto → Manual → Auto: each time the half in force is selected, the other is not, and both stay
    /// enabled (nothing is left dimmed or pressed).
    func testManualAndAutoSwitchBackAndForth() {
        launch()
        waitFor("modeSwitch:manual")
        selected("modeSwitch:manual")
        for (picked, other) in [("auto", "manual"), ("manual", "auto"), ("auto", "manual")] {
            tap("modeSwitch:\(picked)")
            selected("modeSwitch:\(picked)")
            notSelected("modeSwitch:\(other)")
            XCTAssertTrue(element("modeSwitch:\(picked)").isEnabled, "\(picked) stays enabled")
            XCTAssertTrue(element("modeSwitch:\(other)").isEnabled, "\(other) stays enabled")
            XCTAssertTrue(element("modeSwitch:\(other)").isHittable, "\(other) can be tapped again")
        }
        shot("autopilot-switched")
    }

    /// "For a while" opens the menu of the three timed modes.
    func testForAWhileOpensTheMenuOfTimedModes() {
        launch()
        tap("forAWhile")
        for mode in ["overnight", "lockdown", "bypass"] {
            XCTAssertTrue(app.buttons.matching(identifier: "mode:\(mode)").firstMatch.waitForExistence(timeout: 5), "\(mode) is in the menu")
        }
        shot("trust-for-a-while-menu")
    }

    func testBypassIsHeldAndLockDownIsAPlainTap() {
        launch()
        waitFor("modeSwitch:manual")
        selected("modeSwitch:manual")
        shot("autopilot-manual")
        tap("modeSwitch:auto")
        selected("modeSwitch:auto")
        pickTimed("bypass")
        waitFor("confirmBypass")
        shot("autopilot-bypass-sheet")
        tap("bypass:60")
        // A tap is not enough: held for two seconds.
        waitFor("confirmBypass").tap()
        XCTAssertTrue(element("confirmBypass").exists, "a tap does not start a bypass")
        waitFor("confirmBypass").press(forDuration: 2.6)
        waitFor("bypassClock")
        shot("autopilot-bypass")
        tap("stopBypass")
        selected("modeSwitch:auto")
        // Lock down: no dialog.
        pickTimed("lockdown")
        waitFor("endLockdown")
        XCTAssertFalse(app.alerts.firstMatch.exists, "Lock down asks nothing")
        shot("autopilot-lockdown")
        tap("endLockdown")
        // Lockdown replaced the mode picked before it, so it ends in Manual (as on Android).
        waitFor("modeSwitch:manual")
        selected("modeSwitch:manual")
    }

    /// Overnight starts on a tap from "For a while" (the owner's check is the core's), runs as Trust's field with what
    /// to do with what is too risky under it, and stops back to the mode it interrupted.
    func testOvernightStartsOnATapRunsAndStopsBackToTheModeItInterrupted() {
        launch()
        tap("modeSwitch:auto")
        selected("modeSwitch:auto")
        pickTimed("overnight")
        waitFor("overnightClock")
        selected("whenRisky:wait")
        tap("whenRisky:deny")
        selected("whenRisky:deny")
        shot("autopilot-overnight")
        tap("stopOvernight")
        selected("modeSwitch:auto")
    }

    func testABypassCanRunUntilTurnedOff() {
        launch()
        pickTimed("bypass")
        tap("bypass:custom")
        waitFor("bypassForDialog")
        tap("bypassFor:untilOff")
        XCTAssertTrue(waitFor("untilOffWarning").label.contains("Everything is approved without asking until you stop it."))
        shot("autopilot-bypass-for")
        tap("closeBypassFor")
        // Back on the first sheet, Custom shows the choice.
        XCTAssertTrue(waitFor("bypass:custom").label.contains("Until off"))
        waitFor("confirmBypass").press(forDuration: 2.6)
        XCTAssertTrue(waitFor("bypassClock").label.contains("until you stop it"))
        shot("autopilot-bypass-until-off")
        tap("stopBypass")
        waitFor("modeSwitch:manual")
    }

    func testACustomLengthStartsFromItsOwnSheet() {
        launch()
        pickTimed("bypass")
        tap("bypass:custom")
        waitFor("bypassForDialog")
        let stepper = waitFor("bypassStepper")
        // iOS has no increment(): the stepper's "+" is its right end (one adjustable element to VoiceOver).
        stepper.coordinate(withNormalizedOffset: CGVector(dx: 0.92, dy: 0.5)).tap()
        XCTAssertEqual(stepper.value as? String, "4 days")
        waitFor("confirmBypassFor").press(forDuration: 2.6)
        waitFor("bypassClock")
        tap("stopBypass")
    }

    func testTheModelDownloadsWithProgressAndCanBeDeleted() {
        launch(["-demoNoModel"])
        text("Auto needs the model")
        XCTAssertFalse(waitFor("modeSwitch:auto").isEnabled, "Auto waits for the model")
        shot("autopilot-no-model")
        tap("downloadModel")
        waitFor("modelProgress")
        shot("autopilot-downloading")
        // Installed: the offer goes; the model's row is further down Trust, under the grants.
        XCTAssertTrue(element("modelOffer").waitForNonExistence(timeout: 15), "the offer goes once the model is here")
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

    /// The model updates itself; its sheet has the switch, and "Check now" under More says what came of it.
    func testTheModelSheetUpdatesAutomaticallyAndChecksUnderMore() {
        launch()
        scrollTo("modelInstalled")
        waitFor("modelInstalled")
        tap("modelRow")
        waitFor("modelSheet")
        waitFor("autoUpdate")
        waitFor("modelUpdated")
        XCTAssertFalse(element("checkModelNow").exists, "More starts closed")
        tap("modelMore")
        tap("checkModelNow")
        waitFor("checkModelResult", timeout: 10)
        shot("autopilot-model-check")
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
        app.alerts.buttons["Trust"].tap()
        text("Trusted by you")
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

    /// A running bypass, Overnight or lockdown collapses in two steps as Trust scrolls: full, then short (the clock and
    /// Stop on one line), then mini (only the island's row, Stop a full target); back at the top it is full again.
    func testTheRunningFieldCollapsesInTwoStepsOnScroll() {
        for (mode, button, clock) in [("bypass", "stopBypass", "bypassClock"), ("overnight", "stopOvernight", "overnightClock"), ("lockdown", "endLockdown", nil)] {
            launchRunning(["-demoMode", mode])
            scrollRunningFieldToTop()
            waitForFold("open")
            if let clock { waitFor(clock) }
            shot("trust-\(mode)-open")

            // A slow drag held before letting go stops without momentum, past the full field's clock.
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75))
            from.press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.53)), withVelocity: .slow, thenHoldForDuration: 0.4)
            waitForFold("short")
            shot("trust-\(mode)-short")

            app.swipeUp()
            app.swipeUp()
            waitForFold("compact")
            if let clock { waitFor(clock) }
            let stop = waitFor(button)
            // To the point: a frame measured between fractional positions reads 43.999999999999986.
            XCTAssertGreaterThanOrEqual(stop.frame.height.rounded(), 44, "\(button) stays a full touch target")
            XCTAssertLessThan(stop.frame.maxY, app.frame.minY + 120, "\(button) moved up into the island's row")
            shot("trust-\(mode)-compact")

            scrollRunningFieldToTop()
            waitForFold("open")

            // Mini again, a finger on Stop stops the mode.
            app.swipeUp()
            app.swipeUp()
            waitForFold("compact")
            waitFor(button).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
            XCTAssertTrue(element("runningField").waitForNonExistence(timeout: 8), "\(button) in the mini bar stopped \(mode)")
            app.terminate()
        }
    }

    /// A bypass that approved 100 requests shows its newest with the rest stacked under it; "+99" opens them in place
    /// and Show less folds them back. Trust opens folded again.
    func testABypassListStacksUnderItsNewest() {
        launchRunning(["-demoMode", "bypass", "-demoRunningEntries", "100"])
        let showAll = waitFor("runningShowAll")
        XCTAssertEqual(app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'runningEntry:'")).count, 1)
        shot("trust-bypass-list-folded")
        // The 100 seeded and whatever else the demo's bypass decided in its 12 minutes, all but the one shown.
        let hidden = Int(showAll.label.replacingOccurrences(of: "Show all ", with: "")) ?? 0
        XCTAssertGreaterThanOrEqual(hidden, 99, showAll.label)
        showAll.tap()
        XCTAssertTrue(showAll.waitForNonExistence(timeout: 5), "+99 opened the stack")
        XCTAssertGreaterThan(app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'runningEntry:'")).count, 1)
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
        // The button itself: its list cell carries the same identifier and has no frame of its own.
        let addFrame = scrollTo("addMcp", type: .button, toEnd: true)
        XCTAssertLessThanOrEqual(addFrame.maxY, app.tab("connections").frame.minY, "Add MCP server clears the tab bar")
        shot("connections-bottom")
    }

    func testTryItJudgesAnExample() {
        launch()
        scrollTo("openTryIt")
        tap("openTryIt")
        waitFor("situation")
        // The examples scroll sideways: bring the fourth into view.
        let injection = waitFor("example:Injection attempt")
        for _ in 0..<3 where injection.frame.maxX > app.frame.maxX { waitFor("example:Push a branch").swipeLeft() }
        injection.tap()
        tap("evaluate")
        waitFor("verdictWord", timeout: 8)
        text("Deny")
        app.swipeUp()
        shot("try-it")
    }
}
