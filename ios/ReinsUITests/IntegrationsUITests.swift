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
        app.launchArguments = ["-demo", "-noPopup", "-open", "reins://integrations"]
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
        text("Telegram sent a code to +15550123")
        code.typeText("22222")
        shot("telegram-code")
        waitFor("submitCode").tap()
        let password = waitFor("tgPassword")
        text("(hint: pet)")
        password.typeText("wrong")
        waitFor("submitPassword").tap()
        _ = waitFor("accountError")
        text("That password is wrong.")
        shot("telegram-password-error")
        element("tgPassword").tap()
        element("tgPassword").typeText("hunter2")
        element("submitPassword").tap()
        _ = waitFor("account:+15550123")
        shot("telegram-connected")
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
        _ = waitFor("mcpNotice", timeout: 12)
        text("Signed in to Docs")
        _ = waitFor("tool:search_issues")
        shot("mcp-signed-in")
        element("heavy:create_issue").switches.firstMatch.tap()
        _ = waitFor("badge:create_issue:heavy")
    }
}
