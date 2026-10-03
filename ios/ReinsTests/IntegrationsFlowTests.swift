import XCTest
@testable import Reins

/// The integration screens' models against the `-demo` core (Android's `McpFlowTest`, `GitHostsFlowTest` and the
/// service flows): what reaches the core, what the user sees, which sounds play.
@MainActor
final class IntegrationsFlowTests: XCTestCase {
    private var feedback: RecordingFeedback!
    private var platform: FakePlatform!
    private var model: AppModel!

    override func setUp() async throws {
        feedback = RecordingFeedback()
        platform = FakePlatform()
        model = AppModel(core: DemoReinsCore(syncCap: 0.3), feedback: feedback, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshSession()
    }

    private func accounts(_ service: String) -> AccountsModel { AccountsModel(service: service, model: model, platform: platform) }

    // MARK: Services

    func testTheStatusOfEveryAccountIsChecked() async {
        let gmail = accounts("gmail")
        await gmail.refresh()
        XCTAssertEqual(gmail.accounts.map(\.account), ["me@gmail.com", "work@corp.example"])
        XCTAssertEqual(gmail.statuses["me@gmail.com"], .ready)
        XCTAssertEqual(gmail.statuses["work@corp.example"], .needsConsent)
    }

    func testWithoutAGoogleClientGoogleNeedsSetupAndNothingIsAsked() async {
        platform.googleConfigured = false
        let calendar = accounts("gcontacts")
        await calendar.addGoogle()
        XCTAssertEqual(calendar.error, platform.googleSetupMessage)
        XCTAssertTrue(platform.googleAsked.isEmpty)
        XCTAssertEqual(feedback.cues.last, .error)
        XCTAssertTrue(calendar.accounts.isEmpty)
    }

    func testAGoogleAccountIsAddedWithTheAddressThatSignedIn() async {
        platform.googleAnswer = "ada@gmail.com"
        let contacts = accounts("gcontacts")
        await contacts.addGoogle()
        XCTAssertNil(contacts.error)
        XCTAssertEqual(platform.googleAsked.map(\.service), ["gcontacts"])
        XCTAssertEqual(contacts.accounts.map(\.account), ["ada@gmail.com"])
        XCTAssertEqual(contacts.statuses["ada@gmail.com"], .ready)
        XCTAssertEqual(feedback.cues.last, .reconnected)
    }

    func testAllowingAGmailAccountAgainAsksForThatAccount() async {
        let gmail = accounts("gmail")
        await gmail.refresh()
        platform.googleAnswer = nil
        await gmail.addGoogle(hint: "work@corp.example")
        XCTAssertEqual(platform.googleAsked.last?.hint, "work@corp.example")
        XCTAssertEqual(gmail.statuses["work@corp.example"], .ready)
        XCTAssertEqual(model.accounts.filter { $0.service == "gmail" }.count, 2)
    }

    func testClosingGooglesPageShowsNothing() async {
        platform.googleCancels = true
        let calendar = accounts("gcontacts")
        await calendar.addGoogle()
        XCTAssertNil(calendar.error)
        XCTAssertTrue(calendar.accounts.isEmpty)
        XCTAssertFalse(feedback.cues.contains(.error))
    }

    func testRemovingAGoogleAccountRevokesItsAccess() async {
        let gmail = accounts("gmail")
        await gmail.refresh()
        await gmail.remove("me@gmail.com")
        XCTAssertEqual(gmail.accounts.map(\.account), ["work@corp.example"])
        XCTAssertNil(gmail.statuses["me@gmail.com"])
        XCTAssertEqual(platform.revoked.map(\.account), ["me@gmail.com"])
        XCTAssertTrue(feedback.cues.contains(.delete))
        XCTAssertFalse(model.grants.contains { $0.service == "gmail" && $0.account == "me@gmail.com" })
    }

    func testRemovingATokenAccountDoesNotTalkToGoogle() async {
        let github = accounts("github")
        await github.remove("octo-cat")
        XCTAssertTrue(github.accounts.isEmpty)
        XCTAssertTrue(platform.revoked.isEmpty)
    }

    func testARejectedTokenIsExplainedAndAGoodOneConnects() async {
        let codeberg = accounts("codeberg")
        await codeberg.addSecret("bad")
        XCTAssertEqual(codeberg.error, "Codeberg did not accept that token. Check that it is complete and has the scopes listed.")
        XCTAssertEqual(feedback.cues.last, .error)
        await codeberg.addSecret("  0123456789abcdef0123456789abcdef01234567\n")
        XCTAssertNil(codeberg.error)
        XCTAssertEqual(codeberg.accounts.count, 1)
        XCTAssertEqual(feedback.cues.last, .reconnected)
    }

    func testThePhonesCalendarIsConnectedOnceIOSAllowsIt() async {
        let removed = accounts("device_calendar")
        await removed.remove("this phone")
        XCTAssertTrue(removed.accounts.isEmpty)

        platform.phoneAllows = false
        await removed.addDevice()
        XCTAssertEqual(removed.error, ServiceCopy.permissionRefused)
        XCTAssertTrue(removed.refusedByIOS)
        XCTAssertTrue(removed.accounts.isEmpty)

        platform.phoneAllows = true
        await removed.addDevice()
        XCTAssertNil(removed.error)
        XCTAssertFalse(removed.refusedByIOS)
        XCTAssertEqual(removed.accounts.map(\.account), ["this phone"])
        XCTAssertEqual(platform.phoneAsked, ["device_calendar", "device_calendar"])
    }

    func testTextMessagesAreShownAsUnavailable() {
        let sms = accounts("sms")
        XCTAssertEqual(sms.view?.available, false)
        XCTAssertFalse(model.services.contains { $0.service == "sms" })
    }

    // MARK: Telegram

    func testTelegramSignsInWithAPhoneNumberAndACode() async throws {
        try await model.core.removeServiceAccount(service: "telegram", account: "+15550100")
        let telegram = accounts("telegram")
        await telegram.loginBegin("  +44 7700 900123 ")
        XCTAssertEqual(telegram.login, .code(phone: "+44 7700 900123"))
        await telegram.loginCode("00000")
        XCTAssertEqual(telegram.error, "That code is wrong or has expired. Ask for a new one.")
        XCTAssertEqual(telegram.login, .code(phone: "+44 7700 900123"))
        await telegram.loginCode("12345")
        XCTAssertNil(telegram.error)
        XCTAssertEqual(telegram.login, .phone)
        XCTAssertEqual(telegram.accounts.map(\.account), ["+44 7700 900123"])
        XCTAssertEqual(feedback.cues.last, .reconnected)
    }

    func testTwoStepVerificationAsksForThePassword() async {
        let telegram = accounts("telegram")
        await telegram.loginBegin("+15550199")
        await telegram.loginCode("22222")
        XCTAssertEqual(telegram.login, .password(hint: "pet"))
        await telegram.loginPassword("wrong")
        XCTAssertEqual(telegram.error, "That password is wrong.")
        XCTAssertEqual(telegram.login, .password(hint: "pet"))
        await telegram.loginPassword("hunter2")
        XCTAssertEqual(telegram.login, .phone)
        XCTAssertTrue(telegram.accounts.contains { $0.account == "+15550199" })
    }

    func testAnotherNumberStartsOver() async {
        let telegram = accounts("telegram")
        await telegram.loginBegin("+15550199")
        await telegram.loginCode("00000")
        telegram.loginRestart()
        XCTAssertEqual(telegram.login, .phone)
        XCTAssertNil(telegram.error)
    }

    // MARK: MCP

    private func mcp() -> McpModel { McpModel(model: model, platform: platform) }

    func testAServerThatNeedsNoSignInIsAddedWithItsTools() async {
        let m = mcp()
        let id = await m.add(url: " https://mcp.example.com/mcp ", name: " Example ", token: "")
        XCTAssertEqual(id, "example")
        let server = model.mcpServers.first { $0.id == "example" }
        XCTAssertEqual(server?.name, "Example")
        XCTAssertEqual(server?.status, "ok")
        XCTAssertEqual(server?.tools.count, 4)
        XCTAssertTrue(platform.signIns.isEmpty)
        XCTAssertEqual(feedback.cues.last, .reconnected)
    }

    func testAnEmptyNameIsNotSent() async {
        let id = await mcp().add(url: "https://mcp.acme.dev/mcp", name: "   ", token: "")
        XCTAssertEqual(model.mcpServers.first { $0.id == id }?.name, "Acme")
    }

    func testATokenAddsTheServerWithItAndNoSignIn() async {
        // An address the demo core would otherwise ask a sign-in for.
        let id = await mcp().add(url: "https://mcp.auth-example.com/mcp", name: "", token: " tok_123 ")
        XCTAssertNotNil(id)
        XCTAssertEqual(model.mcpServers.first { $0.id == id }?.status, "ok")
        XCTAssertTrue(platform.signIns.isEmpty)
    }

    func testAFailureIsShownInPlainWords() async {
        let m = mcp()
        let id = await m.add(url: "https://fail.example.com/mcp", name: "", token: "")
        XCTAssertNil(id)
        XCTAssertEqual(m.error, "No connection: the server did not answer")
        XCTAssertEqual(feedback.cues.last, .error)
        XCTAssertFalse(m.busy)
    }

    func testAServerThatNeedsSignInOpensItsPageAndTheSignInFinishesIt() async {
        let m = mcp()
        let id = await m.add(url: "https://mcp.auth.example.com/mcp", name: "Docs", token: "")
        XCTAssertEqual(platform.signIns.map(\.serverId), ["auth"])
        XCTAssertEqual(platform.signIns.first?.url, "https://auth.example.com/authorize?client_id=reins&server=auth")
        XCTAssertEqual(id, "auth")
        XCTAssertNil(m.signingIn)
        XCTAssertEqual(model.mcpServers.first { $0.id == "auth" }?.status, "ok")
        XCTAssertEqual(model.mcpNotice, McpNotice(serverId: "auth", text: "Signed in to Docs. Its tools can be used now.", failed: false))
        XCTAssertEqual(feedback.cues.last, .reconnected)
    }

    func testAFailedSignInIsExplainedOnTheServersPage() async {
        platform.signInFails = CoreError.Invalid(reason: "The sign-in was cancelled")
        let id = await mcp().add(url: "https://mcp.notion.com/mcp", name: "", token: "")
        XCTAssertEqual(id, "notion")
        XCTAssertEqual(model.mcpNotice, McpNotice(serverId: "notion", text: "Signing in did not work: The sign-in was cancelled", failed: true))
        XCTAssertEqual(feedback.cues.last, .error)
    }

    func testClosingTheSignInPageStaysOnTheAddPage() async {
        platform.signInCancels = true
        let m = mcp()
        let id = await m.add(url: "https://mcp.notion.com/mcp", name: "", token: "")
        XCTAssertNil(id)
        XCTAssertNil(m.error)
        XCTAssertNil(model.mcpNotice)
    }

    func testAServerWhoseSignInEndedCanSignInAgainFromItsPage() async {
        let m = mcp()
        await m.refresh("notion")
        XCTAssertEqual(platform.signIns.map(\.serverId), ["notion"])
        XCTAssertEqual(platform.signIns.first?.url, "https://auth.example.com/authorize?client_id=reins&server=notion")
        XCTAssertEqual(feedback.cues.first, .refresh)
        XCTAssertEqual(model.mcpServers.first { $0.id == "notion" }?.status, "ok")
        XCTAssertEqual(model.mcpNotice?.serverId, "notion")
    }

    func testTheHeavyToggleTellsTheCore() async {
        let m = mcp()
        await m.setHeavy("linear", tool: "create_issue", heavy: true)
        XCTAssertEqual(model.mcpServers.first { $0.id == "linear" }?.tools.first { $0.name == "create_issue" }?.heavy, true)
        XCTAssertEqual(model.mcpServers.first { $0.id == "linear" }.map { ToolBadge.of($0.tools[1]) }, [.changes, .heavy])
        await m.setHeavy("linear", tool: "export_project", heavy: false)
        XCTAssertEqual(model.mcpServers.first { $0.id == "linear" }?.tools.first { $0.name == "export_project" }?.heavy, false)
    }

    func testRefreshAsksTheServerAgain() async {
        let m = mcp()
        await m.refresh("sentry")
        XCTAssertEqual(model.mcpServers.first { $0.id == "sentry" }?.status, "ok")
        XCTAssertNil(model.mcpServers.first { $0.id == "sentry" }?.error)
        XCTAssertTrue(platform.signIns.isEmpty)
    }

    func testRemovingAServerForgetsItAndItsGrants() async {
        let removed = await mcp().remove("linear")
        XCTAssertTrue(removed)
        XCTAssertFalse(model.mcpServers.contains { $0.id == "linear" })
        XCTAssertFalse(model.grants.contains { $0.service == "mcp:linear" })
        XCTAssertTrue(feedback.cues.contains(.delete))
    }

    func testTheIntegrationsLinkOpensThePageOverActivity() async {
        model.section = .grants
        await model.handle(.integrations)
        XCTAssertEqual(model.section, .activity)
        XCTAssertEqual(model.path(.activity), [.integrations])
    }
}

// MARK: Fakes

@MainActor
private final class FakePlatform: IntegrationsPlatform {
    var googleConfigured = true
    let googleSetupMessage = "Google access needs setup."
    /// The address Google answers with; nil: the one asked for.
    var googleAnswer: String? = "me@gmail.com"
    var googleCancels = false
    var phoneAllows = true
    var signInCancels = false
    var signInFails: Error?

    var googleAsked: [(service: String, hint: String?)] = []
    var revoked: [(account: String, service: String)] = []
    var phoneAsked: [String] = []
    var signIns: [(serverId: String, url: String)] = []

    func authorizeGoogle(service: String, loginHint: String?) async throws -> String? {
        googleAsked.append((service, loginHint))
        if googleCancels { return nil }
        return googleAnswer ?? loginHint
    }

    func revokeGoogle(account: String, service: String) async { revoked.append((account, service)) }

    func requestPhoneAccess(service: String) async -> Bool {
        phoneAsked.append(service)
        return phoneAllows
    }

    func mcpSignIn(serverId: String, authorizeUrl: String, model: AppModel) async -> McpSignInOutcome {
        signIns.append((serverId, authorizeUrl))
        if signInCancels { return .cancelled }
        if let signInFails { return await model.finishMcpSignIn(.failure(signInFails), serverId: serverId) }
        do {
            let server = try await model.core.mcpFinishSignIn(serverId: serverId, redirectUrl: "com.reins2fa.app://mcp-oauth?code=c&state=s")
            return await model.finishMcpSignIn(.success(server), serverId: serverId)
        } catch {
            return await model.finishMcpSignIn(.failure(error), serverId: serverId)
        }
    }
}

private final class RecordingFeedback: Feedback {
    var cues: [Cue] = []
    func haptic(_ haptic: Haptic) {}
    func cue(_ cue: Cue, step: Int) { cues.append(cue) }
    func cueUnlessRecent(_ cue: Cue) { cues.append(cue) }
}
