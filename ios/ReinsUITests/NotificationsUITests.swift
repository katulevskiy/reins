import XCTest

/// Notifications end to end on a simulator: the permission prompt, then the app in the background posting what the
/// core reports (`-notifierSamples`), shown on the Lock Screen / Notification Center. Screenshots go to the test
/// report and to /tmp/reins-ui (simulator file system = this Mac's).
final class NotificationsUITests: XCTestCase {
    private let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")

    func testPermissionThenBackgroundNotifications() throws {
        try XCTSkipIf(ProcessInfo.processInfo.environment["CI"] != nil, "Quarantined on CI: background notifications do not reach Notification Center reliably in the simulator.")
        toHomeScreen()
        let app = XCUIApplication()
        // A reused simulator keeps the last run's notifications and Live Activities: start without them.
        app.launchArguments = ["-demo", "-noPopup", "-resetNotifications", "-askNotifications", "-notifierSamples"]
        firstLaunch = true
        app.launch()
        firstLaunch = false
        allowNotifications()
        // The first launch of a run can race xcodebuild still installing the app: it fails, or the app comes up again
        // without these arguments (on the real sign-in screen). Once, launch it again, and say so in the log.
        if launchFailures > 0 || !app.tab("activity").waitForExistence(timeout: 20) {
            XCTContext.runActivity(named: "Relaunching: the demo did not start") { _ in
                shot("before-relaunch")
                print("NotificationsUITests: relaunching, the demo did not start")
                app.terminate()
                app.launch()
                allowNotifications()
            }
        }

        toHomeScreen()
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

    /// What went wrong while the first launch ran (the install race above): noted, and the launch tried again, instead
    /// of failing the test.
    private var firstLaunch = false
    private var launchFailures = 0

    override func record(_ issue: XCTIssue) {
        if firstLaunch {
            launchFailures += 1
            print("NotificationsUITests: the first launch failed, it is tried again: \(issue.compactDescription)")
            return
        }
        super.record(issue)
    }

    /// Asked only once per install: a simulator that already answered shows no prompt.
    private func allowNotifications() {
        let allow = springboard.buttons["Allow"]
        if allow.waitForExistence(timeout: 10) {
            shot("notifications-prompt")
            allow.tap()
        }
    }

    /// The Home Screen, unlocked and idle. A simulator xcodebuild has just booted can sit on the Lock Screen, where the
    /// banner of an app sent to the background never shows.
    private func toHomeScreen() {
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(springboard.wait(for: .runningForeground, timeout: 15), "SpringBoard is not in front")
        let icons = springboard.icons.firstMatch
        if !icons.waitForExistence(timeout: 5) {
            // The Lock Screen (a simulator has no passcode): swipe up from the bottom edge.
            springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.995))
                .press(forDuration: 0.1, thenDragTo: springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2)))
        }
        let home = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true"), object: icons)
        XCTAssertEqual(XCTWaiter().wait(for: [home], timeout: 20), .completed, "the Home Screen never showed")
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
