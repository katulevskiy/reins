import XCTest

/// Walks the integration screens in the `-demo` app the way a user would: Telegram's sign-in steps, a rejected
/// token, removing an account, adding an MCP server that needs a sign-in. Screenshots go to the test's attachments
/// and, when `REINS_SHOTS` names a folder, there too.
final class IntegrationsUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
    }

    private func launch(_ integration: String? = nil) {
        app.launchArguments = ["-demo", "-noPopup", "-noLiveActivities", "-open", "reins://integrations"]
            + (integration.map { ["-integration", $0] } ?? [])
        app.launch()
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

    private func waitFor(_ id: String, timeout: TimeInterval = 8) -> XCUIElement {
        let e = element(id)
        XCTAssertTrue(e.waitForExistence(timeout: timeout), "\(id) never showed")
        return e
    }

    private func text(_ s: String, timeout: TimeInterval = 8) {
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", s)).firstMatch.waitForExistence(timeout: timeout), "no text \(s)")
    }

    func testTelegramSignInSteps() {
        launch("telegram")
        let phone = waitFor("phone")
        phone.tap()
        phone.typeText("+15550123")
        waitFor("sendCode").tap()
        let code = waitFor("code")
        code.typeText("22222")
        shot("telegram-code")
        waitFor("submitCode").tap()
        let password = waitFor("tgPassword")
        password.typeText("wrong")
        waitFor("submitPassword").tap()
        _ = waitFor("accountError")
        text("That password is wrong.")
        shot("telegram-password-error")
        element("tgPassword").tap()
        element("tgPassword").typeText("hunter2")
        element("submitPassword").tap()
        _ = waitFor("integrationConnected")
        text("Account connected")
        text("+15550123")
        shot("telegram-connected")
    }

    func testGoogleAccountHasAnExplicitCompletionBeforeAddingAnother() {
        launch("gmail")
        waitFor("addAccount").tap()
        _ = waitFor("integrationConnected")
        text("Account connected")
        text("new.account@gmail.com")
        shot("google-account-connected")
        waitFor("connectionAddAnother").tap()
        XCTAssertFalse(element("integrationConnected").exists)
        _ = waitFor("addAccount")
        _ = waitFor("account:new.account@gmail.com")
    }

    func testGmailPermissionPreviewDisclosesScopeAndCanBeLeftUnsaved() {
        launch("gmail")
        waitFor("addAccount").tap()
        _ = waitFor("integrationConnected")
        reveal(app.buttons["Allow searching and reading"].firstMatch).tap()
        reveal(element("permissionsPreview")).tap()
        _ = waitFor("permissionPreviewSummary")
        shot("gmail-permissions-review")
        _ = reveal(app.staticTexts["Claude Code"].firstMatch)
        _ = reveal(app.staticTexts["No additional agents receive account access."].firstMatch)
        _ = reveal(app.staticTexts["All in this account"].firstMatch)
        _ = reveal(app.staticTexts["At most 20 further uses for each automatic operation."].firstMatch)
        shot("gmail-permissions-review-limits")
        // Discard the preview, then leave setup without saving or requesting owner authentication.
        app.navigationBars.buttons["Back"].firstMatch.tap()
        reveal(element("connectionDone")).tap()
        XCTAssertTrue(element("integrationConnected").waitForNonExistence(timeout: 8), "Later exits account setup")
        // Done/Later returns to Connections. Reopen Gmail to verify the account remains connected;
        // Add another account is the distinct action that deliberately stays on its account list.
        reveal(app.buttons["service:gmail"].firstMatch).tap()
        _ = waitFor("account:new.account@gmail.com")
        XCTAssertFalse(element("permissionsSaved").exists)
    }

    private func reveal(_ control: XCUIElement) -> XCUIElement {
        for _ in 0..<8 {
            if control.exists && control.isHittable { return control }
            app.swipeUp()
        }
        XCTAssertTrue(control.exists && control.isHittable, "Permission control is not visible: \(control)")
        return control
    }

    func testARejectedTokenIsExplained() {
        launch("codeberg")
        waitFor("pasteManually").tap()
        let secret = waitFor("secret")
        secret.tap()
        secret.typeText("bad")
        element("connectSecret").tap()
        _ = waitFor("accountError")
        text("did not accept that token")
        shot("codeberg-rejected")
    }

    func testRemovingAnAccountAsksFirst() {
        launch("gmail")
        waitFor("removeAccount:me@gmail.com").tap()
        let remove = app.buttons["Remove"].firstMatch
        XCTAssertTrue(remove.waitForExistence(timeout: 5))
        shot("gmail-remove-confirm")
        remove.tap()
        let gone = NSPredicate(format: "exists == false")
        expectation(for: gone, evaluatedWith: element("account:me@gmail.com"))
        waitForExpectations(timeout: 8)
        shot("gmail-removed")
    }

    func testAddingAnMcpServerThatNeedsASignIn() {
        launch("mcpAdd")
        let url = waitFor("mcpUrl")
        url.tap()
        url.typeText("https://mcp.auth.example.com/mcp")
        element("mcpName").tap()
        element("mcpName").typeText("Docs")
        shot("mcp-add")
        element("mcpAddSubmit").tap()
        _ = waitFor("integrationConnected", timeout: 12)
        text("Server connected")
        text("Docs")
        shot("mcp-signed-in")
        waitFor("connectionDone").tap()
        _ = waitFor("tool:search_issues")
        // The app's switch (SignalSwitchStyle) is one element, and a tap anywhere on its row toggles it.
        element("heavy:create_issue").tap()
        _ = waitFor("badge:create_issue:heavy")
    }
}
