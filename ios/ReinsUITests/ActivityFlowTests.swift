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

    func testWaitingRequestsComeAboveTheHistoryAndOpenTheirSheet() {
        let app = launch()
        let card = element(app, "pending:req1")
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        XCTAssertTrue(element(app, "modePill").exists)
        shot(app, "activity-list")
        card.tap()
        XCTAssertTrue(element(app, "approvalSheet").waitForExistence(timeout: 5))
        XCTAssertTrue(element(app, "waitLine").exists)
        shot(app, "activity-sheet-search")
    }

    func testMoreOptionsHoldTheStandingChoicesAndApprovingClosesTheSheet() {
        let app = launch("reins://item?kind=request&id=req1")
        XCTAssertTrue(element(app, "approvalSheet").waitForExistence(timeout: 10))
        XCTAssertFalse(element(app, "lifetime:hour").exists, "the sheet shows only approve and deny until More is opened")
        element(app, "moreToggle").tap()
        let hour = element(app, "lifetime:hour")
        XCTAssertTrue(hour.waitForExistence(timeout: 3))
        hour.tap()
        shot(app, "activity-sheet-more")
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
        element(app, "moreToggle").tap()
        let slider = app.sliders["grantSlider"]
        XCTAssertTrue(slider.waitForExistence(timeout: 3))
        slider.adjust(toNormalizedSliderPosition: 0)
        shot(app, "activity-sheet-shorten")
        XCTAssertEqual(element(app, "grantDuration").label, "1 min")
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
        XCTAssertTrue(message.waitForExistence(timeout: 10))
        shot(app, "activity-entry")
        message.tap()
        XCTAssertTrue(element(app, "emailBody").waitForExistence(timeout: 5))
        shot(app, "activity-email")
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
