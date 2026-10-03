import XCTest
@testable import Reins

/// "Continue" (passwordless sign-in): where the account goes next, the Unlock screen (asking the other phone, the
/// recovery code), the lock that survives a relaunch, the approval side of another phone, and the recovery code.
@MainActor
final class PasswordlessTests: XCTestCase {
    override func setUp() {
        super.setUp()
        KeysLock.clear()
        DeviceStatus.clear()
    }

    override func tearDown() {
        KeysLock.clear()
        DeviceStatus.clear()
        super.tearDown()
    }

    private func model(_ core: DemoReinsCore, demo: Bool = true) -> AppModel {
        AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: demo)
    }

    /// "Continue" with the web sheet answered the way the demo core expects.
    private func signInLikeTheRealApp(_ app: AppModel) async {
        let sso = SsoSignIn()
        sso.browse = { _, scheme in URL(string: "\(scheme)://sso-callback?code=x&state=demo-state")! }
        await sso.run(app, feedback: NoFeedback.shared, server: DemoData.server)
    }

    func testContinueWithANewAccountGoesOnLikeASignIn() async {
        let core = DemoReinsCore(signedIn: false, syncCap: 0.3)
        let app = model(core)
        await app.refreshSession()
        XCTAssertEqual(app.session, .signedOut)
        let sso = SsoSignIn()
        await sso.run(app, feedback: NoFeedback.shared, server: DemoData.server)
        XCTAssertNil(sso.error)
        guard case let .signedIn(info) = app.session else { return XCTFail("signed in: \(app.session)") }
        XCTAssertEqual(info.email, DemoData.email)
        XCTAssertTrue(app.approvalDevice, "the phone took the approval role")
        XCTAssertTrue(app.onboarding)
        XCTAssertTrue(app.recoveryCodeAvailable, "Settings offers the recovery code")
    }

    func testLockedKeysWaitOnTheUnlockScreenWithoutTakingTheApprovalRole() async {
        let core = DemoReinsCore(signedIn: false, syncCap: 0.3, keysLocked: true)
        let app = model(core, demo: false)
        await app.refreshSession()
        await signInLikeTheRealApp(app)
        guard case let .keysLocked(info) = app.session else { return XCTFail("locked: \(app.session)") }
        XCTAssertFalse(app.approvalDevice)
        XCTAssertTrue(KeysLock.matches(info))

        // A relaunch comes back to the Unlock screen.
        let relaunched = model(core, demo: false)
        await relaunched.refreshSession()
        XCTAssertEqual(relaunched.session, .keysLocked(info))
        XCTAssertFalse(relaunched.approvalDevice)

        // A wrong code says so; the right one (typed loosely) opens the account and goes on.
        let unlock = UnlockModel()
        unlock.showRecovery()
        unlock.code = "AAAA-BBBB"
        await unlock.unlock(relaunched)
        XCTAssertNotNil(unlock.error)
        XCTAssertEqual(relaunched.session, .keysLocked(info))
        unlock.code = DemoData.recoveryCode.lowercased().replacingOccurrences(of: "-", with: " ")
        await unlock.unlock(relaunched)
        XCTAssertNil(unlock.error)
        XCTAssertEqual(relaunched.session, .signedIn(info))
        XCTAssertTrue(relaunched.approvalDevice)
        XCTAssertFalse(KeysLock.matches(info), "the lock is gone")
    }

    func testAskingTheOtherPhoneShowsTheCodeAndOpensTheAccountWhenItApproves() async {
        let core = DemoReinsCore(signedIn: false, syncCap: 0.3, keysLocked: true)
        let app = model(core)
        await SsoSignIn().run(app, feedback: NoFeedback.shared, server: DemoData.server)
        let unlock = UnlockModel()
        unlock.pollInterval = .seconds(3600) // polled by hand below
        await unlock.ask(app)
        XCTAssertEqual(unlock.stage, .waiting("482 193"))
        let first = await unlock.pollOnce(app)
        XCTAssertFalse(first)
        _ = await unlock.pollOnce(app)
        let done = await unlock.pollOnce(app)
        XCTAssertTrue(done)
        guard case .signedIn = app.session else { return XCTFail("signed in: \(app.session)") }
        unlock.stopWaiting()
    }

    func testSigningOutForgetsTheLock() async {
        let core = DemoReinsCore(signedIn: false, syncCap: 0.3, keysLocked: true)
        let app = model(core, demo: false)
        await signInLikeTheRealApp(app)
        guard case let .keysLocked(info) = app.session else { return XCTFail("locked") }
        await app.signOut()
        XCTAssertEqual(app.session, .signedOut)
        XCTAssertFalse(KeysLock.matches(info))
    }

    func testAClosedSignInPageIsNoError() async {
        let core = DemoReinsCore(signedIn: false, syncCap: 0.3)
        let app = AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: false)
        let sso = SsoSignIn()
        sso.browse = { _, _ in throw WebAuth.Failure.cancelled }
        await sso.run(app, feedback: NoFeedback.shared, server: DemoData.server)
        XCTAssertNil(sso.error)
        XCTAssertFalse(sso.busy)
        sso.browse = { _, _ in URL(string: "com.reins2fa.app://sso-callback?code=x&state=other")! }
        await sso.run(app, feedback: NoFeedback.shared, server: DemoData.server)
        XCTAssertNotNil(sso.error, "an answer to another sign-in is refused")
    }

    // MARK: Taking the approval role from another phone

    func testARefusedTakeoverShowsTheUnlockScreenAndTheRecoveryCodeTakesTheRole() async {
        XCTAssertEqual(
            CoreError.OtherApprovalDevice.userMessage,
            "This account already has a phone for approvals. Approve this phone from it, or enter your recovery code."
        )
        let core = DemoReinsCore(syncCap: 0.3, approvalElsewhere: true)
        let app = model(core, demo: false)
        await app.refreshSession()
        let info = SessionInfo(serverUrl: DemoData.server, email: DemoData.email)
        XCTAssertEqual(app.session, .otherApprovalDevice(info))
        XCTAssertEqual(app.session.unlocking, info)
        XCTAssertFalse(app.approvalDevice)
        XCTAssertFalse(app.deviceReplaced)
        XCTAssertNil(app.registrationError, "the Unlock screen says why, not a banner")

        // Nothing is kept: a relaunch is refused again and shows the same.
        let relaunched = model(core, demo: false)
        await relaunched.refreshSession()
        XCTAssertEqual(relaunched.session, .otherApprovalDevice(info))

        let unlock = UnlockModel()
        unlock.showRecovery()
        unlock.code = "AAAA-BBBB"
        await unlock.unlock(relaunched)
        XCTAssertNotNil(unlock.error)
        XCTAssertEqual(relaunched.session, .otherApprovalDevice(info))
        unlock.code = DemoData.recoveryCode
        await unlock.unlock(relaunched)
        XCTAssertNil(unlock.error)
        XCTAssertEqual(relaunched.session, .signedIn(info))
        XCTAssertTrue(relaunched.approvalDevice)
        XCTAssertTrue(DeviceStatus.approvalDevice)
        XCTAssertNil(relaunched.registrationError)
    }

    func testTheOtherPhonesApprovalLetsThisOneTakeTheRole() async {
        let core = DemoReinsCore(syncCap: 0.3, approvalElsewhere: true)
        let app = model(core)
        await app.refreshSession()
        guard case .otherApprovalDevice = app.session else { return XCTFail("refused: \(app.session)") }
        let unlock = UnlockModel()
        unlock.pollInterval = .seconds(3600) // polled by hand below
        await unlock.ask(app)
        XCTAssertEqual(unlock.stage, .waiting("482 193"))
        _ = await unlock.pollOnce(app)
        _ = await unlock.pollOnce(app)
        let done = await unlock.pollOnce(app)
        XCTAssertTrue(done)
        guard case .signedIn = app.session else { return XCTFail("signed in: \(app.session)") }
        XCTAssertTrue(app.approvalDevice)
        unlock.stopWaiting()
    }

    func testASignInThatIsRefusedTheRoleGoesThroughTheOnboardingStepsOnceItHasIt() async throws {
        let core = DemoReinsCore(signedIn: false, syncCap: 0.3, approvalElsewhere: true)
        let app = model(core, demo: false)
        await app.refreshSession()
        let info = try await core.login(serverUrl: DemoData.server, email: "takeover-\(UUID().uuidString)@example.com", password: "pw", totp: nil)
        await app.finishSignIn(info)
        XCTAssertEqual(app.session, .otherApprovalDevice(info))
        XCTAssertFalse(app.onboarding)
        XCTAssertNil(app.registrationError)

        let unlock = UnlockModel()
        unlock.showRecovery()
        unlock.code = "correct horse battery staple" // the master password works too
        await unlock.unlock(app)
        XCTAssertEqual(app.session, .signedIn(info))
        XCTAssertTrue(app.approvalDevice)
        XCTAssertTrue(app.onboarding, "the first sign-in's steps still show")
    }

    func testAPhoneThatHeldTheRoleShowsAsReplacedUntilItAsksForTheRoleAgain() async {
        DeviceStatus.approvalDevice = true
        let core = DemoReinsCore(syncCap: 0.3, approvalElsewhere: true)
        let app = model(core)
        await app.refreshSession()
        guard case .signedIn = app.session else { return XCTFail("signed in: \(app.session)") }
        XCTAssertTrue(app.deviceReplaced, "a refused refresh means another phone took the role")
        XCTAssertFalse(app.approvalDevice)
        XCTAssertNil(app.registrationError)

        // "Use this phone for approvals" in Settings.
        do {
            try await app.registerDevice(force: true)
            XCTFail("refused")
        } catch CoreError.OtherApprovalDevice {
        } catch {
            XCTFail("\(error)")
        }
        guard case .otherApprovalDevice = app.session else { return XCTFail("the Unlock screen: \(app.session)") }
        let unlock = UnlockModel()
        unlock.showRecovery()
        unlock.code = DemoData.recoveryCode
        await unlock.unlock(app)
        guard case .signedIn = app.session else { return XCTFail("signed in: \(app.session)") }
        XCTAssertTrue(app.approvalDevice)
        XCTAssertFalse(app.deviceReplaced)
    }

    // MARK: The approval side

    func testAnotherPhoneAsksAndTheApprovalDeviceAddsIt() async throws {
        let core = DemoReinsCore(syncCap: 0.3, joinWaiting: true)
        let app = model(core)
        await app.refreshSession()
        let item = try XCTUnwrap(app.pending.first { $0.kind == .join })
        XCTAssertEqual(item.sheetTarget, .join(item.id))
        XCTAssertEqual(item.snapshotKind, .join)
        XCTAssertEqual(NotificationText.title(item), "Add a phone")
        XCTAssertEqual(NotificationText.headline(item), "Add Pixel 9 to your account?")
        XCTAssertEqual(NotificationCategory(item.kind), .join)
        XCTAssertEqual(PushPayload.kind(of: .join), "join")

        let join = JoinModel(joinId: item.id)
        await join.load(app)
        XCTAssertEqual(join.view?.code, "482 193")
        await join.approve(app)
        XCTAssertTrue(join.finished)
        XCTAssertFalse(app.pending.contains { $0.id == item.id })
    }

    func testTheRecoveryCodeShowsInRowsOfThree() {
        let rows = RecoveryCodeSheet.rows(DemoData.recoveryCode)
        XCTAssertEqual(rows.count, 4)
        XCTAssertEqual(rows.first, "TKRQ 7HXM 2PLA")
        XCTAssertEqual(rows.last?.split(separator: " ").count, 4)
        XCTAssertEqual(rows.joined(separator: " ").split(separator: " ").count, 13)
    }
}
