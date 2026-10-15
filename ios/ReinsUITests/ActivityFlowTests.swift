import XCTest

/// The Activity section and the decision sheets, driven over the `-demo` core (the Android app's AppFlowTest cases
/// for these screens). Set `REINS_SHOTS=/some/dir` in the test environment to keep a screenshot of each step.
final class ActivityFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ link: String? = nil, extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        // A frozen clock keeps the countdowns still, so the app goes idle between steps.
        app.launchArguments = ["-demo", "-noPopup", "-freezeTime", String(Int(Date().timeIntervalSince1970))] + extra + (link.map { ["-open", $0] } ?? [])
        app.launch()
        // The first launch of a run can race xcodebuild still installing the app, which then comes up again without
        // these arguments (on the real sign-in screen): once, launch it again, and say so in the log. The demo shows
        // its tab bar, or a linked item's sheet over it.
        let demo = app.descendants(matching: .any).matching(NSPredicate(
            format: "elementType == %d OR identifier IN %@",
            XCUIElement.ElementType.tabBar.rawValue, ["approvalSheet", "pairingSheet", "uploadSheet"]
        )).firstMatch
        if !demo.waitForExistence(timeout: 20) {
            XCTContext.runActivity(named: "Relaunching: the demo's first screen did not appear") { activity in
                let screen = XCTAttachment(screenshot: app.screenshot())
                screen.name = "before relaunch"
                screen.lifetime = .keepAlways
                activity.add(screen)
                print("ActivityFlowTests: relaunching \(name), the demo's first screen did not appear")
                app.terminate()
                app.launch()
            }
        }
        return app
    }

    private func shot(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let dir = ProcessInfo.processInfo.environment["REINS_SHOTS"] {
            try? app.screenshot().pngRepresentation.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name).png"))
        }
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
    }

    /// Waits for `element` with an explicit expectation, long enough for a simulator sharing the Mac with others.
    private func expectToAppear(_ element: XCUIElement, timeout: TimeInterval = 30) {
        let shown = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true"), object: element)
        XCTAssertEqual(XCTWaiter().wait(for: [shown], timeout: timeout), .completed, "\(element) never appeared")
    }

    /// The core's word about syncing with the other phone is a row on Activity until OK dismisses it.
    func testTheSyncNoticeShowsUntilOK() {
        let app = launch(extra: ["-demoSyncNotice"])
        let row = element(app, "syncNoticeBanner")
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Synced with your other phone. A backup of this phone's changes is kept."].exists)
        shot(app, "activity-sync-notice")
        row.buttons["OK"].tap()
        XCTAssertTrue(row.waitForNonExistence(timeout: 5))
    }

    func testWaitingRequestsComeAboveTheHistoryAndOpenTheirSheet() {
        let app = launch()
        let card = element(app, "pending:req1")
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "modePill").exists)
        shot(app, "activity-list")
        // The card's sentence opens the full screen; its buttons answer.
        element(app, "pendingTitle:req1").tap()
        XCTAssertTrue(element(app, "approvalSheet").waitForExistence(timeout: 5))
        XCTAssertTrue(element(app, "waitLine").exists)
        shot(app, "activity-sheet-search")
    }

    /// The mode pill drops the compact Autopilot menu, not the page; "Autopilot settings" opens the page.
    func testTheModePillOpensTheQuickMenuAndMoreOpensAutopilot() {
        let app = launch()
        let pill = element(app, "modePill")
        XCTAssertTrue(pill.waitForExistence(timeout: 10))
        pill.tap()
        XCTAssertTrue(element(app, "autopilotQuick").waitForExistence(timeout: 3), "the menu drops from the pill")
        for id in ["quick:MANUAL", "quick:AUTO", "quick:OVERNIGHT", "quick:BYPASS", "quick:LOCKDOWN", "quickMore"] {
            XCTAssertTrue(element(app, id).exists, "the menu has \(id)")
        }
        XCTAssertFalse(element(app, "modeHero").exists, "the page waits for Autopilot settings")
        shot(app, "activity-quick-menu")
        element(app, "quickMore").tap()
        XCTAssertTrue(element(app, "modeHero").waitForExistence(timeout: 5))
    }

    /// Manual → Auto → Manual → Auto from the menu: each time the pick is the card in force and the other is not, both
    /// enabled; Bypass opens the same sheet as the Autopilot page.
    func testTheQuickMenuSwitchesModesAndBypassAsksForALength() {
        let app = launch()
        let pill = element(app, "modePill")
        XCTAssertTrue(pill.waitForExistence(timeout: 10))
        for (picked, other) in [("AUTO", "MANUAL"), ("MANUAL", "AUTO"), ("AUTO", "MANUAL")] {
            pill.tap()
            XCTAssertTrue(element(app, "quick:\(picked)").waitForExistence(timeout: 3))
            element(app, "quick:\(picked)").tap()
            XCTAssertTrue(element(app, "autopilotQuick").waitForNonExistence(timeout: 3), "a pick closes the menu")
            pill.tap()
            let on = element(app, "quick:\(picked)")
            let off = element(app, "quick:\(other)")
            XCTAssertTrue(on.waitForExistence(timeout: 3))
            XCTAssertTrue(on.isSelected, "\(picked) is in force")
            XCTAssertFalse(off.isSelected, "\(other) is not")
            XCTAssertTrue(on.isEnabled && off.isEnabled, "both stay enabled")
            element(app, "quickScrim").tap()
            XCTAssertTrue(element(app, "autopilotQuick").waitForNonExistence(timeout: 3), "the scrim closes the menu")
        }
        pill.tap()
        element(app, "quick:BYPASS").tap()
        XCTAssertTrue(element(app, "confirmBypass").waitForExistence(timeout: 5), "the shared bypass sheet")
        XCTAssertTrue(element(app, "bypass:custom").exists, "with its custom lengths")
        shot(app, "activity-quick-bypass-sheet")
    }

    /// Overnight from the menu opens its own sheet: its lengths, what to do with what is too risky, the hold.
    func testTheQuickMenuOffersOvernight() {
        let app = launch()
        let pill = element(app, "modePill")
        XCTAssertTrue(pill.waitForExistence(timeout: 10))
        pill.tap()
        element(app, "quick:OVERNIGHT").tap()
        XCTAssertTrue(element(app, "confirmOvernight").waitForExistence(timeout: 5), "the Overnight sheet")
        XCTAssertTrue(element(app, "overnight:custom").exists, "with its custom lengths")
        XCTAssertTrue(element(app, "whenRisky").exists, "and what to do with what is too risky")
        shot(app, "activity-quick-overnight-sheet")
    }

    /// A request the model rated shows its risk, and that Overnight held it.
    func testARatedRequestShowsItsRiskAndThatOvernightHeldIt() {
        let app = launch("reins://item?kind=request&id=req10")
        XCTAssertTrue(element(app, "approvalSheet").waitForExistence(timeout: 10))
        let risk = element(app, "risk")
        XCTAssertTrue(risk.waitForExistence(timeout: 5))
        XCTAssertTrue(risk.label.contains("Risk: severe"))
        XCTAssertTrue(element(app, "riskHeld").exists, "held by Overnight")
        shot(app, "activity-sheet-risk")
    }

    func testTheDockOffersHowLongAndApprovingClosesTheScreen() {
        let app = launch("reins://item?kind=request&id=req1")
        XCTAssertTrue(element(app, "approvalSheet").waitForExistence(timeout: 10))
        let hour = element(app, "lifetime:hour")
        XCTAssertTrue(hour.waitForExistence(timeout: 3), "how long is in the dock")
        XCTAssertFalse(element(app, "moreToggle").exists, "what it covers waits under Custom")
        element(app, "lifetimeCustom").tap()
        XCTAssertTrue(element(app, "moreToggle").waitForExistence(timeout: 3))
        shot(app, "activity-sheet-custom")
        element(app, "closeOverlay").tap()
        hour.tap()
        shot(app, "activity-sheet-hour")
        element(app, "approve").tap()
        XCTAssertTrue(element(app, "approvalSheet").waitForNonExistence(timeout: 5))
        XCTAssertFalse(element(app, "pending:req1").exists)
    }

    func testClearThenApproveAsksForAtLeastOneEmail() {
        // req4 has no email a grant already covers (those stay shared whatever is ticked).
        let app = launch("reins://item?kind=request&id=req4")
        XCTAssertTrue(element(app, "clearAll").waitForExistence(timeout: 10))
        element(app, "clearAll").tap()
        element(app, "approve").tap()
        XCTAssertTrue(element(app, "approvalError").waitForExistence(timeout: 3))
        XCTAssertTrue(element(app, "approvalSheet").exists)
        shot(app, "activity-sheet-error")
    }

    func testAPermissionCanOnlyBeShortened() {
        let app = launch("reins://item?kind=request&id=req3")
        XCTAssertTrue(element(app, "grantCard").waitForExistence(timeout: 10))
        element(app, "grantPeriod").tap()
        let slider = app.sliders["grantSlider"]
        XCTAssertTrue(slider.waitForExistence(timeout: 3))
        slider.adjust(toNormalizedSliderPosition: 0)
        shot(app, "activity-sheet-shorten")
        XCTAssertEqual(element(app, "grantDuration").label, "1 minute")
    }

    func testPairingNeedsTheMatchingCode() {
        let app = launch("reins://item?kind=pairing&id=pair1")
        let code = element(app, "code:42")
        XCTAssertTrue(code.waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "approve").isEnabled)
        code.tap()
        shot(app, "activity-pairing-picked")
        element(app, "approve").tap()
        XCTAssertTrue(element(app, "pairingSheet").waitForNonExistence(timeout: 5))
    }

    func testAnEntryOpensAndAnEmailInItOpensInFull() {
        let app = launch("reins://activity?id=10")
        let message = element(app, "detailMessage:0")
        expectToAppear(message)
        shot(app, "activity-entry")
        message.tap()
        expectToAppear(element(app, "emailBody"))
        shot(app, "activity-email")
    }

    /// A request decided here opens from Activity as its approval showed it: the same hero and facts, read-only, with
    /// how it ended on the hero and no way to answer it again.
    func testADecidedRequestOpensAsItsApprovalReadOnly() {
        let app = launch("reins://item?kind=request&id=req1")
        XCTAssertTrue(element(app, "approvalSheet").waitForExistence(timeout: 10))
        element(app, "approve").tap()
        XCTAssertTrue(element(app, "approvalSheet").waitForNonExistence(timeout: 5))
        // The seeded history ends at 14: the decision is the next entry. The history sits below the waiting cards in a
        // lazy list, so its rows only exist once scrolled to.
        let row = element(app, "entry:15")
        var swipes = 0
        while !row.isHittable && swipes < 12 {
            app.swipeUp()
            swipes += 1
        }
        XCTAssertTrue(row.waitForExistence(timeout: 8))
        row.tap()
        XCTAssertTrue(element(app, "entryApproval").waitForExistence(timeout: 5))
        // Decided means paper: the entry's own header, not the approval's field.
        XCTAssertTrue(element(app, "entryHero").exists)
        XCTAssertFalse(element(app, "hero").exists)
        XCTAssertTrue(element(app, "entryOutcome").exists, "how it ended, under its line")
        XCTAssertFalse(element(app, "selectAll").exists, "nothing to tick again")
        XCTAssertFalse(element(app, "clearAll").exists)
        XCTAssertFalse(element(app, "approve").exists, "no decision dock")
        XCTAssertFalse(element(app, "waitLine").exists, "no timer")
        shot(app, "activity-entry-approval")
    }

    func testThisWasWrongAsksAndThanks() {
        let app = launch("reins://activity?id=13")
        let wrong = element(app, "thisWasWrong")
        XCTAssertTrue(wrong.waitForExistence(timeout: 10))
        wrong.tap()
        let confirm = app.alerts.buttons["Approve next time"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 3))
        shot(app, "activity-correct-ask")
        confirm.tap()
        XCTAssertTrue(element(app, "corrected").waitForExistence(timeout: 5))
    }

    func testDenyIsOneTap() {
        let app = launch("reins://item?kind=blob&id=blob_q3numbers")
        XCTAssertTrue(element(app, "uploadSheet").waitForExistence(timeout: 10))
        element(app, "deny").tap()
        XCTAssertTrue(element(app, "uploadSheet").waitForNonExistence(timeout: 5))
        XCTAssertFalse(element(app, "pending:blob_q3numbers").exists)
    }
}
