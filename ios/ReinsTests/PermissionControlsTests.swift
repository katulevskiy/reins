import XCTest
@testable import Reins

@MainActor
final class PermissionControlsTests: XCTestCase {
    private final class Answer: Authenticating {
        var answer = true
        var prompts = 0
        func confirm(_ reason: String) async -> Bool { prompts += 1; return answer }
    }
    private final class Owner: OwnerCheck, @unchecked Sendable {
        func confirm(reason: String) async throws -> Bool { true }
    }
    private func app(_ core: DemoReinsCore, _ auth: Answer = Answer()) async -> AppModel {
        let app = AppModel(core: core, feedback: NoFeedback.shared, authenticator: auth, demo: true)
        await app.refreshSession()
        return app
    }
    func testPresetUsesCoreFactoryAndExplicitAdmissionIsInTheReviewedDraft() async throws {
        let core = DemoReinsCore(syncCap: 0.1)
        let auth = Answer()
        let app = await app(core, auth)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com")
        await editor.reload()
        await editor.preset(.readAssist)
        editor.agent("c2", selected: true)
        XCTAssertEqual(editor.draft?.preset, .readAssist)
        XCTAssertTrue(editor.draft?.grantAccountAccess == true)
        XCTAssertEqual(editor.draft?.agentIds, ["c1", "c2"])
        XCTAssertEqual(editor.draft?.rules.first(where: { $0.capability == .mailSend })?.mode, .ask)
        await editor.reviewChanges()
        XCTAssertNil(editor.preview?.eligibleHistoryCount)
        XCTAssertTrue(PermissionCopy.history(try XCTUnwrap(editor.preview)).contains("unavailable"))
        await editor.save()
        XCTAssertTrue(editor.saved)
        XCTAssertEqual(auth.prompts, 1)
    }
    func testCustomizingAPresetKeepsItsCoreRulesAndExplicitAgents() async {
        let core = DemoReinsCore(syncCap: 0.1)
        let app = await app(core)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com")
        await editor.reload(); await editor.preset(.readAssist)
        editor.agent("c2", selected: true)
        let expiry = editor.draft?.expiresAt
        await editor.preset(.custom)
        XCTAssertEqual(editor.draft?.preset, .custom)
        XCTAssertEqual(editor.draft?.expiresAt, expiry)
        XCTAssertEqual(editor.draft?.maxUses, 20)
        XCTAssertEqual(editor.draft?.rules.first(where: { $0.capability == .mailRead })?.mode, .auto)
        XCTAssertTrue(editor.draft?.agentIds.contains("c2") == true)
    }
    func testEmptyAgentsCanSetReviewAndBlockDefaultsWithoutAccountAdmission() async {
        let core = DemoReinsCore(syncCap: 0.1)
        let app = await app(core)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com")
        await editor.reload(); editor.agent("c1", selected: false); editor.mode(.mailSend, .block)
        await editor.reviewChanges()
        XCTAssertNotNil(editor.preview)
        XCTAssertEqual(editor.draft?.agentIds, [])
        XCTAssertFalse(editor.draft?.grantAccountAccess == true)
        XCTAssertFalse(editor.draft?.rules.contains(where: { $0.mode == .auto }) == true)
        await editor.save(); XCTAssertTrue(editor.saved)
    }
    func testCancelledOwnerDoesNotSaveOptionalPermissions() async throws {
        let core = DemoReinsCore(syncCap: 0.1)
        let auth = Answer(); auth.answer = false
        let app = await app(core, auth)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com")
        await editor.reload(); await editor.preset(.readAssist); await editor.reviewChanges(); await editor.save()
        XCTAssertFalse(editor.saved)
        let view = try await core.accountPermissions(service: "gmail", account: "me@gmail.com")
        XCTAssertEqual(view.capabilities.first?.configuredMode, .ask)
        XCTAssertEqual(view.revision, 1)
    }
    func testChangedRevisionRequiresReloadInsteadOfReplacingNewerPolicy() async throws {
        let core = DemoReinsCore(syncCap: 0.1)
        let app = await app(core)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com")
        await editor.reload(); await editor.preset(.readAssist)
        let changed = try await core.accountPermissionDraft(service: "gmail", account: "me@gmail.com", preset: .reviewEveryRequest)
        _ = try await core.saveAccountPermissions(draft: changed, owner: Owner())
        await editor.reviewChanges()
        XCTAssertTrue(editor.changedElsewhere)
        XCTAssertNil(editor.preview)
        XCTAssertEqual(editor.draft?.expectedRevision, 1)
        await editor.reload()
        XCTAssertEqual(editor.draft?.expectedRevision, 2)
        XCTAssertFalse(editor.changedElsewhere)
    }
    func testTypedAddressComesBackFromAccountBoundPicker() async throws {
        let core = DemoReinsCore(syncCap: 0.1)
        let app = await app(core)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com")
        await editor.reload()
        await editor.scopeOptions(.mailSend, query: "FRIEND@example.com")
        let option = try XCTUnwrap(editor.options?.options.first)
        XCTAssertEqual(option.account, "me@gmail.com")
        XCTAssertEqual(option.id, "recipient:friend@example.com")
        editor.mode(.mailSend, .auto)
        editor.scope(.mailSend, PermissionScopeSelection(kind: option.kind, ids: [option.id]))
        XCTAssertEqual(editor.draft?.rules.first(where: { $0.capability == .mailSend })?.scope.ids, [option.id])
    }
    func testSessionChangeStopsOldEditorMutation() async throws {
        let core = DemoReinsCore(syncCap: 0.1)
        let app = await app(core)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com")
        await editor.reload()
        await app.signOut()
        await editor.preset(.readAssist)
        XCTAssertEqual(editor.draft?.preset, .custom)
        XCTAssertFalse(editor.saved)
    }
    func testLeavingEditorStopsLaterChoiceAndOwnerActions() async {
        let core = DemoReinsCore(syncCap: 0.1)
        let app = await app(core)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com")
        await editor.reload()
        editor.deactivate()
        await editor.preset(.readAssist)
        XCTAssertEqual(editor.draft?.preset, .custom)
        XCTAssertFalse(editor.saved)
    }
    func testTemporaryFactoryAndCasLifecycleArePresentedWithoutRecreatingScope() async throws {
        let core = DemoReinsCore(syncCap: 0.1)
        let app = await app(core)
        let editor = AccountPermissionsModel(app: app, service: "gmail", account: "me@gmail.com", temporary: true)
        await editor.reload()
        XCTAssertEqual(editor.draft?.rules.count, 2)
        XCTAssertEqual(editor.draft?.maxUses, 20)
        await editor.reviewChanges(); await editor.save()
        XCTAssertTrue(editor.saved)
        let records = try await core.delegations()
        let record = try XCTUnwrap(records.first)
        let paused = try await core.changeDelegation(id: record.id, expectedRevision: record.revision, change: .pause, owner: Owner())
        XCTAssertEqual(paused.state, .paused)
        let revoked = try await core.changeDelegation(id: paused.id, expectedRevision: paused.revision, change: .revoke, owner: Owner())
        XCTAssertEqual(revoked.state, .revoked)
    }
    func testUnsupportedIntegrationIsReadOnlyAndCannotCreateOneWayBlock() async throws {
        let core = DemoReinsCore(syncCap: 0.1)
        let app = await app(core)
        let editor = AccountPermissionsModel(app: app, service: "github", account: "octo-cat")
        await editor.reload()
        XCTAssertFalse(editor.canEdit)
        XCTAssertNil(editor.draft)
        XCTAssertEqual(editor.view?.availablePresets, [])
        XCTAssertEqual(editor.view?.capabilities.first?.supportedModes, [])
        await editor.preset(.custom)
        await editor.reviewChanges()
        await editor.save()
        XCTAssertNil(editor.draft)
        XCTAssertNil(editor.preview)
        XCTAssertFalse(editor.saved)
        XCTAssertEqual(editor.view?.capabilities.first?.restrictions.first?.controller, .unsupportedAdapter)
    }
}
