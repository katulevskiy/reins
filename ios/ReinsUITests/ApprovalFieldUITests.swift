import XCTest

/// The approval's field and its facts over the `-demo` core: every warning a request carries is on screen, on the
/// field or as the first fact row, never only behind a fold or a (?). Set `REINS_SHOTS=/some/dir` in the test
/// environment to keep a screenshot of each.
final class ApprovalFieldUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func open(_ id: String, extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-noPopup", "-freezeTime", String(Int(Date().timeIntervalSince1970))] + extra
            + ["-open", "reins://item?kind=request&id=\(id)"]
        app.launch()
        // The first launch of a run can race xcodebuild still installing the app: once, launch it again.
        if !element(app, "approvalSheet").waitForExistence(timeout: 20) {
            app.terminate()
            app.launch()
        }
        XCTAssertTrue(element(app, "approvalSheet").waitForExistence(timeout: 20))
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any).matching(identifier: id).firstMatch
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

    /// On screen without scrolling, with its words.
    private func assertShown(_ app: XCUIApplication, _ id: String, says words: String, file: StaticString = #filePath, line: UInt = #line) {
        let e = element(app, id)
        XCTAssertTrue(e.waitForExistence(timeout: 10), "\(id) shows", file: file, line: line)
        XCTAssertTrue(app.windows.firstMatch.frame.contains(e.frame), "\(id) is on screen", file: file, line: line)
        XCTAssertTrue(e.label.contains(words), "\(id) says “\(words)”: \(e.label)", file: file, line: line)
    }

    func testAForcePushToMainSaysEveryWarningAndIsHeld() {
        let app = open("req21")
        XCTAssertTrue(element(app, "what").label.hasPrefix("Force-push 2 commits to"))
        assertShown(app, "forceWarning:0", says: "rewrites history")
        assertShown(app, "defaultBranch", says: "default branch main")
        XCTAssertFalse(element(app, "onceWarning").exists, "the red line already says why it always asks")
        XCTAssertFalse(element(app, "suggestion").exists, "nothing invites a tap on what is held")
        XCTAssertTrue(element(app, "noStanding").exists, "asked every time")
        shot(app, "field-force-push")
    }

    func testUncheckedHistoryIsSaidOnTheField() {
        let app = open("req22")
        assertShown(app, "forceUnknown:1", says: "history not checked")
    }

    func testADestructiveToolSaysSo() {
        let app = open("req32")
        assertShown(app, "mcpDestructive", says: "can't be undone")
        XCTAssertEqual(element(app, "fieldLabel").label.uppercased(), "LINEAR · MCP TOOL")
    }

    func testTheRiskOvernightSawAndItsHoldAreOnTheField() {
        let app = open("req10")
        assertShown(app, "risk", says: "Risk: severe")
        assertShown(app, "riskHeld", says: "Held by Overnight")
        assertShown(app, "novel", says: "First time for this AI")
        shot(app, "field-risk")
    }

    func testTheProgramWarningIsTheFirstFact() {
        let app = open("req43", extra: ["-demoProgramWarning"])
        let warning = element(app, "envProgramWarning")
        XCTAssertTrue(warning.waitForExistence(timeout: 10))
        XCTAssertTrue(warning.label.contains("not where it usually is"))
        XCTAssertLessThan(warning.frame.minY, element(app, "envWho").frame.minY, "before who runs it")
        XCTAssertTrue(app.windows.firstMatch.frame.contains(warning.frame), "on screen without scrolling")
        shot(app, "facts-program-warning")
    }

    func testAPurchaseShowsItsReceiptAndIsHeld() {
        let app = open("req60", extra: ["-demoPay"])
        assertShown(app, "alwaysAsks", says: "money can't come back")
        assertShown(app, "purchaseWarning:0", says: "new for this AI")
        XCTAssertLessThan(element(app, "purchaseWarning:0").frame.minY, element(app, "purchaseMerchant").frame.minY, "the warning comes first")
        XCTAssertTrue(element(app, "purchaseTotal").label.contains("$63.29"))
        XCTAssertTrue(element(app, "purchaseCard").label.contains("4242"))
        XCTAssertTrue(element(app, "noStanding").exists)
        shot(app, "facts-purchase")
    }

    func testADrillInKeepsTheAskInItsBand() {
        let app = open("req20")
        let commits = element(app, "gitCommits:0")
        XCTAssertTrue(commits.waitForExistence(timeout: 10))
        commits.tap()
        let band = element(app, "askBand")
        XCTAssertTrue(band.waitForExistence(timeout: 5))
        XCTAssertTrue(band.label.contains("Push 8 commits to octo/app"))
        shot(app, "band-commits")
    }
}
