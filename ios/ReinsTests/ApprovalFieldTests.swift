import XCTest
@testable import Reins

/// The approval's field: its label line and ask, and its warning lines, one test per flag. Each warning says its own
/// words on a line of its own; none hides behind another, a fold or only a (?).
final class ApprovalFieldTests: XCTestCase {
    private typealias T = ApprovalLogicTests

    private func lines(_ view: ApprovalView, suggestion: SuggestionView? = nil, risk: RiskView? = nil, riskInFacts: Bool = false) -> [FieldLine] {
        FieldLines.of(view, suggestion: suggestion, risk: risk, riskInFacts: riskInFacts)
    }

    private func line(_ all: [FieldLine], _ id: String, file: StaticString = #filePath, lineNo: UInt = #line) -> FieldLine? {
        let found = all.first { $0.id == id }
        XCTAssertNotNil(found, "no \(id) in \(all.map(\.id))", file: file, line: lineNo)
        return found
    }

    private func ref(_ name: String = "feature/x", force: Bool = false, forceUnknown: Bool = false, commits: [GitCommitView] = []) -> GitRefView {
        GitRefView(
            name: "refs/heads/\(name)", kind: "branch", shortName: name, change: "update", force: force, forceUnknown: forceUnknown,
            commitCount: 8, commits: commits, filesChanged: 0, files: [], additions: 1, deletions: 0
        )
    }

    private func push(_ refs: [GitRefView], defaultBranch: String? = nil) -> ApprovalView {
        var v = T.view("req20", kind: .write, service: "github", op: "git_push", opTitle: "Push with git", action: "write")
        v.git = GitPushView(repo: "octo/app", packBytes: 0, notes: [], refs: refs, defaultBranch: defaultBranch)
        return v
    }

    private func ask(rule: AskRuleView? = nil, uncertain: Bool = false, destructive: Bool = false, noStanding: Bool = false) -> ApprovalView {
        var v = T.view("q1", kind: .write, service: "", op: "", noStanding: noStanding, action: "write")
        v.ask = AskView(question: "Run it?", detail: "rm -rf build", topic: nil, rule: rule, uncertain: uncertain, destructive: destructive)
        return v
    }

    private let envRule = AskRuleView(label: ".env files", pattern: ".env", matched: "app/.env", group: nil, destructive: false, reported: false)
    private let forceRule = AskRuleView(
        label: "Rewriting git history", pattern: "git push --force*", matched: "git push --force origin main", group: nil, destructive: true, reported: false
    )

    private func mcp(destructive: Bool) -> ApprovalView {
        var v = T.view("m1", kind: .write, service: "mcp:linear", op: "delete_project", action: "write")
        v.mcp = McpCallView(
            serverName: "Linear", serverUrl: "https://mcp.linear.app", tool: "delete_project", title: "Delete project",
            description: "", argumentsJson: "{}", readOnly: false, destructive: destructive
        )
        return v
    }

    private func ssh(host: String?, hostKey: String?) -> ApprovalView {
        var v = T.view("s1", kind: .write, service: "ssh", op: "ssh_sign", noStanding: true, action: "write")
        v.ssh = SshSignView(keyName: "work", keyFingerprint: "SHA256:abc", host: host, hostKey: hostKey)
        return v
    }

    private func purchase() -> ApprovalView {
        var v = T.view("p1", kind: .write, service: "pay", op: "purchase", action: "write")
        v.purchase = PurchaseView(
            merchant: "Acme", domain: "acme.com", merchantUrl: "https://acme.com", checkoutUrl: "https://acme.com/pay", checkoutHost: "acme.com",
            items: [], subtotal: "$20.00", shipping: nil, tax: nil, discount: nil, total: "$20.00", totalMinor: 2000, currency: "USD", note: nil,
            methods: [], methodId: nil, ships: false, addresses: [], addressId: nil, warnings: [], budgetLines: [], limitMethods: [], delivery: nil
        )
        return v
    }

    private func suggestion(novel: Bool = false, floor: Bool = false) -> SuggestionView {
        SuggestionView(
            requestId: "r", verdict: .approve, mode: .manual, pApprove: 0.97, pDeny: 0.02, confidence: 0.9, reason: "", neighbours: [],
            profileId: "", profileName: "", classKey: "", novel: novel, floor: floor, judged: !floor, risk: nil
        )
    }

    private func hidden() -> GitCommitView {
        GitCommitView(shortSha: "a1b2c3d", subject: "x", author: "a", date: 0, filesChanged: nil, files: [], additions: nil, deletions: nil, hiddenChars: true)
    }

    func testManagedMailExplainsOneTimeApprovalWithoutPromisingEveryFutureRequestAsks() {
        let view = T.view(kind: .send, service: "gmail", noStanding: true, action: "send")
        XCTAssertTrue(ApprovalWords.rememberLocked(view))
        XCTAssertTrue(ApprovalWords.mailPermissionSettings(view))
        let warning = lines(view).first(where: { $0.id == "alwaysAsks" })
        XCTAssertEqual(warning?.text, "This approval is once only. Manage ongoing access in Trust.")
        XCTAssertFalse(warning?.text.contains("never remembered") == true)
    }

    // MARK: The ask (each kind: AskFormatTests)

    func testALongQuestionIsWholeOnTheLineBelow() {
        var v = ask()
        v.ask?.question = String(repeating: "Should this run on the build server? ", count: 60) + "THE END"
        XCTAssertEqual(AskFormat.waiting(v).role, .ask, "the sentence is Reins' own and short")
        XCTAssertTrue(AskFormat.waiting(v).mono?.contains(v.ask!.question) == true, "the question is below, never cut")
    }

    func testTheOtherSidesServerNameIsOneShortLine() {
        var v = mcp(destructive: false)
        v.mcp?.serverName = "Linear\n" + String(repeating: "x", count: 200)
        let label = AskFormat.label(v)
        XCTAssertFalse(label.contains("\n"))
        XCTAssertLessThan(label.count, 90)
    }

    // MARK: One line per warning

    func testAGuardRuleSaysItsNameAndWhatMatched() {
        let w = line(lines(ask(rule: envRule)), "guardRule")
        XCTAssertEqual(w?.text, ".env files · app/.env")
        XCTAssertEqual(w?.tone, .amber)
    }

    func testARedGuardRuleSaysItAlwaysAsks() {
        let w = line(lines(ask(rule: forceRule)), "guardRule")
        XCTAssertEqual(w?.text, "Always asks · rewriting git history · git push --force origin main")
        XCTAssertEqual(w?.tone, .red)
    }

    func testARuleTheComputerReportedIsSaidOnItsOwnLine() {
        let own = AskRuleView(label: "Your rule", pattern: "make deploy", matched: "make deploy", group: nil, destructive: false, reported: true)
        XCTAssertEqual(line(lines(ask(rule: own)), "ruleReported")?.text, "Your computer says so; this phone doesn't know the rule")
        XCTAssertFalse(lines(ask(rule: envRule)).contains { $0.id == "ruleReported" })
    }

    func testACommandThisPhoneCouldNotReadIsSaidWithOrWithoutARule() {
        XCTAssertEqual(line(lines(ask(uncertain: true)), "askUncertain")?.text, "Couldn't read this")
        XCTAssertEqual(lines(ask(rule: envRule, uncertain: true)).map(\.id).filter { ["guardRule", "askUncertain"].contains($0) }, ["guardRule", "askUncertain"])
        XCTAssertNotNil(line(lines(ask(uncertain: true, destructive: true)), "askUncertain"), "beside can't be undone too")
    }

    func testAForcePushRewritesHistory() {
        let w = line(lines(push([ref(force: true)])), "forceWarning:0")
        XCTAssertEqual(w?.text, "Always asks · rewrites history")
        XCTAssertEqual(w?.tone, .red)
    }

    func testAPushWhoseHistoryWasNotChecked() {
        XCTAssertEqual(line(lines(push([ref(force: true, forceUnknown: true)])), "forceUnknown:0")?.text, "Always asks · history not checked")
    }

    func testHiddenCharactersAreSaidBesideAForcePush() {
        let all = lines(push([ref(force: true, commits: [hidden()])]))
        XCTAssertEqual(line(all, "forceWarning:0")?.text, "Always asks · rewrites history")
        XCTAssertEqual(line(all, "hiddenChars")?.text, "Always asks · hidden characters")
    }

    func testAPushToTheDefaultBranchSaysSoAndIsNotRed() {
        let w = line(lines(push([ref("main")], defaultBranch: "main")), "defaultBranch")
        XCTAssertEqual(w?.text, "To the default branch main")
        XCTAssertEqual(w?.tone, .amber)
        XCTAssertFalse(lines(push([ref("feature/x")], defaultBranch: "main")).contains { $0.id == "defaultBranch" })
    }

    func testADestructiveToolCannotBeUndone() {
        XCTAssertEqual(line(lines(mcp(destructive: true)), "mcpDestructive")?.text, "Always asks · can't be undone")
        var asked = mcp(destructive: true)
        asked.noStanding = true
        XCTAssertEqual(lines(asked).filter { $0.text == "Always asks · can't be undone" }.count, 1, "said once")
    }

    func testADeleteCannotBeUndone() {
        let delete = T.view("req11", kind: .write, service: "github", op: "repo_delete", noStanding: true, opTitle: "Delete a GitHub repository", action: "write")
        let w = line(lines(delete), "onceWarning")
        XCTAssertEqual(w?.text, "Always asks · can't be undone")
        XCTAssertEqual(w?.tone, .red)
    }

    func testAnSshServerWithNoName() {
        XCTAssertEqual(line(lines(ssh(host: nil, hostKey: "ssh-ed25519 AAAA")), "unknownHost")?.text, "Server known only by its key")
        XCTAssertEqual(line(lines(ssh(host: nil, hostKey: nil)), "unknownHost")?.text, "Unknown server")
        XCTAssertFalse(lines(ssh(host: "build.example.com", hostKey: "k")).contains { $0.id == "unknownHost" })
    }

    func testWhyItIsAskedEveryTimeWhenNoRedLineSaysSo() {
        XCTAssertEqual(line(lines(purchase()), "alwaysAsks")?.text, "Always asks · money can't come back")
        var printed = T.view("e1", kind: .write, service: "secrets", op: "env_release", action: "write")
        printed.envRelease = EnvReleaseView(
            project: "web", env: "prod", runner: "c1", runnerLabel: "Claude", agent: true, root: "~/web", command: "npm run show",
            program: "/usr/bin/npm", programWarning: nil, variables: ["API_KEY"], allVariables: false, leaseSecs: 60, purpose: nil,
            commands: [], print: true, alwaysAsk: false, alwaysAskSet: false
        )
        XCTAssertEqual(line(lines(printed), "alwaysAsks")?.text, "Always asks · it prints a value")
        XCTAssertFalse(lines(push([ref(force: true)])).contains { $0.id == "alwaysAsks" }, "the red line already says it")
        XCTAssertFalse(lines(T.view(kind: .search)).contains { $0.id == "alwaysAsks" })
    }

    func testAPaymentIsHeld() {
        XCTAssertTrue(ApprovalWords.held(purchase()))
        XCTAssertEqual(ApprovalWords.tone(purchase()).field, .delete)
    }

    func testTheFirstTimeForThisAI() {
        XCTAssertEqual(line(lines(T.view(kind: .search), suggestion: suggestion(novel: true)), "novel")?.text, "First time for this AI")
        XCTAssertFalse(lines(T.view(kind: .search), suggestion: suggestion()).contains { $0.id == "novel" })
    }

    func testTheRiskWrapsToTwoLinesAndTheHoldHasItsOwn() {
        let risk = RiskView(level: .catastrophic, pCatastrophic: 0.4, why: "deletes the default branch main", held: true)
        let all = lines(T.view(kind: .search), risk: risk)
        XCTAssertEqual(line(all, "risk")?.text, "Risk: catastrophic · deletes the default branch main")
        XCTAssertEqual(line(all, "risk")?.maxLines, 2)
        XCTAssertEqual(line(all, "riskHeld")?.text, "Held by Overnight")
        let short = lines(T.view(kind: .search), risk: risk, riskInFacts: true)
        XCTAssertEqual(line(short, "risk")?.text, "Risk: catastrophic", "its reason went to the first fact")
        XCTAssertFalse(lines(T.view(kind: .search), risk: RiskView(level: .severe, pCatastrophic: 0, why: "", held: false)).contains { $0.id == "riskHeld" })
    }

    func testEveryOtherLineIsNeverCut() {
        let long = AskRuleView(label: "Deletes files", pattern: "rm *", matched: String(repeating: "a ", count: 500) + "rm -rf ~", group: nil, destructive: true, reported: false)
        let w = line(lines(ask(rule: long)), "guardRule")
        XCTAssertNil(w?.maxLines)
        XCTAssertTrue(w?.text.hasSuffix("rm -rf ~") == true, "a long command's tail stays on screen")
    }

    func testAnOrdinaryRequestHasNoWarnings() {
        XCTAssertTrue(lines(T.view(kind: .search), suggestion: suggestion()).isEmpty)
        XCTAssertTrue(lines(push([ref()])).isEmpty)
    }
}
