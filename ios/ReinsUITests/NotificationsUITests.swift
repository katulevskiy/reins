import XCTest

/// Notifications end to end on a simulator: the permission prompt, then the app in the background posting what the
/// core reports (`-notifierSamples`), shown on the Lock Screen / Notification Center. Screenshots go to the test
/// report and to /tmp/reins-ui (simulator file system = this Mac's).
final class NotificationsUITests: XCTestCase {
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    func testPermissionThenBackgroundNotifications() throws {
        let app = XCUIApplication()
        // A reused simulator keeps the last run's notifications and Live Activities: start without them.
        app.launchArguments = ["-demo", "-noPopup", "-resetNotifications", "-askNotifications", "-notifierSamples"]
        app.launch()

        // Asked only once per install: a simulator that already answered shows no prompt.
        let allow = springboard.buttons["Allow"]
        if allow.waitForExistence(timeout: 10) {
            shot("notifications-prompt")
            allow.tap()
        }

        XCUIDevice.shared.press(.home)
        // The samples are posted 3 s after the app leaves the front, and a banner stays only a few seconds; a slow
        // simulator can miss it, but Notification Center keeps it.
        let reins = NSPredicate(format: "label CONTAINS 'Approval needed' OR label CONTAINS 'Connect an AI' OR label CONTAINS 'Permission requested'")
        let banner = springboard.descendants(matching: .any).matching(reins).firstMatch
        let shown = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true"), object: banner)
        if XCTWaiter().wait(for: [shown], timeout: 30) != .completed {
            let top = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.01))
            top.press(forDuration: 0.1, thenDragTo: springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.6)))
            XCTAssertTrue(banner.waitForExistence(timeout: 10), "no Reins notification appeared, nor in Notification Center")
        }
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
