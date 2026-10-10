import XCTest

/// The passkeys that open the vault, driven through the `-demo` app and its passkey sheet (`DemoPasskeys`): offered
/// before the recovery code on a new vault, "Unlock with passkey" on a locked one, and the list in Settings. Screenshots
/// go to /tmp/reins-ui/passkeys-*.png.
final class PasskeyUITests: XCTestCase {
    private var app: XCUIApplication!
    /// The demo account's passkey and the next one the demo sheet makes (base64url, as the identifiers name them).
    private let firstPasskey = "ZGVtby1wYXNza2V5LTE"
    private let madePasskey = "ZGVtby1wYXNza2V5LTI"

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
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: dir.appendingPathComponent("passkeys-\(name).png"))
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

    /// Through the setup's pages with their forward button ("Not now" on notifications) until `id` shows.
    private func advance(to id: String, timeout: TimeInterval = 10) {
        for _ in 0..<8 {
            if element(id).waitForExistence(timeout: 1.5) { return }
            if element("onboardingNext").waitForExistence(timeout: timeout) { element("onboardingNext").tap() }
        }
        XCTAssertTrue(element(id).waitForExistence(timeout: timeout), "the setup never showed \(id)")
    }

    private func gone(_ id: String, _ timeout: TimeInterval = 5) {
        let e = element(id)
        let deadline = Date().addingTimeInterval(timeout)
        while e.exists && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.2)) }
        XCTAssertFalse(e.exists, "\(id) is still there")
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

    private func recordRecovery() {
        wait("recoveryRecorded", 10)
        scrollTo("recoveryRecorded")
        tap("recoveryRecorded")
        scrollTo("recoveryDone")
        tap("recoveryDone")
    }

    /// "Continue" on the welcome screen: a new account (its vault without a passkey with `-demoNoPasskeys`).
    private func continueAsNewAccount(_ extra: [String] = []) {
        launch(["-signedout", "-demoNoPasskeys"] + extra)
        tap("continue")
        wait("passkeyOffer", 10)
    }

    // MARK: A new vault

    func testANewVaultIsOfferedAPasskeyAndTheRecoveryCodeFollows() {
        continueAsNewAccount()
        shot("1-offer")
        tap("addPasskey")
        recordRecovery()
        advance(to: "scanQR")
    }

    func testTheOfferCanBeSkippedForTheRecoveryCodeAlone() {
        continueAsNewAccount()
        tap("skipPasskey")
        gone("passkeyOffer")
        recordRecovery()
        advance(to: "scanQR")
    }

    func testAPasswordManagerWithoutPrfIsNamedAndTheCodeStillWorks() {
        continueAsNewAccount(["-demoPasskeyUnsupported"])
        tap("addPasskey")
        XCTAssertEqual(
            wait("passkeyError").label,
            "This password manager can't unlock Reins. Choose another one, or keep the recovery code."
        )
        shot("2-unsupported")
        wait("passkeyOffer")
        tap("skipPasskey")
        recordRecovery()
        advance(to: "scanQR")
    }

    func testAProviderThatGivesThePrfOnlyOnUseStillAddsOne() {
        continueAsNewAccount(["-demoPasskeyLatePrf"])
        tap("addPasskey")
        recordRecovery()
        advance(to: "scanQR")
    }

    // MARK: A locked vault

    func testALockedAccountUnlocksWithItsPasskeyFirst() {
        launch(["-signedout", "-demoLocked"])
        tap("continue")
        let unlock = wait("unlockWithPasskey", 10)
        XCTAssertLessThan(unlock.frame.minY, wait("askOtherPhone").frame.minY, "the passkey comes first")
        shot("3-unlock")
        unlock.tap()
        recordRecovery()
        advance(to: "scanQR")
    }

    func testNoPasskeyOnThisPhoneSaysWhatElseOpensTheVault() {
        launch(["-signedout", "-demoLocked", "-demoPasskeyNoneHere"])
        tap("continue")
        tap("unlockWithPasskey")
        XCTAssertEqual(
            wait("unlockError").label,
            "No passkey on this phone opens your vault. Ask your other phone, or enter your recovery code."
        )
        wait("askOtherPhone")
        wait("enterRecoveryCode")
    }

    func testALockedAccountWithoutAPasskeyHasNoPasskeyButton() {
        launch(["-signedout", "-demoLocked", "-demoNoPasskeys"])
        tap("continue")
        wait("askOtherPhone", 10)
        XCTAssertFalse(element("unlockWithPasskey").exists)
    }

    // MARK: Settings

    private func openSettings() {
        let tab = app.tabBars.buttons["Settings"]
        if tab.waitForExistence(timeout: 5) {
            tab.tap()
        } else {
            let cell = app.buttons["Settings"].firstMatch
            XCTAssertTrue(cell.waitForExistence(timeout: 5), "no Settings")
            cell.tap()
        }
    }

    func testSettingsListsAddsAndRemovesPasskeys() {
        launch()
        openSettings()
        scrollTo("vaultPasskeysRow")
        XCTAssertTrue(element("vaultPasskeysRow").label.contains("1 passkey"), element("vaultPasskeysRow").label)
        tap("vaultPasskeysRow")
        wait("passkey:\(firstPasskey)", 8)
        shot("4-settings")
        tap("addPasskey")
        wait("passkey:\(madePasskey)", 8)
        tap("removePasskey:\(firstPasskey)")
        let remove = app.sheets.buttons["Remove"]
        if remove.waitForExistence(timeout: 3) {
            remove.tap()
        } else {
            let all = app.buttons.matching(identifier: "Remove")
            XCTAssertGreaterThan(all.count, 0)
            all.element(boundBy: all.count - 1).tap()
        }
        gone("passkey:\(firstPasskey)")
        wait("passkey:\(madePasskey)")
        shot("5-removed")
    }
}
