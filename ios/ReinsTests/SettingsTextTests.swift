import XCTest
@testable import Reins

/// What Settings, a connection's page and sign-in say and allow.
final class SettingsTextTests: XCTestCase {
    func testTheSoundsRowSaysWhatIsOn() {
        var s = FeedbackSettings()
        XCTAssertEqual(SettingsText.soundsSummary(s), "Sounds and haptics on")
        s.haptics = false
        XCTAssertEqual(SettingsText.soundsSummary(s), "Sounds on, haptics off")
        s.haptics = true
        s.sounds = false
        XCTAssertEqual(SettingsText.soundsSummary(s), "Haptics on, sounds off")
        s.haptics = false
        XCTAssertEqual(SettingsText.soundsSummary(s), "Off")
        var off = FeedbackSettings()
        off.master = false
        XCTAssertEqual(SettingsText.soundsSummary(off), "Off")
    }

    func testIntegrationsCountTheirAccounts() {
        XCTAssertEqual(SettingsText.accountsLine(0), "Connect Gmail and more")
        XCTAssertEqual(SettingsText.accountsLine(1), "1 account connected")
        XCTAssertEqual(SettingsText.accountsLine(3), "3 accounts connected")
    }

    func testTheAutopilotRowNamesTheModeAndTheModel() async throws {
        XCTAssertEqual(SettingsText.autopilotSummary(nil), "Answers requests for you, on this phone")
        let core = DemoRewardenCore(signedIn: true, modelInstalled: false, syncCap: 0.3)
        var s = try await core.autopilotSettings()
        s.mode = .manual
        XCTAssertEqual(SettingsText.autopilotSummary(s), "Manual · no model yet")
        s.model.state = .installed
        s.mode = .auto
        XCTAssertEqual(SettingsText.autopilotSummary(s), "Auto · model on this phone")
        s.model.state = .downloading
        XCTAssertEqual(SettingsText.autopilotSummary(s), "Auto · downloading the model")
    }

    func testAModeChangeSoundsLikeWhereItGoes() {
        XCTAssertNil(SettingsText.modeChangeEvent(.auto, .auto))
        XCTAssertEqual(SettingsText.modeChangeEvent(.manual, .bypass), .bypassOn)
        XCTAssertEqual(SettingsText.modeChangeEvent(.manual, .lockdown), .lockdownOn)
        XCTAssertEqual(SettingsText.modeChangeEvent(.bypass, .manual), .bypassOff)
        XCTAssertEqual(SettingsText.modeChangeEvent(.lockdown, .manual), .lockdownOff)
        XCTAssertEqual(SettingsText.modeChangeEvent(.manual, .auto), .autopilotOn)
        XCTAssertEqual(SettingsText.modeChangeEvent(.auto, .assisted), .autopilotOff)
        XCTAssertEqual(SettingsText.modeChangeEvent(nil, .assisted), .autopilotOn)
    }

    func testABypassCountsItsLastMinuteAsOne() {
        XCTAssertEqual(SettingsText.minutesLeft(1_000 + 42 * 60, now: 1_000), "42 min left")
        XCTAssertEqual(SettingsText.minutesLeft(1_010, now: 1_000), "1 min left")
        XCTAssertEqual(SettingsText.minutesLeft(900, now: 1_000), "1 min left")
    }

    func testAConnectionSaysWhereItRunsAndWhenItWasUsed() {
        let c = ConnectionView(id: "c1", label: "Claude", clientHost: "claude.ai", createdAt: 0, lastUsedAt: 9_700, icon: nil)
        XCTAssertEqual(SettingsText.connectionLine(c, now: 10_000), "claude.ai · used 5 min ago")
        var never = c
        never.lastUsedAt = nil
        XCTAssertEqual(SettingsText.connectionLine(never, now: 10_000), "claude.ai · never used")
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

    @MainActor
    func testAnIconPickIsFoundByLabelOnlyWhenTheIdIsMissingAndUnambiguous() async {
        let core = DemoRewardenCore(signedIn: true, modelInstalled: true, syncCap: 0.3)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshConnections()
        XCTAssertEqual(model.iconPick(connectionId: "c1", label: "x"), "claude")
        XCTAssertEqual(model.iconPick(connectionId: "", label: "Claude"), "claude")
        XCTAssertNil(model.iconPick(connectionId: "gone", label: "Claude"))
    }
}
