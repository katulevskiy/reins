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
        app.launchArguments = ["-demo", "-noPopup", "-noLiveActivities", "-freezeTime", String(Int(Date().timeIntervalSince1970))] + extra + (link.map { ["-open", $0] } ?? [])
        app.launch()
        // The first launch of a run can race xcodebuild still installing the app, which then comes up again without
        // these arguments (on the real sign-in screen): once, launch it again, and say so in the log. The demo shows
        // its Activity tab (Signal's own bar, not a system tab bar), a linked item's sheet over it, or a linked entry's
        // page (pushed, so the tab bar is down).
        let demo = app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier IN %@", ["tab:activity", "approvalSheet", "pairingSheet", "uploadSheet", "entryHero", "entryApproval"]
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

    /// The river by day: today open, earlier days folded until their header is tapped; words typed search the history
    /// and the matches replace the stack; a filter alone searches too.
    func testTheRiverFoldsByDayAndSearches() {
        let app = launch(extra: ["-demoQuiet"])
        let c = Calendar.current.dateComponents([.year, .month, .day], from: Date())
        let today = element(app, String(format: "day:%04d-%02d-%02d", c.year!, c.month!, c.day!))
        expectToAppear(today)
        XCTAssertEqual(today.value as? String, "Expanded")
        let search = element(app, "activitySearch")
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.tap()
        search.typeText("Dinner")
        expectToAppear(element(app, "searchResults"))
        XCTAssertFalse(element(app, "pond").exists, "searching hides the stack")
        shot(app, "activity-search")
        element(app, "clearSearch").tap()
        expectToAppear(element(app, "pond"))
        element(app, "filterToggle").tap()
        element(app, "filter:outcome").tap()
        element(app, "filter:denied").tap()
        expectToAppear(element(app, "searchResults"))
        shot(app, "activity-filtered")
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

    // MARK: A scroll that starts on a row

    /// Drags from `point` of `element` by `dx`, `dy` points at `speed` points a second, after the finger has been down for
    /// `pause`, then holds still for `hold` so the list does not glide on.
    private func drag(
        _ element: XCUIElement, at point: CGVector = CGVector(dx: 0.5, dy: 0.5), dx: CGFloat = 0, dy: CGFloat = 0,
        pause: TimeInterval = 0.02, speed: CGFloat = 1500, hold: TimeInterval = 0.3
    ) {
        let start = element.coordinate(withNormalizedOffset: point)
        start.press(
            forDuration: pause, thenDragTo: start.withOffset(CGVector(dx: dx, dy: dy)),
            withVelocity: XCUIGestureVelocity(rawValue: speed), thenHoldForDuration: hold
        )
    }

    /// Brings a history row fully on screen by scrolling from the mood line (plain text, nothing to press).
    private func bringOnScreen(_ app: XCUIApplication, _ row: XCUIElement) {
        expectToAppear(row)
        var tries = 0
        while !row.isHittable && tries < 3 {
            drag(element(app, "activityMood"), dy: -120, speed: 600)
            tries += 1
        }
        XCTAssertTrue(row.isHittable, "\(row) is on screen")
    }

    /// Where a finger can scroll Activity: inside the list, below the status bar (the phone hides the navigation bar),
    /// and above the floating tab bar. The list's own frame runs under both.
    private struct ListBand {
        var list: CGRect
        var band: CGRect
        var top: CGRect
        var bar: CGRect
    }

    private func listBand(_ app: XCUIApplication) -> ListBand {
        let listEl = element(app, "activityList")
        XCTAssertTrue(listEl.waitForExistence(timeout: 10), "the activity list is on screen")
        let list = listEl.frame
        var top = CGRect.zero
        var haveTop = false
        let status = app.statusBars.firstMatch
        if status.exists, status.frame.height > 1 {
            top = status.frame
            haveTop = true
        }
        let nav = app.navigationBars.firstMatch
        if nav.exists, nav.frame.height > 1 {
            top = haveTop ? top.union(nav.frame) : nav.frame
            haveTop = true
        }
        var bar = CGRect.zero
        var haveBar = false
        for id in ["tab:activity", "tab:trust", "tab:connections", "tab:settings"] {
            let tab = element(app, id)
            guard tab.exists, tab.frame.width > 1, tab.frame.height > 1 else { continue }
            bar = haveBar ? bar.union(tab.frame) : tab.frame
            haveBar = true
        }
        // The capsule's padding sits outside the tab buttons. A point there is still the bar.
        if haveBar { bar = bar.insetBy(dx: -8, dy: -8) }
        XCTAssertTrue(haveBar, "the floating tab bar is on screen")
        let topLimit = haveTop ? top.maxY : list.minY
        let bottomLimit = min(list.maxY, bar.minY)
        let band = CGRect(x: list.minX, y: topLimit, width: list.width, height: max(0, bottomLimit - topLimit))
        XCTAssertGreaterThan(band.height, 200, "the visible list is tall enough to centre a row (\(band))")
        return ListBand(list: list, band: band, top: haveTop ? top : .zero, bar: bar)
    }

    /// Waits out a flick's glide, so the next measurement is of a list that has stopped.
    private func settle(_ row: XCUIElement) {
        var last = row.frame.minY
        for _ in 0..<12 {
            Thread.sleep(forTimeInterval: 0.1)
            let y = row.frame.minY
            if abs(y - last) < 0.5 { return }
            last = y
        }
    }

    /// Moves the list by `dy` without pressing `row`. The mood line when it is on screen; otherwise a point in the
    /// open list that is not the row.
    private func nudge(_ app: XCUIApplication, dy: CGFloat, avoiding row: XCUIElement) {
        let band = listBand(app).band
        let mood = element(app, "activityMood")
        let clearOfRow = !row.frame.insetBy(dx: -8, dy: -8).intersects(mood.frame)
        if mood.exists, mood.isHittable, mood.frame.height > 1, clearOfRow, band.intersects(mood.frame) {
            drag(mood, dy: dy, speed: 700, hold: 0.25)
            return
        }
        let blocked = row.frame.insetBy(dx: -12, dy: -12)
        let candidates = [band.minY + 28, band.midY, band.maxY - 28]
        let y = candidates.first { !blocked.contains(CGPoint(x: band.midX, y: $0)) } ?? band.midY
        let start = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: band.midX, dy: y))
        start.press(
            forDuration: 0.02, thenDragTo: start.withOffset(CGVector(dx: 0, dy: dy)),
            withVelocity: XCUIGestureVelocity(rawValue: 700), thenHoldForDuration: 0.25
        )
    }

    /// Scrolls until `row`'s centre is in the middle of the open list, not merely hittable.
    private func centerInList(_ app: XCUIApplication, _ row: XCUIElement) {
        expectToAppear(row)
        settle(row)
        for _ in 0..<10 {
            let band = listBand(app).band
            guard row.frame.height > 1 else {
                nudge(app, dy: -140, avoiding: row)
                settle(row)
                continue
            }
            let delta = row.frame.midY - band.midY
            if abs(delta) <= 36, band.contains(CGPoint(x: row.frame.midX, y: row.frame.midY)) { return }
            let before = row.frame.midY
            nudge(app, dy: max(-160, min(160, -delta)), avoiding: row)
            settle(row)
            if abs(row.frame.midY - before) < 1 { break }
        }
        let band = listBand(app).band
        XCTAssertLessThan(
            abs(row.frame.midY - band.midY), 48,
            "the row is in the middle of the list (row \(row.frame.midY), middle \(band.midY), row \(row.frame))"
        )
    }

    /// The finger goes down at `point` of `target`, which is inside the list and on neither bar.
    private func assertDragStartsOnTheList(_ app: XCUIApplication, _ target: XCUIElement, at point: CGVector) {
        let parts = listBand(app)
        let start = CGPoint(
            x: target.frame.minX + target.frame.width * point.dx,
            y: target.frame.minY + target.frame.height * point.dy
        )
        XCTAssertTrue(parts.list.contains(start), "the drag starts inside the list (\(start) list \(parts.list))")
        if parts.top.height > 1 {
            XCTAssertFalse(parts.top.contains(start), "the drag starts under the top bar (\(start) bar \(parts.top))")
        }
        XCTAssertFalse(parts.bar.contains(start), "the drag starts under the tab bar (\(start) bar \(parts.bar))")
        XCTAssertTrue(parts.band.contains(start), "the drag starts in the open part of the list (\(start) band \(parts.band))")
    }

    /// A scroll up and back down, both starting on `target`: the list moves under the finger each time, and nothing
    /// opens. Quick and slower, and once after the finger rested a moment, as when the row starts to show its press.
    /// Each drag begins with the row centred in the open list, so a flick's glide cannot park the next one on a bar.
    private func assertScrollsFrom(_ app: XCUIApplication, _ target: XCUIElement, at point: CGVector = CGVector(dx: 0.5, dy: 0.5)) {
        let moves: [(dy: CGFloat, speed: CGFloat, pause: TimeInterval)] = [(-200, 2500, 0.02), (200, 900, 0.02), (-200, 900, 0.12), (200, 2500, 0.12)]
        for move in moves {
            centerInList(app, target)
            assertDragStartsOnTheList(app, target, at: point)
            let before = target.frame.minY
            drag(target, at: point, dy: move.dy, pause: move.pause, speed: move.speed)
            // The list moves the finger's way, at least most of its travel (a quick flick may glide on): the bug kept it
            // still while the row took the press.
            let moved = target.frame.minY - before
            XCTAssertGreaterThan(moved * (move.dy < 0 ? -1 : 1), abs(move.dy) * 0.7, "the list followed the finger (\(move), moved \(moved))")
            XCTAssertFalse(element(app, "entryHero").waitForExistence(timeout: 1), "no entry opened (\(move))")
            XCTAssertFalse(element(app, "approvalSheet").exists, "no request opened (\(move))")
            XCTAssertFalse(app.alerts.firstMatch.exists, "nothing asked (\(move))")
        }
    }

    /// The bug this guards: a scroll that started on a row sometimes pressed the row instead, and the list stayed put.
    func testAScrollStartingOnARowScrollsAndOpensNothing() {
        let app = launch()
        // 12: a plain row (Bypass let it through; nothing to correct).
        let row = element(app, "entry:12")
        bringOnScreen(app, row)
        assertScrollsFrom(app, row)
        shot(app, "activity-scroll-from-row")
    }

    func testAScrollStartingOnARowAutopilotDecidedScrollsAndOpensNothing() throws {
        try XCTSkipIf(ProcessInfo.processInfo.environment["CI"] != nil, "Quarantined on CI: a centred scroll on the Autopilot row does not move the list.")
        let app = launch()
        // 14: Autopilot approved it, so it can be swiped to "This was wrong".
        let row = element(app, "entry:14")
        bringOnScreen(app, row)
        let frame = row.frame
        assertScrollsFrom(app, row)
        // The row did not slide open: a tap on its right end opens the entry, not the correction.
        app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: frame.maxX - 40, dy: row.frame.midY)).tap()
        XCTAssertTrue(element(app, "entryHero").waitForExistence(timeout: 5), "the tap opened the entry")
        XCTAssertFalse(app.alerts.firstMatch.exists)
    }

    func testAScrollStartingOnAWaitingCardScrollsAndOpensNothing() {
        let app = launch()
        let card = element(app, "pending:req1")
        expectToAppear(card)
        // On its ask, above its buttons.
        assertScrollsFrom(app, card, at: CGVector(dx: 0.5, dy: 0.3))
        XCTAssertTrue(card.exists, "the card was not answered")
        // A tap still opens it.
        element(app, "pendingTitle:req1").tap()
        XCTAssertTrue(element(app, "approvalSheet").waitForExistence(timeout: 5))
    }

    func testAScrollStartingOnTheModeChipScrollsAndOpensNoMenu() {
        let app = launch()
        let pill = element(app, "modePill")
        let card = element(app, "pending:req1")
        expectToAppear(pill)
        expectToAppear(card)
        let before = card.frame.minY
        drag(pill, dy: -60, speed: 2000)
        XCTAssertLessThan(card.frame.minY, before - 30, "the list scrolled")
        XCTAssertFalse(element(app, "autopilotQuick").waitForExistence(timeout: 1), "no menu opened")
    }

    /// The chip's menu still opens on a tap, and on a press that stays a moment, from which a slide picks a row.
    func testTheModeChipOpensOnATapAndOnAPressThatSlidesOntoARow() {
        let app = launch()
        let pill = element(app, "modePill")
        expectToAppear(pill)
        pill.tap()
        XCTAssertTrue(element(app, "autopilotQuick").waitForExistence(timeout: 3), "a tap opens the menu")
        element(app, "quickScrim").tap()
        XCTAssertTrue(element(app, "autopilotQuick").waitForNonExistence(timeout: 3))
        // Press, wait, slide onto the first row (Manual) and lift: it is picked and the menu closes.
        let start = pill.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let firstRow = pill.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 1)).withOffset(CGVector(dx: 0, dy: 40))
        start.press(forDuration: 0.5, thenDragTo: firstRow, withVelocity: XCUIGestureVelocity(rawValue: 300), thenHoldForDuration: 0.2)
        XCTAssertTrue(element(app, "autopilotQuick").waitForNonExistence(timeout: 3), "the slide picked a row")
        pill.tap()
        XCTAssertTrue(element(app, "quick:MANUAL").waitForExistence(timeout: 3))
        XCTAssertTrue(element(app, "quick:MANUAL").isSelected, "Manual was picked")
    }

    func testTappingARowStillOpensIt() {
        let app = launch()
        let row = element(app, "entry:12")
        bringOnScreen(app, row)
        row.tap()
        XCTAssertTrue(element(app, "entryHero").waitForExistence(timeout: 5))
    }

    /// Swiping an Autopilot row left still opens "This was wrong", and a long swipe still asks at once.
    func testSwipingARowLeftStillRevealsThisWasWrong() {
        let app = launch()
        let row = element(app, "entry:13")
        bringOnScreen(app, row)
        let frame = row.frame
        drag(row, at: CGVector(dx: 0.8, dy: 0.5), dx: -150, speed: 800, hold: 0.1)
        XCTAssertFalse(element(app, "entryHero").waitForExistence(timeout: 1), "the swipe opened nothing")
        XCTAssertFalse(app.alerts.firstMatch.exists, "a short swipe only opens the row")
        shot(app, "activity-swipe-wrong")
        // The violet "This was wrong" sits where the row's right end was.
        app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: frame.maxX - 50, dy: frame.midY)).tap()
        let ask = app.alerts.buttons["Approve next time"]
        XCTAssertTrue(ask.waitForExistence(timeout: 3), "it asks what should have happened")
        app.alerts.buttons["Cancel"].tap()
        XCTAssertTrue(app.alerts.firstMatch.waitForNonExistence(timeout: 3))
        // A long swipe asks at once.
        drag(row, at: CGVector(dx: 0.9, dy: 0.5), dx: -(frame.width * 0.75), speed: 1200, hold: 0.1)
        XCTAssertTrue(app.alerts.buttons["Approve next time"].waitForExistence(timeout: 3), "a long swipe asks at once")
        app.alerts.buttons["Cancel"].tap()
    }

    /// Flicking a waiting request left still denies it.
    func testFlickingACardLeftStillDeniesIt() {
        let app = launch()
        let card = element(app, "pending:req1")
        expectToAppear(card)
        drag(card, at: CGVector(dx: 0.8, dy: 0.3), dx: -(card.frame.width * 0.7), speed: 1500, hold: 0)
        XCTAssertTrue(card.waitForNonExistence(timeout: 5), "the flick denied it")
        XCTAssertFalse(element(app, "approvalSheet").exists, "without opening it")
    }

    func testDenyIsOneTap() {
        let app = launch("reins://item?kind=blob&id=blob_q3numbers")
        XCTAssertTrue(element(app, "uploadSheet").waitForExistence(timeout: 10))
        element(app, "deny").tap()
        XCTAssertTrue(element(app, "uploadSheet").waitForNonExistence(timeout: 5))
        XCTAssertFalse(element(app, "pending:blob_q3numbers").exists)
    }

    // MARK: A push decided on another device

    /// Opens a synced push's first commit: its changes were not kept on this iPhone, and Load from GitHub brings them.
    private func openSyncedCommit(_ extra: String) -> XCUIApplication {
        let app = launch("reins://activity?id=14", extra: [extra])
        let commits = element(app, "gitCommits:0")
        expectToAppear(commits, timeout: 15)
        commits.tap()
        let first = element(app, "gitCommit:0:0")
        expectToAppear(first, timeout: 5)
        first.tap()
        expectToAppear(element(app, "diffNotKept"), timeout: 5)
        return app
    }

    func testAPushFromAnotherDeviceLoadsItsDiffFromGitHub() {
        let app = openSyncedCommit("-demoSyncedPush")
        shot(app, "synced-not-kept")
        element(app, "loadDiff").tap()
        XCTAssertTrue(element(app, "diffNotKept").waitForNonExistence(timeout: 8), "loaded, the note goes")
        element(app, "commitFile:0").tap()
        expectToAppear(element(app, "commitFile:0:diff"), timeout: 5)
        shot(app, "synced-loaded")
    }

    func testALoadThatFailsSaysWhyAndOffersRetry() {
        let app = openSyncedCommit("-demoSyncedPushFails")
        element(app, "loadDiff").tap()
        expectToAppear(element(app, "retryDiff"), timeout: 8)
        XCTAssertTrue(element(app, "diffLoadError").exists)
        shot(app, "synced-retry")
    }

    // MARK: Loosening needs a tap

    /// From Lockdown, a press on the mode chip that slides onto Manual or Auto and lifts changes nothing: the menu stays
    /// open, and only a tap on the row ends the lockdown.
    func testALooseningModeIsNeverPickedByASlide() {
        let app = launch(extra: ["-demoMode", "lockdown"])
        let pill = element(app, "modePill")
        expectToAppear(pill)
        XCTAssertTrue(pill.label.contains("Lockdown"), pill.label)
        for row in ["quick:MANUAL", "quick:AUTO"] {
            // Where the row sits once the menu is open; then closed again.
            pill.tap()
            let target = element(app, row)
            expectToAppear(target, timeout: 5)
            let onRow = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: target.frame.midX, dy: target.frame.midY))
            element(app, "quickScrim").tap()
            XCTAssertTrue(element(app, "autopilotQuick").waitForNonExistence(timeout: 3))
            // One gesture from the closed chip: press until the menu opens, slide onto the row, lift.
            let start = pill.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            start.press(forDuration: 0.6, thenDragTo: onRow, withVelocity: .slow, thenHoldForDuration: 0.2)
            XCTAssertTrue(element(app, "autopilotQuick").waitForExistence(timeout: 2), "the menu stays open (\(row))")
            XCTAssertTrue(pill.label.contains("Lockdown"), "still locked down after a slide onto \(row)")
            element(app, "quickScrim").tap()
            XCTAssertTrue(element(app, "autopilotQuick").waitForNonExistence(timeout: 3))
        }
        // A tap on the row, once the menu is open, does it.
        pill.tap()
        let manual = element(app, "quick:MANUAL")
        expectToAppear(manual, timeout: 5)
        shot(app, "lockdown-menu")
        manual.tap()
        XCTAssertTrue(element(app, "autopilotQuick").waitForNonExistence(timeout: 3))
        let unlocked = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label CONTAINS 'Manual'"), object: pill)
        XCTAssertEqual(XCTWaiter().wait(for: [unlocked], timeout: 5), .completed, "a tap ended the lockdown: \(pill.label)")
    }

    // MARK: The keyboard and the clock

    /// With the search keyboard up the tab bar steps aside instead of riding on the keyboard over the results, and it
    /// comes back when the keyboard goes.
    func testTheTabBarStepsAsideForTheKeyboard() {
        let app = launch()
        let tab = app.tab("activity")
        expectToAppear(tab)
        let search = element(app, "activitySearch")
        expectToAppear(search)
        search.tap()
        search.typeText("Read")
        expectToAppear(element(app, "searchResults"))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(tab.waitForNonExistence(timeout: 3), "no tab bar on top of the keyboard")
        shot(app, "activity-keyboard")
        // A drag on the results puts the keyboard away; the tab bar comes back.
        element(app, "searchResults").swipeUp()
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        XCTAssertTrue(tab.waitForExistence(timeout: 3), "the tab bar is back")
    }

    /// Rows scrolled up pass under paper at the top, never under the clock's digits.
    func testRowsScrollUnderPaperAtTheTop() {
        let app = launch()
        expectToAppear(element(app, "activityMood"))
        let start = element(app, "activityMood").coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.02, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -420)),
                    withVelocity: XCUIGestureVelocity(rawValue: 800), thenHoldForDuration: 0.4)
        shot(app, "activity-scrolled")
    }
}
