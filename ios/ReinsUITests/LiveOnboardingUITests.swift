import XCTest

/// The phone half of `ios/scripts/live-onboarding.sh`: the real app (real core, no `-demo`) creates an account on a
/// real local server, allows notifications, becomes the approval device, then pairs the "computer" the host plays by
/// the code it shows, opened as a `reins://pair` link (the simulator has no camera to scan its QR code with). Skipped
/// unless the script passes the server's details (TEST_RUNNER_REINS_LIVE_*). Screenshots go to the live folder
/// (/tmp/reins-onboarding).
///
/// The script and this test talk through files in that folder: `created` (written here: the account exists and this
/// phone approves), `usercode` and `confirm` (the code and the number the computer shows), `faceid` (present while an
/// approval waits for Face ID: the script keeps offering a matching face), `host-done` (the computer's session works).
final class LiveOnboardingUITests: XCTestCase {
    private var app: XCUIApplication!
    private var dir: URL!

    override func setUp() {
        continueAfterFailure = false
    }

    private func env(_ key: String) -> String? { ProcessInfo.processInfo.environment[key] }

    private func file(_ name: String) -> URL { dir.appendingPathComponent(name) }

    private func shot(_ name: String) {
        let png = XCUIScreen.main.screenshot().pngRepresentation
        try? png.write(to: file("\(name).png"))
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func element(_ id: String) -> XCUIElement { app.descendants(matching: .any)[id].firstMatch }

    @discardableResult
    private func wait(_ id: String, _ timeout: TimeInterval = 10) -> XCUIElement {
        let e = element(id)
        if !e.waitForExistence(timeout: timeout) {
            shot("failed-waiting-for-\(id)")
            XCTFail("\(id) did not appear within \(Int(timeout)) s")
        }
        return e
    }

    /// Through the setup's pages with their forward button ("Not now" on notifications) until `id` shows.
    private func advance(to id: String, timeout: TimeInterval = 10) {
        for _ in 0..<8 {
            if element(id).waitForExistence(timeout: 1.5) { return }
            if element("onboardingNext").waitForExistence(timeout: timeout) { element("onboardingNext").tap() }
        }
        XCTAssertTrue(element(id).waitForExistence(timeout: timeout), "the setup never showed \(id)")
    }

    private func waitForFile(_ name: String, _ timeout: TimeInterval) -> String? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let text = try? String(contentsOf: file(name), encoding: .utf8) {
                return text.trimmingCharacters(in: .whitespacesAndNewlines)
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return nil
    }

    /// Replaces a text field's contents.
    private func replace(_ field: XCUIElement, with text: String) {
        field.tap()
        if let current = field.value as? String, !current.isEmpty, current != field.placeholderValue {
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count))
        }
        field.typeText(text)
    }

    /// Scrolls until `id` can be tapped.
    private func scrollTo(_ id: String) {
        var tries = 0
        while !(element(id).exists && element(id).isHittable) && tries < 8 {
            app.swipeUp()
            tries += 1
        }
    }

    /// Taps a button of a system alert if one shows (notifications, opening a link).
    private func allowSystemPrompt(_ titles: [String], timeout: TimeInterval) {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for title in titles where springboard.buttons[title].exists {
                springboard.buttons[title].tap()
                return
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
    }

    func testCreateAnAccountAndPairAComputerByItsCode() throws {
        guard let path = env("REINS_LIVE_DIR"), let server = env("REINS_LIVE_SERVER"), let email = env("REINS_LIVE_EMAIL"),
              let password = env("REINS_LIVE_PASSWORD")
        else { throw XCTSkip("run by ios/scripts/live-onboarding.sh") }
        dir = URL(fileURLWithPath: path)
        app = XCUIApplication()
        app.launch()

        // Welcome, then a new account on the local server rather than the hosted one the screen starts with.
        wait("otherServer", 30)
        shot("01-welcome")
        element("otherServer").tap()
        replace(wait("server"), with: server)
        wait("createAccount").tap()
        replace(wait("email"), with: email)
        wait("password").tap()
        element("password").typeText(password)
        wait("passwordAgain").tap()
        element("passwordAgain").typeText(password)
        scrollTo("acceptTerms")
        app.switches["acceptTerms"].tap()
        scrollTo("create")
        shot("02-create-account")
        XCTAssertTrue(wait("create").isEnabled, "the form is complete")
        element("create").tap()

        // The account exists: the setup asks for notifications on its page (allowed), and the phone becomes the
        // approval device by the computer page.
        wait("onboardingNext", 30).tap()
        wait("allowNotifications", 10).tap()
        allowSystemPrompt(["Allow"], timeout: 20)
        advance(to: "phoneReady", timeout: 60)
        XCTAssertFalse(element("registerAgain").exists, "registering as the approval device failed")
        shot("03-connect-your-computer")

        // Claude.ai or ChatGPT: the server's /mcp address, copied; back to the computer step.
        wait("onboardingNext").tap()
        XCTAssertEqual(wait("mcpAddress").label, server.hasSuffix("/") ? "\(server)mcp" : "\(server)/mcp")
        wait("copyMcp").tap()
        shot("04-connect-claude-or-chatgpt")
        wait("onboardingBack").tap()
        wait("scanQR")
        FileManager.default.createFile(atPath: file("created").path, contents: nil)

        // The computer shows its code (in the QR code and under it) and a number; the code arrives as a link.
        // `XCUIApplication.open` relaunches the app with it: the cold-start path, where the code waits until the
        // restored session has registered this phone again. A relaunch leaves the onboarding steps for the app.
        guard let code = waitForFile("usercode", 120), let confirm = waitForFile("confirm", 10), let n = Int(confirm)
        else { return XCTFail("no pairing code from the host") }
        app.open(URL(string: "reins://pair?code=\(code)")!)
        allowSystemPrompt(["Open"], timeout: 3)
        wait("pairingSheet", 30)
        wait("keyFingerprint")
        shot("05-pairing")
        let number = String(format: "%02d", n)
        wait("code:\(number)").tap()
        shot("06-number-picked")

        // Approve with Face ID; the sheet closes and the computer's session works.
        FileManager.default.createFile(atPath: file("faceid").path, contents: nil)
        wait("approve").tap()
        let closed = element("pairingSheet").waitForNonExistence(timeout: 30)
        try? FileManager.default.removeItem(at: file("faceid"))
        if !closed { shot("failed-pairing-stayed") }
        XCTAssertTrue(closed, "the pairing sheet did not close after approving")
        XCTAssertNotNil(waitForFile("host-done", 90), "the computer's session did not work")
        wait("integrations", 30)
        Thread.sleep(forTimeInterval: 2)
        shot("07-activity")

        // The computer is one of the AI connections in Settings.
        let settings = app.tabBars.buttons["Settings"]
        XCTAssertTrue(settings.waitForExistence(timeout: 5))
        settings.tap()
        scrollTo("connectComputer")
        let desktop = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'connection:'")).firstMatch
        XCTAssertTrue(desktop.waitForExistence(timeout: 10), "no AI connection in Settings")
        XCTAssertTrue(desktop.label.contains("Reins desktop app"), "the connection is the computer: \(desktop.label)")
        shot("08-settings-connections")
    }
}
