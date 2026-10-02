import XCTest

/// The welcome, creating an account, the onboarding steps after it, and connecting a computer by its pairing code
/// (typed, as the simulator has no camera, or opened as a link), driven through the `-demo` app. Screenshots go to
/// /tmp/reins-ui/onboarding-*.png.
final class OnboardingUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ extra: [String] = []) {
        app = XCUIApplication()
        app.launchArguments = ["-demo", "-noPopup"] + extra
        app.launch()
    }

    private func shot(_ name: String) {
        let dir = URL(fileURLWithPath: "/tmp/reins-ui")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: dir.appendingPathComponent("onboarding-\(name).png"))
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any)[id].firstMatch }

    @discardableResult
    private func wait(_ id: String, _ timeout: TimeInterval = 5) -> XCUIElement {
        let e = element(id)
        if !e.waitForExistence(timeout: timeout) {
            shot("failed-waiting-for-\(id)")
            XCTFail("\(id) did not appear")
        }
        return e
    }

    private func tap(_ id: String) { wait(id).tap() }

    private func typeInto(_ id: String, _ text: String) {
        let field = wait(id)
        field.tap()
        field.typeText(text)
    }

    /// Scrolls until `id` can be tapped.
    private func scrollTo(_ id: String) {
        var tries = 0
        while !(element(id).exists && element(id).isHittable) && tries < 8 {
            app.swipeUp()
            tries += 1
        }
        XCTAssertTrue(element(id).exists, "\(id) not found")
    }

    /// Types the code in the connect-computer sheet and picks the number the computer shows; the pairing sheet is up.
    private func pairByTypedCode() {
        wait("pairingCode")
        XCTAssertFalse(element("pairWithCode").isEnabled, "nothing typed yet")
        typeInto("pairingCode", "bcdf ghjk")
        tap("pairWithCode")
        wait("keyFingerprint", 8)
        wait("pairingSheet")
    }

    func testCreatingAnAccountLeadsThroughTheOnboardingStepsIntoTheApp() {
        launch(["-signedout"])
        wait("createAccount", 15)
        wait("signInChoice")
        shot("1-welcome")
        tap("createAccount")

        typeInto("email", "new@example.com")
        typeInto("password", "correct horse battery")
        typeInto("passwordAgain", "correct horse")
        XCTAssertEqual(wait("passwordHint").label, "The two passwords do not match.")
        // A secure field empties when typed into again; the deletes cover one that does not.
        typeInto("passwordAgain", String(repeating: XCUIKeyboardKey.delete.rawValue, count: 13) + "correct horse battery")
        XCTAssertEqual(element("passwordHint").label, "At least 12 characters.")
        XCTAssertFalse(element("create").isEnabled, "the Terms are not accepted yet")
        scrollTo("acceptTerms")
        app.switches["acceptTerms"].tap()
        shot("2-create-account")
        scrollTo("create")
        XCTAssertTrue(element("create").isEnabled)
        tap("create")

        // Step 1: the phone approves, then a computer connects by its code.
        wait("phoneReady", 10)
        shot("3-connect-computer")
        tap("scanQR")
        pairByTypedCode()
        tap("code:47")
        tap("approve")
        XCTAssertTrue(element("pairingSheet").waitForNonExistence(timeout: 8))
        XCTAssertTrue(wait("computerConnected", 5).label.contains("Reins desktop app on studio"), "the computer just paired")
        shot("4-computer-connected")

        // Step 2: the /mcp address for Claude.ai or ChatGPT, copied.
        tap("onboardingNext")
        XCTAssertEqual(wait("mcpAddress").label, "https://rewarden.example.com/mcp")
        tap("copyMcp")
        XCTAssertTrue(app.buttons["Copied"].waitForExistence(timeout: 2))
        shot("5-connect-ai")
        tap("onboardingDone")
        wait("integrations", 8)
    }

    func testSettingsConnectsAComputerByItsCode() {
        launch()
        let tab = app.tabBars.buttons["Settings"]
        XCTAssertTrue(tab.waitForExistence(timeout: 10))
        tab.tap()
        scrollTo("connectComputer")
        tap("connectComputer")
        shot("6-settings-connect-computer")
        pairByTypedCode()
        shot("7-pairing-sheet")
    }

    func testAnExpiredCodeSaysToShowANewOne() {
        launch()
        let tab = app.tabBars.buttons["Settings"]
        XCTAssertTrue(tab.waitForExistence(timeout: 10))
        tab.tap()
        scrollTo("connectComputer")
        tap("connectComputer")
        typeInto("pairingCode", "BBBB-CDFG")
        tap("pairWithCode")
        XCTAssertTrue(app.staticTexts["This code has expired or was already used. Show a new one on your computer."].waitForExistence(timeout: 5))
    }

    func testAPairLinkOpensThePairing() {
        launch(["-open", "reins://pair?code=BCDF-GHJK"])
        wait("keyFingerprint", 10)
        wait("code:47")
    }
}
