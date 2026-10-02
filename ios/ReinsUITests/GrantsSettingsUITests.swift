import XCTest

/// Grants, Settings, a connection's page and sign-in, driven through the `-demo` app (the Android AppFlowTest cases
/// for these screens). `testScreenshots` saves the screens to /tmp/reins-ui/ for a look at the result.
final class GrantsSettingsUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ extra: [String] = []) {
        app = XCUIApplication()
        app.launchArguments = ["-demo", "-noPopup"] + extra
        app.launch()
    }

    private var pad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    /// The simulator's appearance, for the screenshot names: run with TEST_RUNNER_REINS_LOOK=dark after
    /// `xcrun simctl ui <udid> appearance dark`.
    private var look: String { ProcessInfo.processInfo.environment["REINS_LOOK"] ?? "light" }

    /// Saves what is on screen as /tmp/reins-ui/<device>-<appearance>-<name>.png.
    private func shot(_ name: String) {
        let dir = URL(fileURLWithPath: "/tmp/reins-ui")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let file = dir.appendingPathComponent("\(pad ? "ipad" : "iphone")-\(look)-\(name).png")
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: file)
    }

    private func section(_ title: String) {
        let tab = app.tabBars.buttons[title]
        if tab.waitForExistence(timeout: 3) {
            tab.tap()
        } else {
            let cell = app.buttons[title].firstMatch
            XCTAssertTrue(cell.waitForExistence(timeout: 5), "no section \(title)")
            cell.tap()
        }
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any)[id].firstMatch }

    @discardableResult
    private func wait(_ id: String, _ timeout: TimeInterval = 5) -> XCUIElement {
        let e = element(id)
        XCTAssertTrue(e.waitForExistence(timeout: timeout), "\(id) did not appear")
        return e
    }

    private func tap(_ id: String) { wait(id).tap() }

    /// Scrolls down until `id` is on screen (rows of a long list are made only when they come into view). On iPad,
    /// `detail` scrolls the right-hand column, else the section's list in the middle.
    private func scrollTo(_ id: String, detail: Bool = false) {
        let x = pad ? (detail ? 0.78 : 0.4) : 0.5
        var tries = 0
        while !(element(id).exists && element(id).isHittable) && tries < 8 {
            app.coordinate(withNormalizedOffset: CGVector(dx: x, dy: 0.75))
                .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: x, dy: 0.3)))
            tries += 1
        }
        XCTAssertTrue(element(id).exists, "\(id) not found")
    }

    /// The confirming button of a confirmation dialog, not a same-named button behind it.
    private func confirm(_ title: String) {
        let inSheet = app.sheets.buttons[title]
        if inSheet.waitForExistence(timeout: 2) {
            inSheet.tap()
            return
        }
        let all = app.buttons.matching(identifier: title)
        XCTAssertGreaterThan(all.count, 0)
        all.element(boundBy: all.count - 1).tap()
    }

    private func typeInto(_ id: String, _ text: String) {
        let field = wait(id)
        field.tap()
        field.typeText(text)
    }

    // MARK: Screenshots

    func testScreenshots() {
        launch()
        section("Grants")
        wait("grant:g1")
        shot("grants")
        tap("expiredHeader")
        wait("ended:g3")
        shot("grants-expired")
        tap("resume:g3")
        tap("resumeMore")
        wait("customAmount")
        shot("resume-more")
        app.buttons["Cancel"].tap()
        tap("expiredHeader")
        XCTAssertTrue(element("expiredList").waitForNonExistence(timeout: 3))
        tap("grant:g1")
        wait("grantTitle")
        shot("grant-detail")
        app.terminate()

        launch()
        section("Grants")
        tap("newGrant")
        wait("createGrant")
        shot("new-grant")
        section("Settings")
        wait("registerPhone")
        shot("settings")
        app.swipeUp()
        app.swipeUp()
        shot("settings-bottom")
        app.swipeDown()
        app.swipeDown()
        scrollTo("connection:c1")
        tap("connection:c1")
        wait("connectionMode")
        shot("connection")
        app.terminate()

        launch(["-signedout"])
        wait("continue")
        shot("welcome")
        tap("otherServer")
        tap("signInChoice")
        wait("signIn")
        shot("signin")
    }

    // MARK: Grants

    func testEndedGrantsWaitBehindTheExpiredPillWhichStaysOpenWhileBrowsing() {
        launch()
        section("Grants")
        wait("grant:g1")
        XCTAssertFalse(element("ended:g3").exists)
        XCTAssertTrue(app.staticTexts["Expired · 3"].exists)
        tap("expiredHeader")
        wait("expiredList")
        wait("ended:g3")
        wait("ended:g6")
        wait("ended:g7")
        tap("ended:g3")
        wait("grantTitle")
        if !pad {
            app.navigationBars.buttons.firstMatch.tap()
            wait("expiredList")
        }
        tap("expiredHeader")
        XCTAssertTrue(element("expiredList").waitForNonExistence(timeout: 3))
    }

    func testARunningGrantOpensToItsDetailsAndCanBeDeleted() {
        launch()
        section("Grants")
        tap("grant:g1")
        wait("grantTitle")
        XCTAssertTrue(app.staticTexts["Used 12 times"].exists)
        scrollTo("revoke", detail: true)
        tap("revoke")
        confirm("Delete")
        if pad {
            XCTAssertTrue(element("grantTitle").waitForNonExistence(timeout: 5))
        } else {
            wait("newGrant")
        }
        tap("expiredHeader")
        wait("ended:g1")
    }

    func testAnEndedGrantResumesForAChosenPeriod() {
        launch()
        section("Grants")
        tap("expiredHeader")
        tap("resume:g3")
        wait("confirmResume")
        XCTAssertFalse(element("customAmount").exists, "the extras stay tucked away")
        tap("period:week")
        tap("confirmResume")
        wait("grant:g3")
    }

    func testAnInvalidResumeExplainsWhatIsWrong() {
        launch()
        section("Grants")
        tap("expiredHeader")
        tap("resume:g3")
        tap("resumeMore")
        typeInto("customAmount", "0")
        tap("confirmResume")
        wait("resumeInvalid")
        XCTAssertTrue(app.staticTexts["Choose at least a minute."].exists)
    }

    func testAnEndedGrantCanBeDeletedForGood() {
        launch()
        section("Grants")
        tap("expiredHeader")
        tap("delete:g6")
        confirm("Delete")
        XCTAssertTrue(element("ended:g6").waitForNonExistence(timeout: 5))
        wait("ended:g3")
    }

    func testAGrantCanBeCreatedInAdvance() {
        launch()
        section("Grants")
        tap("newGrant")
        scrollTo("createGrant", detail: true)
        tap("createGrant")
        XCTAssertTrue(app.staticTexts["Choose which AI this is for."].waitForExistence(timeout: 3))
        let x = pad ? 0.78 : 0.5
        for _ in 0..<2 {
            app.coordinate(withNormalizedOffset: CGVector(dx: x, dy: 0.3))
                .press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: x, dy: 0.8)))
        }
        tap("conn:c1")
        tap("acct:me@gmail.com")
        typeInto("parties", "alerts@bank.com, @statements.bank.com\n\n")
        scrollTo("newLifetime:oneTime", detail: true)
        tap("newLifetime:oneTime")
        scrollTo("createGrant", detail: true)
        tap("createGrant")
        wait("newGrant")
    }

    // MARK: Settings

    func testSettingsHoldConnectionsIntegrationsAndTheApprovalDevice() {
        launch()
        section("Settings")
        wait("registerPhone")
        scrollTo("connection:c1")
        scrollTo("openIntegrations")
        XCTAssertTrue(app.staticTexts["2 accounts connected"].exists)
    }

    func testAConnectionsModeAndIconCanBePickedAndItCanBeDisconnected() {
        launch()
        section("Settings")
        scrollTo("connection:c1")
        tap("connection:c1")
        wait("connectionMode")
        tap("connMode:follow")
        let line = wait("connectionModeLine")
        let predicate = NSPredicate(format: "label BEGINSWITH 'Like every AI'")
        expectation(for: predicate, evaluatedWith: line)
        waitForExpectations(timeout: 5)
        scrollTo("icon:grok", detail: true)
        tap("icon:grok")
        scrollTo("disconnect", detail: true)
        tap("disconnect")
        confirm("Disconnect")
        if pad {
            wait("connectionGone")
        } else {
            wait("connection:c2")
            XCTAssertFalse(element("connection:c1").exists)
        }
    }

    func testSigningOutReturnsToSignIn() {
        launch()
        section("Settings")
        scrollTo("signOut")
        tap("signOut")
        confirm("Sign out")
        wait("continue")
    }

    // MARK: Sign-in

    func testSignInAsksForATwoStepCodeWhenTheServerDoes() {
        launch(["-signedout"])
        tap("otherServer")
        tap("signInChoice")
        XCTAssertFalse(wait("signIn").isEnabled)
        typeInto("email", "me2fa@example.com")
        typeInto("password", "hunter2")
        tap("signIn")
        wait("totp")
        typeInto("totp", "123456")
        tap("signIn")
        XCTAssertTrue(element("signIn").waitForNonExistence(timeout: 8))
    }

    func testTheServerFieldAndPasswordsWaitBehindUseAnotherServer() {
        launch(["-signedout"])
        wait("continue")
        XCTAssertFalse(element("server").exists)
        XCTAssertFalse(element("signInChoice").exists, "no password sign-in on the hosted server")
        XCTAssertFalse(element("createAccount").exists)
        tap("otherServer")
        wait("server")
        wait("signInChoice")
        XCTAssertFalse(element("otherServer").exists)
    }

    func testWrongCredentialsShowAnActionableError() {
        launch(["-signedout"])
        tap("otherServer")
        tap("signInChoice")
        typeInto("email", "me@example.com")
        typeInto("password", "wrong")
        tap("signIn")
        XCTAssertTrue(app.staticTexts["Wrong email, password or code."].waitForExistence(timeout: 5))
    }
}
