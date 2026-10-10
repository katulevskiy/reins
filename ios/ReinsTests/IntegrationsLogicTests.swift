import XCTest
@testable import Reins

/// The integration screens' pure logic (Android's `GithubTokenTest` and the MCP / git host parts of
/// `NewFeaturesLogicTest`).
final class IntegrationsLogicTests: XCTestCase {
    // MARK: GitHub

    func testOnlyThingsThatLookLikeAGitHubTokenArePickedUp() {
        XCTAssertTrue(GitHubToken.looksLikeToken("github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz_0123456789"))
        XCTAssertTrue(GitHubToken.looksLikeToken("  ghp_abcdefghijklmnopqrstuvwxyz0123456789\n"))
        for other: String? in [nil, "", "hello", "https://github.com", "ghp_short", "my password is hunter2 and more words here",
                               "xghp_abcdefghijklmnopqrstuvwxyz0123456789", "ghp_abcdefghijklmnopqrstuvwxyz0123456789 extra"] {
            XCTAssertFalse(GitHubToken.looksLikeToken(other), other ?? "nil")
        }
    }

    func testTheFineGrainedPageAsksForWhatTheToolsNeedAndNothingOutsideARepository() {
        for permission in [
            "contents=write", "issues=write", "pull_requests=write", "actions=write", "workflows=write", "administration=write",
            "repository_hooks=write", "secrets=write", "variables=write", "environments=write", "checks=write", "statuses=write",
            "security_events=read", "vulnerability_alerts=read", "secret_scanning_alerts=read", "metadata=read",
        ] {
            XCTAssertTrue(GitHubToken.fineGrainedPage.contains("&\(permission)"), permission)
        }
        XCTAssertFalse(GitHubToken.fineGrainedPage.contains("gist") || GitHubToken.fineGrainedPage.contains("notifications"))
        XCTAssertTrue(GitHubToken.fineGrainedPage.hasPrefix("https://github.com/settings/personal-access-tokens/new?name=Reins&"))
    }

    func testTheClassicPageAsksForEveryScopeTheToolsUse() {
        XCTAssertEqual(
            GitHubToken.classicPage,
            "https://github.com/settings/tokens/new?description=Reins&scopes=repo,workflow,gist,notifications,read:org,admin:repo_hook,delete_repo"
        )
    }

    private func names(_ make: () -> String, after key: String) -> Set<String> {
        Set((1...20).map { _ in
            let url = make()
            let start = url.range(of: key)?.upperBound ?? url.endIndex
            return String(url[start...].prefix { $0 != "&" })
        })
    }

    func testEveryTokenPageGetsItsOwnNameSoASecondTokenNeverClashes() {
        XCTAssertTrue(GitHubToken.fineGrainedURL(suffix: 123_456).contains("name=Reins-123456&"))
        let fine = names({ GitHubToken.fineGrainedURL() }, after: "name=")
        XCTAssertGreaterThan(fine.count, 1)
        XCTAssertTrue(fine.allSatisfy { $0.wholeMatch(of: #/Reins-\d{6}/#) != nil })

        XCTAssertEqual(
            GitHubToken.classicURL(suffix: 123_456),
            "https://github.com/settings/tokens/new?description=Reins-123456&scopes=repo,workflow,gist,notifications,read:org,admin:repo_hook,delete_repo"
        )
        let classic = names({ GitHubToken.classicURL() }, after: "description=")
        XCTAssertGreaterThan(classic.count, 1)
        XCTAssertTrue(classic.allSatisfy { $0.wholeMatch(of: #/Reins-\d{6}/#) != nil })
    }

    // MARK: Other git hosts

    func testGitLabsPageIsFilledInWithAFreshNameAndTheScopesGitNeeds() throws {
        let gitlab = try XCTUnwrap(GitHosts.of("gitlab"))
        XCTAssertEqual(
            gitlab.tokenURL(suffix: 123_456),
            "https://gitlab.com/-/user_settings/personal_access_tokens?name=Reins-123456&scopes=read_api,read_repository,write_repository"
        )
        XCTAssertGreaterThan(names({ gitlab.tokenURL() }, after: "name=").count, 1)
    }

    func testCodebergAndBitbucketOpenTheirTokenPages() {
        XCTAssertEqual(GitHosts.of("codeberg")?.tokenURL(), "https://codeberg.org/user/settings/applications")
        XCTAssertEqual(GitHosts.of("bitbucket")?.tokenURL(), "https://id.atlassian.com/manage-profile/security/api-tokens")
        XCTAssertNil(GitHosts.of("github"))
        XCTAssertEqual(GitHosts.of("bitbucket")?.pasteFirst, true)
        XCTAssertEqual(GitHosts.of("gitlab")?.pasteFirst, false)
    }

    func testOnlyRealLookingTokensArePickedUp() throws {
        let gitlab = try XCTUnwrap(GitHosts.of("gitlab"))
        XCTAssertTrue(gitlab.looksLikeToken("glpat-abcdefghijklmnopqrst"))
        XCTAssertFalse(gitlab.looksLikeToken("hello world"))
        XCTAssertFalse(gitlab.looksLikeToken(nil))
        let codeberg = try XCTUnwrap(GitHosts.of("codeberg"))
        XCTAssertTrue(codeberg.looksLikeToken("0123456789abcdef0123456789abcdef01234567"))
        XCTAssertFalse(codeberg.looksLikeToken("0123456789abcdef"))
        // Bitbucket needs the email too, so nothing is ever taken from the clipboard by itself.
        XCTAssertFalse(try XCTUnwrap(GitHosts.of("bitbucket")).looksLikeToken("ATATT3xFfGF0abcdefghijklmnop"))
    }

    // MARK: MCP

    func testAServerIsShownByItsHostNeverWithAQuery() {
        XCTAssertEqual(McpLogic.host("https://mcp.linear.app/mcp"), "mcp.linear.app")
        XCTAssertEqual(McpLogic.host("https://mcp.notion.com/mcp?key=secret"), "mcp.notion.com")
        XCTAssertEqual(McpLogic.host("http://localhost:8080/mcp"), "localhost:8080")
        XCTAssertEqual(McpLogic.host("not a url"), "not a url")
    }

    func testStatusesAndToolCountsReadAsWords() {
        XCTAssertEqual(McpLogic.statusLabel("ok"), "Connected")
        XCTAssertEqual(McpLogic.statusLabel("needs_sign_in"), "Needs sign-in")
        XCTAssertEqual(McpLogic.statusLabel("error"), "Error")
        XCTAssertEqual(McpLogic.toolCount(0), "No tools")
        XCTAssertEqual(McpLogic.toolCount(1), "1 tool")
        XCTAssertEqual(McpLogic.toolCount(4), "4 tools")
    }

    func testEachToolSaysWhetherItChangesThingsIsAskedEveryTimeAndIsHeavy() {
        let tools = DemoData.mcpTools()
        XCTAssertEqual(tools.map(ToolBadge.of), [[.readOnly], [.changes], [.changes, .asksEveryTime], [.readOnly, .heavy]])
        XCTAssertEqual(ToolBadge.readOnly.label, "Read only")
        XCTAssertEqual(ToolBadge.changes.label, "Changes things")
        XCTAssertEqual(ToolBadge.asksEveryTime.label, "Asks every time")
        XCTAssertEqual(ToolBadge.heavy.label, "Large results")
    }

    func testOnlyTheAppsOwnRedirectFinishesASignIn() {
        XCTAssertTrue(McpLogic.isRedirect("com.reins2fa.app://mcp-oauth?code=a&state=b"))
        XCTAssertTrue(McpLogic.isRedirect("com.reins2fa.app://mcp-oauth/?error=access_denied&state=b"))
        XCTAssertFalse(McpLogic.isRedirect(nil))
        XCTAssertFalse(McpLogic.isRedirect("https://evil.example.com/mcp-oauth?code=a"))
        XCTAssertFalse(McpLogic.isRedirect("com.reins2fa.app://other?code=a"))
        XCTAssertFalse(McpLogic.isRedirect("com.reins2fa.app://mcp-oauth.evil.com?code=a"))
        XCTAssertFalse(McpLogic.isRedirect("com.reins2fa.app://user@mcp-oauth?code=a"))
        XCTAssertFalse(McpLogic.isRedirect("com.reins2fa.app://mcp-oauth?code=" + String(repeating: "a", count: 9_000)))
    }

    func testOnlyWebPagesAreOpenedForASignIn() {
        XCTAssertTrue(McpLogic.isWebPage("https://linear.app/oauth/authorize?x=1"))
        XCTAssertTrue(McpLogic.isWebPage("http://localhost:3000/authorize"))
        XCTAssertFalse(McpLogic.isWebPage("intent://evil#Intent;end"))
        XCTAssertFalse(McpLogic.isWebPage("javascript:alert(1)"))
        XCTAssertFalse(McpLogic.isWebPage("file:///sdcard/x"))
        XCTAssertFalse(McpLogic.isWebPage("https://"))
    }

    // MARK: Accounts

    func testTelegramStepsCheckWhatWasTyped() {
        XCTAssertTrue(TelegramStep.phoneReady("+1 555 010 0100"))
        XCTAssertFalse(TelegramStep.phoneReady("+1 555"))
        XCTAssertEqual(TelegramStep.cleanCode("12 34-5678 90"), "12345678")
        XCTAssertEqual(TelegramStep.cleanCode("１２３４"), "")
        XCTAssertTrue(TelegramStep.codeReady("1234"))
        XCTAssertFalse(TelegramStep.codeReady("123"))
    }

    private func service(_ id: String, kind: String, accounts: Int = 0, available: Bool = true) -> ServiceView {
        ServiceView(service: id, name: id.capitalized, kind: kind, available: available, note: nil,
                    accounts: (0..<accounts).map { AccountView(service: id, account: "a\($0)", addedAt: 0) })
    }

    func testTheListSaysHowManyAccountsEachServiceHas() {
        XCTAssertEqual(ServiceCopy.summary(service("github", kind: "token")), "")
        XCTAssertEqual(ServiceCopy.summary(service("github", kind: "token", accounts: 1)), "1 account")
        XCTAssertEqual(ServiceCopy.summary(service("github", kind: "token", accounts: 3)), "3 accounts")
        XCTAssertEqual(ServiceCopy.summary(ServiceCopy.sms), "Not available")
        XCTAssertFalse(ServiceCopy.sms.available)
    }

    func testEachKindOfServiceSaysWhatToDoWhenItNeedsTheUserAgain() {
        XCTAssertEqual(ServiceCopy.needsAgain(service("telegram", kind: "telegram")), "Signed out")
        XCTAssertEqual(ServiceCopy.needsAgain(service("github", kind: "token")), "Token expired")
        XCTAssertEqual(ServiceCopy.needsAgain(service("gcalendar", kind: "google")), "Allow again")
        XCTAssertTrue(ServiceCopy.canAllowAgain(service("gcalendar", kind: "google")))
        XCTAssertTrue(ServiceCopy.canAllowAgain(service("device_contacts", kind: "device")))
        XCTAssertFalse(ServiceCopy.canAllowAgain(service("telegram", kind: "telegram")))
    }

    func testThisPhonesServicesAreNamedByTheService() {
        let phone = ServiceView(service: "device_calendar", name: "Phone calendar", kind: "device", available: true, note: nil, accounts: [])
        XCTAssertEqual(ServiceCopy.removeTitle(phone, "this phone"), "Remove Phone calendar?")
        XCTAssertEqual(ServiceCopy.removeTitle(service("telegram", kind: "telegram"), "+15550100"), "Remove +15550100?")
        XCTAssertEqual(ServiceCopy.emptyTitle(service("gmail", kind: "google")), "No account yet")
        XCTAssertEqual(ServiceCopy.emptyTitle(service("vault", kind: "vault")), "Not connected")
        XCTAssertEqual(ServiceCopy.emptyTitle(ServiceCopy.sms), "Not available")
    }

    func testTheIntegrationsDeepLinkRoundTrips() {
        XCTAssertEqual(DeepLink.integrations.url.absoluteString, "reins://integrations")
        XCTAssertEqual(DeepLink(url: DeepLink.integrations.url), .integrations)
    }
}
