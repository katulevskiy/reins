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
        app.launchArguments = ["-demo", "-noPopup", "-noLiveActivities"] + extra
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

    /// Opens one of the four places ("trust", "connections", "settings").
    private func section(_ name: String) {
        XCTAssertTrue(app.openTab(name), "no section \(name)")
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
        section("trust")
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
        if !pad {
            app.navigationBars.buttons.firstMatch.tap()
            XCTAssertTrue(element("expiredList").waitForNonExistence(timeout: 3))
        }
        tap("grant:g1")
        wait("grantTitle")
        shot("grant-detail")
        app.terminate()

        launch()
        section("trust")
        tap("newGrant")
        wait("createGrant")
        shot("new-grant")
        section("settings")
        wait("accountCard")
        shot("settings")
        app.swipeUp()
        app.swipeUp()
        shot("settings-bottom")
        app.swipeDown()
        app.swipeDown()
        section("connections")
        shot("connections")
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

    func testEndedGrantsFoldIntoOneRowThatOpensTheirList() {
        launch()
        section("trust")
        wait("grant:g1")
        XCTAssertFalse(element("ended:g3").exists)
        XCTAssertTrue(element("expiredHeader").exists)
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
            app.navigationBars.buttons.firstMatch.tap()
            XCTAssertTrue(element("expiredList").waitForNonExistence(timeout: 3))
        }
    }

    func testARunningGrantOpensToItsDetailsAndCanBeDeleted() {
        launch()
        section("trust")
        tap("grant:g1")
        wait("grantTitle")
        XCTAssertTrue(app.staticTexts["Used 12 times"].exists)
        // Ending a grant is held, not tapped: a tap does nothing.
        tap("revoke")
        XCTAssertTrue(element("grantTitle").exists, "a tap ends nothing")
        element("revoke").press(forDuration: 2.0)
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
        section("trust")
        tap("expiredHeader")
        tap("resume:g3")
        wait("confirmResume")
        XCTAssertFalse(element("customAmount").exists, "the extras stay tucked away")
        tap("period:custom")
        tap("period:week")
        tap("confirmResume")
        XCTAssertTrue(element("ended:g3").waitForNonExistence(timeout: 5))
        if !pad { app.navigationBars.buttons.firstMatch.tap() }
        wait("grant:g3")
    }

    func testAnInvalidResumeExplainsWhatIsWrong() {
        launch()
        section("trust")
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
        section("trust")
        tap("expiredHeader")
        tap("delete:g6")
        confirm("Delete")
        XCTAssertTrue(element("ended:g6").waitForNonExistence(timeout: 5))
        wait("ended:g3")
    }

    func testAGrantCanBeCreatedInAdvance() {
        launch()
        section("trust")
        tap("newGrant")
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
        tap("createGrant")
        wait("newGrant")
    }

    // MARK: Settings

    func testConnectionsHoldTheAIsAndTheServicesAndSettingsTheApprovalDevice() {
        launch()
        section("settings")
        wait("approvalChip")
        XCTAssertFalse(element("connection:c1").exists, "connections moved out of Settings")
        section("connections")
        scrollTo("connection:c1")
        scrollTo("service:gmail")
        XCTAssertTrue(element("service:gmail").label.contains("2 accounts"), element("service:gmail").label)
    }

    /// The rename sheet's field, Save and "Use its own name" are reachable (the sheet is a container, its id does not
    /// replace theirs), and a name typed there becomes the connection's.
    func testAConnectionCanBeRenamed() {
        launch()
        section("connections")
        scrollTo("connection:c1")
        tap("connection:c1")
        tap("connectionRename")
        wait("renameSheet")
        let field = wait("renameField")
        XCTAssertTrue(wait("renameSave").exists, "Save is reachable inside the sheet")
        field.tap()
        field.typeText("Work Claude")
        shot("rename-sheet")
        tap("renameSave")
        XCTAssertTrue(element("renameSheet").waitForNonExistence(timeout: 5), "Save closes the sheet")
        let named = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", "Work Claude"), object: element("connectionName"))
        XCTAssertEqual(XCTWaiter().wait(for: [named], timeout: 5), .completed, "the new name shows: \(element("connectionName").label)")
    }

    func testAConnectionsModeAndIconCanBePickedAndItCanBeDisconnected() {
        launch()
        section("connections")
        scrollTo("connection:c1")
        tap("connection:c1")
        wait("connectionMode")
        tap("connMode:AUTO")
        expectation(for: NSPredicate(format: "isSelected == true"), evaluatedWith: element("connMode:AUTO"))
        waitForExpectations(timeout: 5)
        tap("connMode:follow")
        expectation(for: NSPredicate(format: "isSelected == true"), evaluatedWith: element("connMode:follow"))
        waitForExpectations(timeout: 5)
        scrollTo("connectionIcon", detail: true)
        tap("connectionIcon")
        tap("icon:grok")
        // Disconnect is held; no dialog.
        scrollTo("disconnect", detail: true)
        wait("disconnect").press(forDuration: 2.6)
        if pad {
            wait("connectionGone")
        } else {
            wait("connection:c2")
            XCTAssertFalse(element("connection:c1").exists)
        }
    }

    /// Claude shows its logo; changing its blobatar's colour and shuffling keep the sheet open and make it show the
    /// blobatar.
    func testAConnectionsBlobatarCanBeChangedAndShuffled() {
        launch()
        section("connections")
        scrollTo("connection:c1")
        tap("connection:c1")
        scrollTo("connectionIcon", detail: true)
        tap("connectionIcon")
        wait("iconSheet")
        var tries = 0
        while !(element("shuffleBlobatar").exists && element("shuffleBlobatar").isHittable) && tries < 4 {
            element("iconSheet").swipeUp()
            tries += 1
        }
        tap("colour:3")
        expectation(for: NSPredicate(format: "isSelected == true"), evaluatedWith: element("colour:3"))
        expectation(for: NSPredicate(format: "isSelected == true"), evaluatedWith: element("icon:blob"))
        waitForExpectations(timeout: 5)
        tap("shape:2")
        expectation(for: NSPredicate(format: "isSelected == true"), evaluatedWith: element("shape:2"))
        waitForExpectations(timeout: 5)
        tap("shuffleBlobatar")
        XCTAssertTrue(element("iconSheet").exists, "changing the blobatar keeps the sheet open")
        expectation(for: NSPredicate(format: "isSelected == false"), evaluatedWith: element("shape:2"))
        waitForExpectations(timeout: 5)
    }

    func testSigningOutReturnsToSignIn() {
        launch()
        section("settings")
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
