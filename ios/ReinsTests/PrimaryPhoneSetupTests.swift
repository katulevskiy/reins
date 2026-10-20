import XCTest
@testable import Reins

@MainActor
final class PrimaryPhoneSetupTests: XCTestCase {
    private final class Answer: Authenticating {
        var accepted = true
        var prompts = 0
        var whenConfirm: (() async -> Void)?
        func confirm(_ reason: String) async -> Bool { prompts += 1; await whenConfirm?(); return accepted }
    }
    private func app(_ core: DemoReinsCore, _ auth: Answer) async -> AppModel {
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: auth, demo: true)
        await model.refreshSession()
        return model
    }
    func testBackupAcknowledgmentAndOwnerAreRequiredWithoutGrantingAnyAgentPermission() async throws {
        let core = DemoReinsCore(syncCap: 0.1); let auth = Answer(); let app = await app(core, auth)
        let model = PrimaryPhoneSetupModel(app: app, parentCurrent: { true })
        await model.check(); XCTAssertTrue(model.canCreate)
        await model.enable(backupConfirmed: false, proof: nil) {}
        XCTAssertEqual(auth.prompts, 0); XCTAssertEqual(core.primarySetupCreated, 0)
        await model.enable(backupConfirmed: true, proof: nil) {}
        XCTAssertTrue(model.complete); XCTAssertEqual(core.primarySetupCreated, 1)
        let permissions = try await core.accountPermissions(service: "gmail", account: "me@gmail.com")
        XCTAssertTrue(permissions.capabilities.allSatisfy { $0.configuredMode == .ask })
        XCTAssertEqual(auth.prompts, 1)
    }
    func testCancelledOwnerLeavesLegacyAuthorityUnchanged() async {
        let core = DemoReinsCore(syncCap: 0.1); let auth = Answer(); auth.accepted = false
        let app = await app(core, auth); let model = PrimaryPhoneSetupModel(app: app, parentCurrent: { true })
        await model.check(); await model.enable(backupConfirmed: true, proof: nil) {}
        XCTAssertFalse(model.complete); XCTAssertEqual(core.primarySetupCreated, 0); XCTAssertNil(model.error)
    }
    func testViewerAndRemovedCannotCreateOrTakeOver() async {
        for role in [PhoneRole.viewer, .removed, .primary] {
            let core = DemoReinsCore(syncCap: 0.1); core.configurePrimarySetup(role: role)
            let auth = Answer(); let app = await app(core, auth)
            let model = PrimaryPhoneSetupModel(app: app, parentCurrent: { true })
            await model.check(); XCTAssertFalse(model.canCreate)
            await model.enable(backupConfirmed: true, proof: nil) {}
            XCTAssertEqual(core.primarySetupCreated, 0); XCTAssertEqual(auth.prompts, 0)
        }
    }
    func testOptionalOldAccountProofCanBeRetriedWithoutFallbackAuthority() async {
        let core = DemoReinsCore(syncCap: 0.1); core.configurePrimarySetup(role: .legacy, requiresProof: true)
        let auth = Answer(); let app = await app(core, auth)
        let model = PrimaryPhoneSetupModel(app: app, parentCurrent: { true })
        await model.check(); await model.enable(backupConfirmed: true, proof: nil) {}
        XCTAssertNotNil(model.error); XCTAssertEqual(core.primarySetupCreated, 0)
        await model.enable(backupConfirmed: true, proof: "synthetic-fixture-password") {}
        XCTAssertTrue(model.complete); XCTAssertEqual(core.primarySetupCreated, 1)
    }
    func testWhitespaceProofIsValidatedRatherThanUsingTheSavedSecret() async {
        let core = DemoReinsCore(syncCap: 0.1); let auth = Answer(); let app = await app(core, auth)
        let model = PrimaryPhoneSetupModel(app: app, parentCurrent: { true })
        await model.check(); await model.enable(backupConfirmed: true, proof: "        ") {}
        XCTAssertNotNil(model.error); XCTAssertEqual(core.primarySetupCreated, 0)
        await model.enable(backupConfirmed: true, proof: nil) {}
        XCTAssertTrue(model.complete); XCTAssertEqual(core.primarySetupCreated, 1)
    }
    func testNavigationAndAccountChangesInvalidateTheOriginalOwnerAttempt() async {
        let core = DemoReinsCore(syncCap: 0.1); let auth = Answer(); let app = await app(core, auth)
        let model = PrimaryPhoneSetupModel(app: app, parentCurrent: { true })
        await model.check()
        auth.whenConfirm = { await app.signOut() }
        await model.enable(backupConfirmed: true, proof: nil) {}
        XCTAssertFalse(model.complete); XCTAssertEqual(core.primarySetupCreated, 0)
        let otherCore = DemoReinsCore(syncCap: 0.1); let otherAuth = Answer(); let otherApp = await self.app(otherCore, otherAuth)
        let other = PrimaryPhoneSetupModel(app: otherApp, parentCurrent: { true })
        await other.check(); other.deactivate()
        await other.enable(backupConfirmed: true, proof: nil) {}
        XCTAssertEqual(otherCore.primarySetupCreated, 0); XCTAssertEqual(otherAuth.prompts, 0)
    }
    func testRoleChangingDuringOwnerConfirmationDoesNotCreateNewAuthority() async {
        let core = DemoReinsCore(syncCap: 0.1); let auth = Answer(); let app = await app(core, auth)
        let model = PrimaryPhoneSetupModel(app: app, parentCurrent: { true })
        await model.check(); auth.whenConfirm = { core.configurePrimarySetup(role: .viewer) }
        await model.enable(backupConfirmed: true, proof: nil) {}
        XCTAssertFalse(model.complete); XCTAssertEqual(core.primarySetupCreated, 0)
    }
}
