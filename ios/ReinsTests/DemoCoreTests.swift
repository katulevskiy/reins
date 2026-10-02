import XCTest
@testable import Reins

/// The `-demo` core must behave like the real one where the screens depend on it.
final class DemoCoreTests: XCTestCase {
    private func core(signedIn: Bool = true, installed: Bool = true) -> DemoRewardenCore {
        DemoRewardenCore(signedIn: signedIn, modelInstalled: installed, syncCap: 0.3)
    }

    private func scope() -> GrantScopeChoice { DemoData.scope(senderDomains: ["bank.com"]) }

    func testSeedCoversEveryKind() async throws {
        let c = core()
        let pending = try await c.pending()
        XCTAssertTrue(pending.contains { $0.kind == .pairing })
        XCTAssertTrue(pending.contains { $0.kind == .blob })
        for item in pending where item.kind == .request {
            let view = try await c.approvalView(requestId: item.id)
            XCTAssertEqual(view.requestId, item.id)
            XCTAssertEqual(view.waitUntil, item.waitUntil)
        }
        let kinds = Set(try await pending.asyncMap { $0.kind == .request ? try await c.approvalView(requestId: $0.id).kind : nil }.compactMap { $0 })
        XCTAssertEqual(kinds, [.search, .read, .send, .grant, .accounts, .fetch, .write])
    }

    func testApproveRemovesTheItemAndLogsIt() async throws {
        let c = core()
        let before = try await c.activity(limit: 300)
        try await c.approve(requestId: "req1", choice: ApprovalChoice(selectedMessageIds: ["m1"], standing: nil))
        let pending = try await c.pending()
        XCTAssertFalse(pending.contains { $0.id == "req1" })
        let after = try await c.activity(limit: 300)
        XCTAssertEqual(after.count, before.count + 1)
        XCTAssertEqual(after.first?.outcome, "released")
        XCTAssertEqual(after.first?.count, 1)
        XCTAssertEqual(after.first?.info.messages.map(\.id), ["m1"])
        XCTAssertGreaterThan(after.first?.id ?? 0, before.map(\.id).max() ?? 0)
        do {
            _ = try await c.approvalView(requestId: "req1")
            XCTFail("a decided request is gone")
        } catch CoreError.NotFound {}
    }

    func testApproveWithAStandingChoiceMakesAGrant() async throws {
        let c = core()
        let before = try await c.grants().count
        try await c.approve(requestId: "req1", choice: ApprovalChoice(selectedMessageIds: [], standing: StandingGrant(durationSecs: 3_600, maxUses: nil, scope: scope())))
        let grants = try await c.grants()
        XCTAssertEqual(grants.count, before + 1)
        let made = try XCTUnwrap(grants.first)
        XCTAssertTrue(made.active)
        XCTAssertEqual(made.summary, "Read emails from @bank.com")
        XCTAssertEqual(made.connectionId, "c1")
        let entry = try await c.activity(limit: 1).first
        XCTAssertEqual(entry?.grantId, made.id)
    }

    func testSendAndGrantRequestOutcomes() async throws {
        let c = core()
        try await c.approve(requestId: "req2", choice: ApprovalChoice(selectedMessageIds: [], standing: nil))
        var first = try await c.activity(limit: 1).first
        XCTAssertEqual(first?.outcome, "sent")
        XCTAssertEqual(first?.info.email?.subject, "Q3 report")
        try await c.approve(requestId: "req3", choice: ApprovalChoice(selectedMessageIds: [], standing: nil))
        first = try await c.activity(limit: 1).first
        XCTAssertEqual(first?.outcome, "granted")
        XCTAssertNotNil(first?.grantId)
    }

    func testDenyLogsDenied() async throws {
        let c = core()
        try await c.deny(requestId: "req9")
        let first = try await c.activity(limit: 1).first
        XCTAssertEqual(first?.outcome, "denied")
        let pending = try await c.pending()
        XCTAssertFalse(pending.contains { $0.id == "req9" })
    }

    func testPairingAddsAConnection() async throws {
        let c = core()
        let before = try await c.connections().count
        try await c.answerPairing(pairingId: "pair1", approve: true, chosenCode: 42, label: "Work Gemini")
        let connections = try await c.connections()
        XCTAssertEqual(connections.count, before + 1)
        XCTAssertEqual(connections.last?.label, "Work Gemini")
        let pending = try await c.pending()
        XCTAssertFalse(pending.contains { $0.id == "pair1" })
    }

    func testBypassRunsForTheMinutesAsked() async throws {
        let c = core()
        try await c.setAutopilotMode(connectionId: nil, mode: .bypass, minutes: 30)
        let settings = try await c.autopilotSettings()
        XCTAssertEqual(settings.mode, .bypass)
        XCTAssertEqual(settings.baseMode, .assisted)
        let until = try XCTUnwrap(settings.bypassUntil)
        XCTAssertEqual(Double(until), Date().timeIntervalSince1970 + 1_800, accuracy: 3)
    }

    func testLockdownRefusesWhatIsWaiting() async throws {
        let c = core()
        try await c.setAutopilotMode(connectionId: nil, mode: .lockdown, minutes: nil)
        let pending = try await c.pending()
        XCTAssertFalse(pending.contains { $0.kind == .request })
        XCTAssertTrue(pending.contains { $0.kind == .pairing })
        let first = try await c.activity(limit: 1).first
        XCTAssertEqual(first?.decidedBy, "lockdown")
        XCTAssertEqual(first?.outcome, "denied")
        let settings = try await c.autopilotSettings()
        XCTAssertEqual(settings.mode, .lockdown)
        XCTAssertTrue(settings.connections.allSatisfy { $0.mode == .lockdown })
    }

    func testModelDownloadReportsProgress() async throws {
        let c = core(installed: false)
        let before = await c.modelStatus()
        XCTAssertEqual(before.state, .notInstalled)
        let settings = try await c.autopilotSettings()
        XCTAssertEqual(settings.mode, .manual)
        let progress = Recorder()
        let status = try await c.downloadModel(progress: progress)
        XCTAssertEqual(status.state, .installed)
        XCTAssertGreaterThan(progress.calls.count, 3)
        XCTAssertEqual(progress.calls.last?.0, DemoData.modelSize)
    }

    func testSignInNeedsACodeForTwoFactorAccounts() async throws {
        let c = core(signedIn: false)
        let none = await c.session()
        XCTAssertNil(none)
        do {
            _ = try await c.sync(waitSecs: 25)
            XCTFail("signed out")
        } catch CoreError.NotLoggedIn {}
        do {
            _ = try await c.login(serverUrl: DemoData.server, email: "me+2fa@example.com", password: "pw", totp: nil)
            XCTFail("needs a code")
        } catch CoreError.TwoFactorRequired {}
        let info = try await c.login(serverUrl: DemoData.server, email: "me+2fa@example.com", password: "pw", totp: "123456")
        XCTAssertEqual(info.email, "me+2fa@example.com")
        let session = await c.session()
        XCTAssertEqual(session, info)
    }

    func testMcpAddNeedsSignInOrAdds() async throws {
        let c = core()
        let step = try await c.mcpAdd(url: "https://mcp.auth-example.com/mcp", name: nil)
        guard case let .needsSignIn(id, _) = step else { return XCTFail("needs sign-in") }
        let signedIn = try await c.mcpFinishSignIn(serverId: id, redirectUrl: "reins://mcp?code=x")
        XCTAssertEqual(signedIn.status, "ok")
        let added = try await c.mcpAdd(url: "https://mcp.example.com/mcp", name: "Example")
        guard case let .added(server) = added else { return XCTFail("added") }
        XCTAssertEqual(server.name, "Example")
        XCTAssertFalse(server.tools.isEmpty)
    }

    func testTelegramSignInSteps() async throws {
        let c = core()
        try await c.loginBegin(service: "telegram", phone: "+15550199")
        let step = try await c.loginCode(service: "telegram", code: "22222")
        XCTAssertEqual(step, .needsPassword(hint: "pet"))
        let account = try await c.loginPassword(service: "telegram", password: "hunter2")
        XCTAssertEqual(account.account, "+15550199")
        let telegram = try await c.services().first { $0.service == "telegram" }
        XCTAssertTrue(telegram?.accounts.contains { $0.account == "+15550199" } ?? false)
    }

    func testGrantLifecycle() async throws {
        let c = core()
        try await c.revokeGrant(grantId: "g1")
        var g1 = try await c.grants().first { $0.id == "g1" }
        XCTAssertEqual(g1?.state, "revoked")
        XCTAssertEqual(g1?.active, false)
        try await c.resumeGrant(grantId: "g1", durationSecs: 3_600)
        g1 = try await c.grants().first { $0.id == "g1" }
        XCTAssertEqual(g1?.active, true)
        XCTAssertEqual(g1?.uses, 0)
        try await c.deleteGrant(grantId: "g1")
        let gone = try await c.grants().contains { $0.id == "g1" }
        XCTAssertFalse(gone)
    }

    func testEmailsOpenOrAreGone() async throws {
        let c = core()
        let mail = try await c.fetchEmail(account: "me@gmail.com", messageId: "a1")
        XCTAssertEqual(mail.subject, "Your March statement is ready")
        do {
            _ = try await c.fetchEmail(account: nil, messageId: "nope")
            XCTFail("unknown email")
        } catch CoreError.Gmail {}
    }

    func testArrivalComesDuringSync() async throws {
        let c = DemoRewardenCore(arriveAfter: 0, syncCap: 1)
        let items = try await c.sync(waitSecs: 25)
        XCTAssertEqual(items.first?.id, "req50")
    }
}

private final class Recorder: DownloadProgress, @unchecked Sendable {
    private let lock = NSLock()
    private var list: [(UInt64, UInt64)] = []
    var calls: [(UInt64, UInt64)] { lock.withLock { list } }
    func progress(downloaded: UInt64, total: UInt64) { lock.withLock { list.append((downloaded, total)) } }
}

private extension Array {
    func asyncMap<T>(_ transform: (Element) async throws -> T) async rethrows -> [T] {
        var out: [T] = []
        for e in self { out.append(try await transform(e)) }
        return out
    }
}
