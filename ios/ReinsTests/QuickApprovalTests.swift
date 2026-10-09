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
        XCTAssertEqual(bar, Burst(connectionId: "", label: "", quick: ["r1", "r2", "r4", "r5"], all: ["r1", "r2", "r4", "r5"], ais: 2))
        XCTAssertEqual(bar?.title, "4 routine · 2 AIs")
        XCTAssertNil(bar?.heldNote, "what needs a look is not in it")
        XCTAssertEqual(Burst.bar(Array(pending.prefix(3)))?.connectionId, "c1", "one AI: its own bar")
        XCTAssertNil(Burst.bar([item("r1", quick: true)]))
    }

    func testTheShortcutSaysForHowLongAndWhat() {
        XCTAssertEqual(QuickAllowText.button(3_600), "Approve and allow for 1 hour")
        XCTAssertEqual(QuickAllowText.button(28_800), "Approve and allow for 8 hours")
        XCTAssertEqual(QuickAllowText.repeatHint(3), "You approved this 3 times in the last 24 hours.")
        XCTAssertEqual(
            QuickAllowText.caption(label: "Claude", what: "searching and reading me@gmail.com"),
            "Claude can then do the same without asking: searching and reading me@gmail.com. Revoke it any time in Grants."
        )
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
