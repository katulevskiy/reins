import XCTest
@testable import Reins

/// What Settings, a connection's page and sign-in say and allow.
final class SettingsTextTests: XCTestCase {
    func testTheSoundsRowSaysWhatIsOn() {
        var s = FeedbackSettings()
        XCTAssertEqual(SettingsText.soundsValue(s), "On")
        s.haptics = false
        XCTAssertEqual(SettingsText.soundsValue(s), "Sounds only")
        s.haptics = true
        s.sounds = false
        XCTAssertEqual(SettingsText.soundsValue(s), "Haptics only")
        s.haptics = false
        XCTAssertEqual(SettingsText.soundsValue(s), "Off")
        var off = FeedbackSettings()
        off.master = false
        XCTAssertEqual(SettingsText.soundsValue(off), "Off")
    }

    func testIntegrationsCountTheirAccounts() {
        XCTAssertEqual(SettingsText.accountsValue(0), "None")
        XCTAssertEqual(SettingsText.accountsValue(1), "1 account")
        XCTAssertEqual(SettingsText.accountsValue(3), "3 accounts")
    }

    func testTheModeShowsItsOwnSymbol() {
        // Manual is a person with a check ("you approve"), in blue.
        XCTAssertEqual(SettingsText.modeSymbol(.manual), "person.fill.checkmark")
        XCTAssertEqual(SettingsText.modeValueSymbol(.manual), "person.fill.checkmark")
        XCTAssertEqual(SettingsText.modeValueSymbol(.auto), "bolt")
        XCTAssertEqual(SettingsText.modeValueSymbol(.bypass), "bolt.fill")
        XCTAssertEqual(SettingsText.modeValueSymbol(.lockdown), "lock.fill")
    }

    func testThePasskeysRowShowsTheCount() {
        XCTAssertEqual(SettingsText.passkeysValue(nil), "Not set up")
        XCTAssertEqual(SettingsText.passkeysValue(0), "None")
        XCTAssertEqual(SettingsText.passkeysValue(2), "2")
    }

    func testAModeChangeSoundsLikeWhereItGoes() {
        XCTAssertNil(SettingsText.modeChangeEvent(.auto, .auto))
        XCTAssertEqual(SettingsText.modeChangeEvent(.manual, .bypass), .bypassOn)
        XCTAssertEqual(SettingsText.modeChangeEvent(.manual, .lockdown), .lockdownOn)
        XCTAssertEqual(SettingsText.modeChangeEvent(.bypass, .manual), .bypassOff)
        XCTAssertEqual(SettingsText.modeChangeEvent(.lockdown, .manual), .lockdownOff)
        XCTAssertEqual(SettingsText.modeChangeEvent(.manual, .auto), .autopilotOn)
        XCTAssertEqual(SettingsText.modeChangeEvent(.auto, .manual), .autopilotOff)
        XCTAssertEqual(SettingsText.modeChangeEvent(nil, .auto), .autopilotOn)
    }

    func testABypassCountsItsLastMinuteAsOne() {
        XCTAssertEqual(SettingsText.minutesLeft(1_000 + 42 * 60, now: 1_000), "42 min left")
        XCTAssertEqual(SettingsText.minutesLeft(1_010, now: 1_000), "1 min left")
        XCTAssertEqual(SettingsText.minutesLeft(900, now: 1_000), "1 min left")
    }

    func testARenamedConnectionStillSaysWhatItCallsItself() {
        var c = ConnectionView(id: "c1", label: "Claude", clientHost: "claude.ai", createdAt: 0, lastUsedAt: 9_700, icon: nil, keyFingerprint: nil,
                               ownLabel: "Claude", clientName: "Claude")
        XCTAssertNil(SettingsText.calledItself(c), "its own name: nothing to add")
        var second = c
        second.number = 2
        second.label = "Claude 2"
        XCTAssertEqual(SettingsText.calledItself(second), "Calls itself “Claude”", "a second Claude says what it is")
        c.name = "Claude (work)"
        c.label = "Claude (work)"
        XCTAssertEqual(SettingsText.calledItself(c), "Calls itself “Claude”")
        c.clientName = "Evil\u{202E}\nname"
        XCTAssertFalse(SettingsText.calledItself(c)!.unicodeScalars.contains { $0 == "\u{202E}" || $0 == "\n" })
        XCTAssertEqual(SettingsText.lastUsed(c, now: 10_000), "5 min ago")
        c.lastUsedAt = nil
        XCTAssertEqual(SettingsText.lastUsed(c, now: 10_000), "Never")
    }

    func testAConnectionSaysWhereItRunsAndWhenItWasUsed() {
        let c = ConnectionView(id: "c1", label: "Claude", clientHost: "claude.ai", createdAt: 0, lastUsedAt: 9_700, icon: nil, keyFingerprint: nil)
        XCTAssertEqual(SettingsText.connectionLine(c, now: 10_000), "claude.ai · 5 min ago")
        XCTAssertEqual(SettingsText.connectionHeadline(c, now: 10_000), "claude.ai · used 5 min ago")
        var never = c
        never.lastUsedAt = nil
        XCTAssertEqual(SettingsText.connectionLine(never, now: 10_000), "never used")
        XCTAssertEqual(SettingsText.connectionHeadline(never, now: 10_000), "claude.ai · never used")
        // A computer: its list line says what it is; its page shows the key it paired with.
        var computer = c
        computer.keyFingerprint = "4821 9930"
        computer.lastUsedAt = 10_000
        XCTAssertEqual(SettingsText.connectionLine(computer, now: 10_000), "computer · just now")
        XCTAssertEqual(SettingsText.connectionHeadline(computer, now: 10_000), "Key 4821 9930 · used just now")
    }

    func testTheNotificationChipFollowsTheApprovalPhoneAndPush() {
        // Another phone approves, or notifications are off: no chip (the card says what to do instead).
        XCTAssertEqual(SettingsText.pushChip(approvalDevice: false, notificationsOn: true, appPush: true, serverPush: true), .hidden)
        XCTAssertEqual(SettingsText.pushChip(approvalDevice: true, notificationsOn: false, appPush: true, serverPush: true), .hidden)
        // No push token yet, or a server that sends none: requests arrive only while Reins is open.
        XCTAssertEqual(SettingsText.pushChip(approvalDevice: true, notificationsOn: true, appPush: false, serverPush: true), .onlyWhileOpen)
        XCTAssertEqual(SettingsText.pushChip(approvalDevice: true, notificationsOn: true, appPush: true, serverPush: false), .onlyWhileOpen)
        // A server that does not say is taken to send push.
        XCTAssertEqual(SettingsText.pushChip(approvalDevice: true, notificationsOn: true, appPush: true, serverPush: nil), .on)
        XCTAssertEqual(SettingsText.notificationsAttention(.off), "Notifications are off")
        XCTAssertEqual(SettingsText.notificationsAttention(.notAsked), "Notifications are off")
        XCTAssertEqual(SettingsText.notificationsAttention(.quiet), "Notifications are quiet")
    }

    func testTheCardAndTheConnectSheetShowAddressesWithoutTheScheme() {
        XCTAssertEqual(SettingsText.host("https://app.reins2fa.com/"), "app.reins2fa.com")
        XCTAssertEqual(SettingsText.host("https://app.reins2fa.com/mcp"), "app.reins2fa.com/mcp")
        XCTAssertEqual(SettingsText.host("http://127.0.0.1:8000"), "127.0.0.1:8000")
    }

    func testTheFooterLinksTheLegalPages() {
        let footer = SettingsText.footer(["CFBundleShortVersionString": "1.4"])
        XCTAssertEqual(String(footer.characters), "Reins 1.4  ·  Terms  ·  Privacy")
        let links = footer.runs.compactMap { $0.link }
        XCTAssertEqual(links, [SettingsText.termsURL, SettingsText.privacyURL])
    }

    func testAnIconChoiceHasAName() {
        XCTAssertEqual(SettingsText.iconName(nil), "Auto")
        XCTAssertEqual(SettingsText.iconName(Providers.blob), "Blobatar")
        XCTAssertEqual(SettingsText.iconName("grok"), "Grok")
        XCTAssertEqual(SettingsText.iconName("macos"), "macOS")
        XCTAssertEqual(SettingsText.iconName("windows"), "Windows")
        XCTAssertEqual(SettingsText.iconName("linux"), "Linux")
    }

    func testAHoldTicksFasterAsItFills() {
        XCTAssertEqual(HoldButton.tickGap(0), 0.26, accuracy: 0.0001)
        XCTAssertEqual(HoldButton.tickGap(1), 0.07, accuracy: 0.0001)
        XCTAssertLessThan(HoldButton.tickGap(0.5), HoldButton.tickGap(0.1))
    }

    func testTheVersionShowsItsBuild() {
        XCTAssertEqual(SettingsText.version(["CFBundleShortVersionString": "0.1.0", "CFBundleVersion": "7"]), "0.1.0 (7)")
    }

    func testSigningInNeedsAServerAnEmailAndAPassword() {
        XCTAssertFalse(SignInState.canSubmit(server: "https://", email: "me@example.com", password: "x"))
        XCTAssertFalse(SignInState.canSubmit(server: "https://s.example.com", email: " ", password: "x"))
        XCTAssertFalse(SignInState.canSubmit(server: "https://s.example.com", email: "me@example.com", password: ""))
        XCTAssertTrue(SignInState.canSubmit(server: "https://s.example.com", email: "me@example.com", password: "hunter2"))
    }

    func testDeletingTheAccountNeedsItsEmailTyped() {
        XCTAssertTrue(SettingsText.deletionConfirmed("me@example.com", email: "me@example.com"))
        XCTAssertTrue(SettingsText.deletionConfirmed("  Me@Example.COM \n", email: "me@example.com"))
        XCTAssertFalse(SettingsText.deletionConfirmed("", email: ""))
        XCTAssertFalse(SettingsText.deletionConfirmed("me@example.co", email: "me@example.com"))
        XCTAssertFalse(SettingsText.deletionConfirmed("DELETE", email: "me@example.com"))
    }

    @MainActor
    func testDeletingTheAccountSignsOutAndForgetsTheAccountOnlyOnceTheCoreDeletedIt() async throws {
        DeviceStatus.clear()
        defer { DeviceStatus.clear() }
        let core = DemoReinsCore(signedIn: true, modelInstalled: true, syncCap: 0.3)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshSession()
        guard case let .signedIn(info) = model.session else { return XCTFail("signed in: \(model.session)") }
        DeviceStatus.approvalDevice = true
        do {
            try await model.deleteAccount(confirmEmail: "someone@else.example")
            XCTFail("a wrong email deletes nothing")
        } catch {
            XCTAssertEqual(error.userMessage, "The email you typed is not this account's email.")
        }
        XCTAssertEqual(model.session, .signedIn(info))
        XCTAssertTrue(DeviceStatus.approvalDevice)
        try await model.deleteAccount(confirmEmail: " \(info.email.uppercased()) ")
        XCTAssertEqual(model.session, .signedOut)
        XCTAssertFalse(DeviceStatus.approvalDevice)
        let session = await core.session()
        XCTAssertNil(session)
    }
}
