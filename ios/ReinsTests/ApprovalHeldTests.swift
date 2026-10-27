import XCTest
@testable import Reins

/// Held, not tapped: every red warning makes the action a hold, locks the chooser to "Asked every time" and answers
/// once, never remembered. One test per warning; the amber ones stay ordinary questions.
final class ApprovalHeldTests: XCTestCase {
    private typealias T = ApprovalLogicTests

    private func ref(
        _ name: String = "feature/passkeys", change: String = "update", force: Bool = false, forceUnknown: Bool = false,
        commits: [GitCommitView] = [], files: [GitFileView] = []
    ) -> GitRefView {
        GitRefView(
            name: "refs/heads/\(name)", kind: "branch", shortName: name, change: change, force: force, forceUnknown: forceUnknown,
            commitCount: 1, commits: commits, filesChanged: UInt32(files.count), files: files, additions: 1, deletions: 0
        )
    }

    private func push(_ refs: [GitRefView], defaultBranch: String? = nil) -> ApprovalView {
        var v = T.view("req20", kind: .write, service: "github", op: "git_push", opTitle: "Push with git", action: "write")
        v.git = GitPushView(repo: "octo/app", packBytes: 0, notes: [], refs: refs, defaultBranch: defaultBranch)
        return v
    }

    private func ask(rule: AskRuleView? = nil, uncertain: Bool = false, destructive: Bool = false, net: String? = nil) -> ApprovalView {
        var v = T.view("q1", kind: .write, service: "", op: "", action: "write")
        v.ask = AskView(question: "Run it?", detail: "rm -rf build", topic: nil, rule: rule, uncertain: uncertain, destructive: destructive, net: net)
        return v
    }

    private func rule(destructive: Bool, reported: Bool = false) -> AskRuleView {
        AskRuleView(label: "Deletes files", pattern: "rm -rf *", matched: "rm -rf build", group: nil, destructive: destructive, reported: reported)
    }

    private func mcp(destructive: Bool) -> ApprovalView {
        var v = T.view("m1", kind: .write, service: "mcp:linear", op: "delete_project", action: "write")
        v.mcp = McpCallView(
            serverName: "Linear", serverUrl: "https://mcp.linear.app", tool: "delete_project", title: "Delete project",
            description: "", argumentsJson: "{}", readOnly: false, destructive: destructive
        )
        return v
    }

    /// Held: a hold, the chooser locked, and an approval for a day still answers once.
    private func assertHeld(_ view: ApprovalView, tag: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(ApprovalWords.dangerTags(view).contains { $0.id == tag && $0.destructive }, "\(tag) is red", file: file, line: line)
        XCTAssertTrue(ApprovalWords.held(view), "\(tag) is held", file: file, line: line)
        XCTAssertEqual(ApprovalWords.tone(view), .danger, "\(tag) paints the delete field", file: file, line: line)
        var draft = ApprovalDraft()
        draft.lifetime = .day
        draft.likeThis = true
        if case let .ok(choice) = buildDecision(view, draft) {
            XCTAssertNil(choice.standing, "\(tag) is never remembered", file: file, line: line)
        }
    }

    private func assertNotHeld(_ view: ApprovalView, tag: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(ApprovalWords.dangerTags(view).contains { $0.id == tag && !$0.destructive }, "\(tag) shows, amber", file: file, line: line)
        XCTAssertFalse(ApprovalWords.held(view), "\(tag) alone is an ordinary question", file: file, line: line)
    }

    func testAForcePushIsHeld() { assertHeld(push([ref(force: true)]), tag: "forceWarning:0") }

    func testUncheckedHistoryIsHeld() { assertHeld(push([ref(force: true, forceUnknown: true)]), tag: "forceUnknown:0") }

    func testHiddenCharactersAreHeld() {
        let hidden = GitCommitView(shortSha: "a1b2c3d", subject: "x", author: "a", date: 0, filesChanged: nil, files: [], additions: nil, deletions: nil, hiddenChars: true)
        assertHeld(push([ref(commits: [hidden])]), tag: "hiddenChars")
        // A ref with only files listed says so too.
        let file = GitFileView(path: "a.ts", status: "modified", additions: 1, deletions: 0, binary: false, hiddenChars: true)
        assertHeld(push([ref(files: [file])]), tag: "hiddenChars")
    }

    func testADestructiveToolIsHeld() {
        assertHeld(mcp(destructive: true), tag: "mcpDestructive")
        XCTAssertFalse(ApprovalWords.held(mcp(destructive: false)))
    }

    func testWhatCannotBeUndoneIsHeld() {
        let delete = T.view("req11", kind: .write, service: "github", op: "repo_delete", noStanding: true, opTitle: "Delete a GitHub repository", action: "write")
        assertHeld(delete, tag: "onceWarning")
    }

    func testADestructiveGuardRuleIsHeldAndAnOrdinaryOneIsNot() {
        assertHeld(ask(rule: rule(destructive: true)), tag: "guardRule")
        assertNotHeld(ask(rule: rule(destructive: false)), tag: "guardRule")
    }

    func testASafetyNetShowsAndNeverLoosens() {
        let netted = ask(rule: rule(destructive: true), destructive: true, net: "Keeps 1 deleted path in the Reins trash for 7 days")
        let tag = ApprovalWords.dangerTags(netted).first { $0.id == "netted" }
        XCTAssertEqual(tag?.label, "Netted")
        XCTAssertEqual(tag?.destructive, false)
        XCTAssertTrue(tag?.help.contains("Reins trash") ?? false)
        // Still held: the phone never trusts the net to make a delete undoable.
        assertHeld(netted, tag: "guardRule")
        XCTAssertFalse(ApprovalWords.dangerTags(ask(rule: rule(destructive: true))).contains { $0.id == "netted" })
    }

    func testARuleOnlyTheComputerKnowsSaysSo() {
        let reported = ask(rule: rule(destructive: false, reported: true))
        XCTAssertTrue(ApprovalWords.dangerTags(reported).contains { $0.id == "ruleReported" })
    }

    func testCouldNotReadShowsEvenBesideCannotBeUndone() {
        assertNotHeld(ask(uncertain: true), tag: "askUncertain")
        let both = ask(uncertain: true, destructive: true)
        XCTAssertEqual(ApprovalWords.dangerTags(both).map(\.id), ["askUncertain", "onceWarning"], "neither hides the other")
        XCTAssertTrue(ApprovalWords.held(both))
    }

    func testAPushToTheDefaultBranchSaysSoAndIsNotHeld() {
        assertNotHeld(push([ref("main")]), tag: "defaultBranch")
        assertNotHeld(push([ref("develop")], defaultBranch: "develop"), tag: "defaultBranch")
        XCTAssertNil(ApprovalWords.defaultBranchPush(GitPushView(repo: "r", packBytes: 0, notes: [], refs: [ref("main")], defaultBranch: "trunk")))
        XCTAssertNil(ApprovalWords.defaultBranchPush(GitPushView(repo: "r", packBytes: 0, notes: [], refs: [ref("main", change: "delete")])))
    }

    func testEveryWarningShowsNotOnlyTheFirst() {
        let hidden = GitCommitView(shortSha: "a", subject: "x", author: "a", date: 0, filesChanged: nil, files: [], additions: nil, deletions: nil, hiddenChars: true)
        let tags = ApprovalWords.dangerTags(push([ref("main", force: true, commits: [hidden])])).map(\.id)
        XCTAssertEqual(tags, ["forceWarning:0", "hiddenChars", "defaultBranch"])
    }
}
