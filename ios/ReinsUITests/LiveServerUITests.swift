import XCTest

/// The phone half of `ios/scripts/live-smoke.sh`: the real app (real core, no `-demo`) signs in to a real local
/// server, becomes the approval device, connects the AI the host plays, adds the host's MCP server, approves one call
/// once and allows the next for a while, then shows Activity, Grants and Settings. Skipped unless the script passes the
/// server's details (TEST_RUNNER_REINS_LIVE_*). Screenshots go to the live folder (/tmp/reins-live).
///
/// The script and this test talk through files in that folder: `code` (the number the AI's browser shows), `faceid`
/// (present while an approval waits for Face ID: the script keeps offering a matching face), `host-done` (the AI's
/// calls all came back).
final class LiveServerUITests: XCTestCase {
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

    /// Taps the approve button of the open sheet and keeps Face ID matching until the sheet is gone, or shows the next
    /// request (which pops up as soon as this one is answered).
    private func approve(_ sheet: String) {
        FileManager.default.createFile(atPath: file("faceid").path, contents: nil)
        defer { try? FileManager.default.removeItem(at: file("faceid")) }
        let before = element("what").exists ? element("what").label : nil
        wait("approve").tap()
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if !element(sheet).exists { return }
            if let before, element("what").exists, element("what").label != before { return }
            Thread.sleep(forTimeInterval: 0.5)
        }
        shot("failed-\(sheet)-stayed")
        XCTFail("\(sheet) did not close after approving")
    }

    private func section(_ title: String) {
        // Back to the section's root first: a pushed page that scrolled has the tab bar minimized, where a tap on the
        // remaining button only brings the bar back.
        var tries = 0
        while app.navigationBars.buttons["BackButton"].exists && tries < 4 {
            app.navigationBars.buttons["BackButton"].tap()
            Thread.sleep(forTimeInterval: 0.8)
            tries += 1
        }
        let tab = app.tabBars.buttons[title]
        if !(tab.exists && tab.isHittable) { app.swipeDown() }
        XCTAssertTrue(tab.waitForExistence(timeout: 5), "no tab \(title)")
        tab.tap()
    }

    func testSignInPairApproveAndLookAround() throws {
        guard let path = env("REINS_LIVE_DIR"), let server = env("REINS_LIVE_SERVER"), let email = env("REINS_LIVE_EMAIL"),
              let password = env("REINS_LIVE_PASSWORD"), let mcp = env("REINS_LIVE_MCP")
        else { throw XCTSkip("run by ios/scripts/live-smoke.sh") }
        dir = URL(fileURLWithPath: path)
        app = XCUIApplication()
        app.launch()

        // Sign in, to the local server rather than the hosted one the screen starts with.
        wait("signInChoice", 30).tap()
        wait("otherServer").tap()
        replace(wait("server"), with: server)
        replace(wait("email"), with: email)
        wait("password").tap()
        element("password").typeText(password)
        shot("01-sign-in")
        wait("signIn").tap()

        // Signed in: the app asks for notifications (allowed), the onboarding steps show this first time (skipped
        // through: the AI here pairs the old way), then the Activity tab, and no banner saying registration failed.
        wait("onboardingNext", 60)
        let allow = XCUIApplication(bundleIdentifier: "com.apple.springboard").buttons["Allow"]
        if allow.waitForExistence(timeout: 10) { allow.tap() }
        wait("phoneReady", 30)
        wait("onboardingNext").tap()
        wait("onboardingDone").tap()
        wait("integrations", 30)
        Thread.sleep(forTimeInterval: 3)
        XCTAssertFalse(element("registrationBanner").exists, "registering as the approval device failed")
        shot("02-signed-in")

        // The AI connects: pick the code its browser shows, name the connection, confirm.
        wait("pairingSheet", 120)
        guard let code = waitForFile("code", 60), let n = Int(code) else { return XCTFail("no pairing code from the host") }
        let label = String(format: "%02d", n)
        wait("code:\(label)").tap()
        replace(wait("label"), with: "Sim Claude")
        shot("03-pairing")
        approve("pairingSheet")
        shot("04-paired")

        // Add the host's MCP server (Integrations > Add MCP server).
        wait("integrations").tap()
        // Rows further down are made only once they scroll into view.
        XCTAssertTrue(app.staticTexts["Gmail"].waitForExistence(timeout: 10), "Integrations did not open")
        var tries = 0
        while !(element("addMcp").exists && element("addMcp").isHittable) && tries < 8 { app.swipeUp(); tries += 1 }
        wait("addMcp").tap()
        replace(wait("mcpUrl"), with: mcp)
        replace(wait("mcpName"), with: "Notes")
        shot("05-mcp-add")
        wait("mcpAddSubmit").tap()
        XCTAssertTrue(element("mcpUrl").waitForNonExistence(timeout: 30), "the MCP server was not added")
        XCTAssertFalse(element("mcpError").exists)
        // First call: a change (add_note), approved once. Its sheet comes up over whatever is open (the server's page:
        // the AI calls as soon as the tools are offered).
        wait("approvalSheet", 120)
        shot("06-mcp-added")
        XCTAssertEqual(wait("mcpEffect").label, "Changes things")
        shot("07-approve-once")
        approve("approvalSheet")

        // Second call: a read (list_notes), allowed for an hour.
        wait("approvalSheet", 60)
        XCTAssertEqual(wait("mcpEffect").label, "Read only")
        wait("moreToggle").tap()
        // "Remember this for: 1 hour".
        let hour = wait("lifetime:hour")
        tries = 0
        while !hour.isHittable && tries < 6 { app.swipeUp(); tries += 1 }
        hour.tap()
        shot("08-allow-for-a-while")
        approve("approvalSheet")

        // The third call is answered by that permission: no sheet; the host saw every result.
        XCTAssertNotNil(waitForFile("host-done", 90), "the host did not get all its results")
        XCTAssertFalse(element("approvalSheet").exists, "the permission should have answered the third call")
        shot("09-mcp-server")
        section("Activity")
        Thread.sleep(forTimeInterval: 2)
        shot("10-activity")

        section("Grants")
        Thread.sleep(forTimeInterval: 1)
        shot("11-grants")

        section("Settings")
        Thread.sleep(forTimeInterval: 1)
        XCTAssertTrue(app.staticTexts[email].waitForExistence(timeout: 5) || app.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS %@", email)).firstMatch.exists, "Settings does not show the account")
        XCTAssertTrue(wait("registerPhone").label.contains("This phone is used for approvals"), "not the approval device")
        shot("12-settings")
        app.swipeUp()
        shot("13-settings-connections")
    }
}
