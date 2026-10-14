import XCTest
@testable import Reins

/// The approval screen's words, colours and lifetimes (the Android app's ApprovalWordsTest, case for case).
final class ApprovalWordsTests: XCTestCase {
    private typealias T = ApprovalLogicTests

    private func sentence(_ view: ApprovalView) -> String {
        ApprovalWords.sentence(view).map { $0.bold ? "**\($0.text)**" : $0.text }.joined()
    }

    private func ref(
        _ name: String = "main", kind: String = "branch", change: String = "update", force: Bool = false, forceUnknown: Bool = false,
        commitCount: UInt32 = 8, commits: [GitCommitView] = [], filesChanged: UInt32 = 12
    ) -> GitRefView {
        GitRefView(
            name: kind == "tag" ? "refs/tags/\(name)" : "refs/heads/\(name)", kind: kind, shortName: name, change: change, force: force,
            forceUnknown: forceUnknown, commitCount: commitCount, commits: commits, filesChanged: filesChanged, files: [],
            additions: 412, deletions: 96
        )
    }

    private func push(_ refs: [GitRefView], noStanding: Bool = false) -> ApprovalView {
        var v = T.view("req20", kind: .write, service: "github", account: "octo-cat", op: "git_push", noStanding: noStanding, opTitle: "Push with git", action: "write")
        v.git = GitPushView(repo: "octo/app", packBytes: 1024, notes: [], refs: refs)
        return v
    }

    private var deleteRepo: ApprovalView {
        T.view(
            "req11", kind: .write, service: "github", account: "octo-cat", op: "repo_delete",
            resources: [ResourceView(id: "octo/old-api", label: "octo/old-api", wider: false)], noStanding: true,
            opTitle: "Delete a GitHub repository", action: "write"
        )
    }

    func testTheHeroSaysWhatIsAskedInOneSentence() {
        XCTAssertEqual(sentence(push([ref()])), "Wants to **push 8 commits**\nto **octo/app**")
        let stripe = T.view(kind: .search, query: "from:stripe", messages: [T.message("m1", "Stripe <a@stripe.com>"), T.message("m2", "Stripe <b@stripe.com>")], count: 2)
        XCTAssertEqual(sentence(stripe), "Wants to **read 2 emails**\nfrom **Stripe**")
        let mixed = T.view(kind: .search, query: "from:bank", messages: [T.message("m1", "Bank <a@bank.com>"), T.message("m2", "x@y.com")], count: 2)
        XCTAssertEqual(sentence(mixed), "Wants to **read 2 emails**\nfound for **from:bank**")
        XCTAssertEqual(sentence(deleteRepo), "Wants to **delete** the repository\n**octo/old-api**")
        let send = T.view(kind: .send, email: EmailView(to: ["Ann <ann@corp.com>"], cc: ["bob@corp.com"], subject: "Hi", body: "b"), count: 2)
        XCTAssertEqual(sentence(send), "Wants to **send an email**\nto **Ann** and 1 more")
        XCTAssertEqual(sentence(push([ref("v1.2", kind: "tag", change: "create", commitCount: 0)])), "Wants to **push tag v1.2**\nto **octo/app**")
        XCTAssertEqual(sentence(push([ref("old", change: "delete", commitCount: 0)])), "Wants to **delete branch old**\nfrom **octo/app**")
        XCTAssertEqual(sentence(T.grantView()), "Wants to **read emails**\n**without asking**")
    }

    func testUntrustedTextCannotReorderTheSentence() {
        let view = T.view(kind: .search, messages: [T.message("m1", "Ev\u{202E}il <a@b.c>")])
        XCTAssertFalse(ApprovalWords.sentence(view).contains { $0.text.unicodeScalars.contains { $0.value == 0x202E } })
    }

    func testTheHeroIsTintedByWhatIsAsked() {
        XCTAssertEqual(ApprovalWords.tone(push([ref()])), .write)
        XCTAssertEqual(ApprovalWords.tone(T.view(kind: .search)), .search)
        XCTAssertEqual(ApprovalWords.tone(T.grantView()), .grant)
        XCTAssertEqual(ApprovalWords.tone(T.accountsView()), .grant)
        XCTAssertEqual(ApprovalWords.tone(deleteRepo), .danger)
        XCTAssertEqual(ApprovalWords.tone(T.view(kind: .fetch, service: "vault", noStanding: true, action: "read")), .search, "a password changes nothing")
        XCTAssertEqual(ApprovalWords.badge(deleteRepo), .trash)
        XCTAssertEqual(ApprovalWords.badge(push([ref()])), .pen)
    }

    func testApproveSaysWhatItDoes() {
        var draft = ApprovalDraft()
        draft.selected = ["m1"]
        let read = T.view(kind: .search, messages: [T.message("m1", "a@b.c"), T.message("m2", "a@b.c"), T.message("m3", "a@b.c", covered: true)], count: 3)
        XCTAssertEqual(ApprovalWords.approveLabel(read, draft), "Share 2")
        XCTAssertEqual(ApprovalWords.approveLabel(T.grantView(), draft), "Allow")
        XCTAssertEqual(ApprovalWords.approveLabel(deleteRepo, draft), "Delete")
        XCTAssertEqual(ApprovalWords.approveLabel(push([ref()]), draft), "Approve")
        XCTAssertTrue(ApprovalWords.rememberLocked(deleteRepo))
        XCTAssertFalse(ApprovalWords.rememberLocked(push([ref()])))
    }

    func testDangerousPushesCarryATagWithItsExplanation() {
        XCTAssertEqual(ApprovalWords.dangerTag(push([ref(force: true)], noStanding: true))?.id, "forceWarning:0")
        XCTAssertEqual(ApprovalWords.dangerTag(push([ref(force: true, forceUnknown: true)], noStanding: true))?.id, "forceUnknown:0")
        XCTAssertEqual(ApprovalWords.dangerTag(deleteRepo)?.id, "onceWarning")
        XCTAssertNil(ApprovalWords.dangerTag(push([ref()])))
        let hidden = GitCommitView(shortSha: "a1b2c3d", subject: "x", author: "a", date: 0, filesChanged: nil, files: [], additions: nil, deletions: nil, hiddenChars: true)
        XCTAssertEqual(ApprovalWords.dangerTag(push([ref(commits: [hidden])]))?.id, "hiddenChars")
    }

    func testADesktopQuestionShowsTheGuardRuleItMatchedRedOnlyWhenItCannotBeUndone() {
        var ask = T.view("req40", kind: .write, service: "desktop", op: "ask", opTitle: "Ask you on your phone", action: "write")
        let secret = AskRuleView(label: ".env files", pattern: ".env", matched: "app/.env", group: "env-files", destructive: false, reported: false)
        ask.ask = AskView(question: "Claude Code wants to run: cat app/.env", detail: "Command:\ncat app/.env", topic: "file:.env", rule: secret)
        let tag = ApprovalWords.dangerTag(ask)
        XCTAssertEqual(tag?.label, ".env files · app/.env")
        XCTAssertEqual(tag?.id, "guardRule")
        XCTAssertEqual(tag?.destructive, false)
        let force = AskRuleView(
            label: "Rewriting git history", pattern: "git push --force*", matched: "git push --force origin main", group: "git-history",
            destructive: true, reported: false
        )
        XCTAssertTrue(ApprovalWords.ruleTag(force).destructive)
        let own = AskRuleView(label: "Your rule", pattern: "make deploy", matched: "make deploy", group: nil, destructive: false, reported: true)
        XCTAssertTrue(ApprovalWords.ruleTag(own).help.contains("Your computer says so"))
    }

    func testTheDocksSegmentsAndTheCustomChoiceReadAsShortPeriods() {
        var d = ApprovalDraft()
        XCTAssertEqual(ApprovalWords.segment(d), .once)
        XCTAssertEqual(ApprovalWords.customLabel(d), "Custom")
        d.lifetime = .custom
        XCTAssertEqual(ApprovalWords.customLabel(d), "3 days")
        d.lifetime = .week
        XCTAssertEqual(ApprovalWords.customLabel(d), "1 week")
        XCTAssertEqual(ApprovalWords.segment(d), .custom)
        d.lifetime = .uses
        d.uses = 10
        XCTAssertEqual(ApprovalWords.customLabel(d), "10 uses")
        d.lifetime = .untilRevoked
        XCTAssertEqual(ApprovalWords.customLabel(d), "Forever")
        XCTAssertEqual(ApprovalWords.rememberButton(push([ref()]), d), "Remember until I revoke")
        d.lifetime = .custom
        d.customAmount = 2
        d.customUnit = .weeks
        XCTAssertEqual(ApprovalWords.rememberButton(push([ref()]), d), "Remember for 2 weeks")
        d.allMail = .hour
        XCTAssertEqual(ApprovalWords.customLabel(d), "All mail")
        XCTAssertEqual(ApprovalWords.period(86_400), "1 day")
        XCTAssertEqual(ApprovalWords.period(2_592_000), "30 days")
        XCTAssertEqual(ApprovalWords.seconds(40), "40s")
        XCTAssertEqual(ApprovalWords.seconds(101), "2m")
        XCTAssertTrue(ApprovalWords.splitPath("src/auth/login.ts") == ("src/auth/", "login.ts"))
        XCTAssertTrue(ApprovalWords.splitPath("README.md") == ("", "README.md"))
    }

    func testACustomTimeIsCountedInItsUnitAndBounded() {
        let read = T.view(kind: .search, messages: [T.message("m1", "a@b.c")])
        var d = ApprovalDraft()
        d.selected = ["m1"]
        d.lifetime = .custom
        XCTAssertEqual(lifetimeSeconds(read, d), 259_200)
        d.customAmount = 2
        d.customUnit = .weeks
        guard case let .ok(choice) = buildDecision(read, d) else { return XCTFail("built") }
        XCTAssertEqual(choice.standing?.durationSecs, 1_209_600)
        d.customAmount = 53
        guard case .invalid = buildDecision(read, d) else { return XCTFail("53 weeks is too long") }
        d.customAmount = 0
        guard case .invalid = buildDecision(read, d) else { return XCTFail("0 is nothing") }
    }
}
