import XCTest
@testable import Reins

/// "Approve and allow for a while" and "Approve all" (the Android app's QuickApprovalsFlowTest, the logic half).
final class QuickApprovalTests: XCTestCase {
    private func item(_ id: String, conn: String = "c1", label: String = "Claude", quick: Bool) -> PendingItem {
        PendingItem(
            kind: .request, id: id, title: "", subtitle: "", createdAt: 0, connectionId: conn, connectionLabel: label,
            action: "search", count: 1, service: "gmail", account: nil, waitUntil: nil, op: "", opTitle: "", suggestion: nil,
            headline: "", quick: quick
        )
    }

    func testABurstNeedsTwoRoutineRequestsFromOneAi() {
        let bursts = Burst.of([
            item("r1", quick: true), item("r2", quick: true), item("r3", quick: false),
            item("r4", conn: "c2", label: "Codex", quick: true),
        ])
        XCTAssertEqual(bursts, [Burst(connectionId: "c1", label: "Claude", quick: ["r1", "r2"], all: ["r1", "r2", "r3"])])
        XCTAssertEqual(bursts[0].title, "Claude · 3 waiting")
        XCTAssertEqual(bursts[0].heldNote, "1 needs a closer look")
        XCTAssertEqual(Burst.of([item("r1", quick: true), item("r2", quick: false)]), [])
    }

    func testBurstsOfSeveralAisAreOneBarOfTheirRoutineRequests() {
        let pending = [
            item("r1", quick: true), item("r2", quick: true), item("r3", quick: false),
            item("r4", conn: "c2", label: "Codex", quick: true), item("r5", conn: "c2", label: "Codex", quick: true),
        ]
        let bar = Burst.bar(pending)
        XCTAssertEqual(bar, Burst(connectionId: "", label: "Claude and Codex", quick: ["r1", "r2", "r4", "r5"], all: ["r1", "r2", "r4", "r5"], ais: 2))
        XCTAssertEqual(bar?.title, "4 routine · Claude and Codex", "says who asks")
        XCTAssertEqual(Burst.names(["A", "B", "C", "D"]), "A, B and 2 more")
        XCTAssertNil(bar?.heldNote, "what needs a look is not in it")
        XCTAssertEqual(Burst.bar(Array(pending.prefix(3)))?.connectionId, "c1", "one AI: its own bar")
        XCTAssertNil(Burst.bar([item("r1", quick: true)]))
    }

    func testOneHourInTheDockIsTheCoresAllowForAWhile() {
        var view = ApprovalLogicTests.view(kind: .search, messages: [ApprovalLogicTests.message("m1", "Bank <alerts@bank.com>")], count: 1)
        XCTAssertEqual(ApprovalWords.shortLabel(view), "1 hour")
        let scope = GrantScopeChoice(
            allMail: true, selectedMessagesOnly: false, senderAddresses: [], senderDomains: [], subjectPattern: nil,
            recipientAddresses: [], recipientDomains: [], resources: [], classes: []
        )
        view.quick = QuickApproval(fromNotification: true, allow: StandingGrant(durationSecs: 28_800, maxUses: nil, scope: scope), allowWhat: "searching and reading me@gmail.com", repeats: 3)
        XCTAssertEqual(ApprovalWords.shortLabel(view), "8 hours", "repeats get the core's longer period")
        // Remembering covers what the screen shows: the ticked email, never all mail, unless chosen.
        var draft = ApprovalDraft.initial(for: view)
        draft.lifetime = .day
        guard case let .ok(day) = buildDecision(view, draft) else { return XCTFail("built") }
        XCTAssertEqual(day.standing?.scope.allMail, false)
        XCTAssertEqual(day.standing?.scope.selectedMessagesOnly, true)
        XCTAssertFalse(ApprovalWords.likeThisOn(view, draft))
        draft.lifetime = .hour
        guard case let .ok(hour) = buildDecision(view, draft) else { return XCTFail("built") }
        XCTAssertEqual(hour.standing?.durationSecs, 28_800)
        XCTAssertEqual(hour.standing?.scope.allMail, false)
        draft.lifetime = .untilRevoked
        guard case let .ok(forever) = buildDecision(view, draft) else { return XCTFail("built") }
        XCTAssertEqual(forever.standing?.scope.allMail, false, "until revoked is never all mail")
        // Chosen explicitly: the core's scope, for the time picked; still never open-ended for all mail.
        draft.likeThis = true
        draft.lifetime = .hour
        guard case let .ok(chosen) = buildDecision(view, draft) else { return XCTFail("built") }
        XCTAssertEqual(chosen.standing, view.quick?.allow)
        XCTAssertTrue(ApprovalWords.likeThisOn(view, draft))
        draft.lifetime = .untilRevoked
        guard case .invalid = buildDecision(view, draft) else { return XCTFail("all mail until revoked is refused") }
    }

    func testTheStartingRuleSaysWhatItGivesOnlyWhenItGivesSomething() {
        XCTAssertEqual(StartingRuleText.title(.readsForADay), "Let it read for a day")
        XCTAssertNotNil(StartingRuleText.onPairing(.readsForADay))
        XCTAssertNil(StartingRuleText.onPairing(.askEveryTime))
        XCTAssertNil(StartingRuleText.onPairing(nil), "not chosen yet: it asks for everything")
    }

    func testAnEmailThatLooksLikeACodeIsNeverTickedForTheUser() {
        let view = ApprovalLogicTests.view(kind: .search, messages: [
            ApprovalLogicTests.message("m1", "Bank <alerts@bank.com>"),
            ApprovalLogicTests.message("m2", "Bank <alerts@bank.com>", sensitive: true),
        ], count: 2)
        XCTAssertEqual(ApprovalDraft.initial(for: view).selected, ["m1"])
    }
}
