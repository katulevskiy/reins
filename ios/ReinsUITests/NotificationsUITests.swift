import XCTest

/// Notifications end to end on a simulator: the permission prompt, then the app in the background posting what the
/// core reports (`-notifierSamples`), shown on the Lock Screen / Notification Center. Screenshots go to the test
/// report and to /tmp/reins-ui (simulator file system = this Mac's).
final class NotificationsUITests: XCTestCase {
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    func testPermissionThenBackgroundNotifications() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-noPopup", "-askNotifications", "-notifierSamples"]
        app.launch()

        let allow = springboard.buttons["Allow"]
        if allow.waitForExistence(timeout: 10) {
            shot("notifications-prompt")
            allow.tap()
        }

        XCUIDevice.shared.press(.home)
        // The samples are posted 3 s after the app leaves the front; the banner stays a few seconds.
        let banner = springboard.staticTexts.containing(NSPredicate(format: "label CONTAINS 'Approval needed' OR label CONTAINS 'Connect an AI' OR label CONTAINS 'Permission requested'")).firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 15), "no Reins notification appeared")
        shot("notifications-banner")
    }

    private func shot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        let dir = URL(fileURLWithPath: "/tmp/reins-ui", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? screenshot.pngRepresentation.write(to: dir.appendingPathComponent("\(name).png"))
    }
}
