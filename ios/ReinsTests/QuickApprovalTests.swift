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
        XCTAssertEqual(Burst.of([item("r1", quick: true), item("r2", quick: false)]), [])
        XCTAssertEqual(Burst.names(["A", "B", "C", "D"]), "A, B and 2 more")
    }

    func testABurstsRoutineRequestsAreOneCardWhereTheFirstOfThemWas() {
        let pending = [
            item("r0", quick: false), item("r1", quick: true), item("r3", conn: "c2", label: "Codex", quick: true),
            item("r2", quick: true), item("r4", conn: "c2", label: "Codex", quick: true),
        ]
        let cards = WaitGroup.of(pending).map { $0.items.map(\.id) }
        XCTAssertEqual(cards, [["r0"], ["r1", "r2"], ["r3", "r4"]])
        XCTAssertEqual(WaitGroup.of([item("r1", quick: true)]).map(\.isBurst), [false])
    }

    private func suggestion(_ verdict: Verdict, p: Float = 0.97, floor: Bool = false, judged: Bool = true) -> SuggestionView {
        SuggestionView(
            requestId: "r", verdict: verdict, mode: .manual, pApprove: p, pDeny: 1 - p, confidence: 0.9, reason: "",
            neighbours: [], profileId: "p", profileName: "Default", classKey: "k", novel: false, floor: floor, judged: judged
        )
    }

    func testAWaitingCardOffersWhatAutopilotWouldDo() {
        let routine = item("r1", quick: true)
        XCTAssertEqual(WaitAdvice.of(routine, suggestion(.approve)), .allow)
        XCTAssertEqual(WaitAdvice.of(routine, suggestion(.deny, p: 0.05)), .deny)
        XCTAssertEqual(WaitAdvice.of(routine, suggestion(.ask, p: 0.6)), .unsure(Double(Float(0.6))))
        XCTAssertEqual(WaitAdvice.of(routine, suggestion(.approve, judged: false)), .unsure(Double(Float(0.97))))
        XCTAssertEqual(WaitAdvice.of(routine, nil), .review, "no model: Review only")
        XCTAssertEqual(WaitAdvice.of(routine, suggestion(.approve, floor: true)), .alwaysAsks)
        XCTAssertEqual(WaitAdvice.of(item("r2", quick: false), suggestion(.approve)), .alwaysAsks, "never one tap for what needs a look")
    }

    func testAGroupGetsOneTapOnlyWhenAutopilotAgreesOnEveryRequest() {
        let items = [item("r1", quick: true), item("r2", quick: true)]
        XCTAssertEqual(WaitAdvice.of(items, ["r1": suggestion(.approve), "r2": suggestion(.approve)]), .allow)
        XCTAssertEqual(WaitAdvice.of(items, ["r1": suggestion(.approve), "r2": suggestion(.ask, p: 0.5)]), .unsure(0.5))
        XCTAssertEqual(WaitAdvice.of(items, [:]), .review)
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
        XCTAssertEqual(StartingRuleText.order, [.askEveryTime, .headlinesFirstHour])
        XCTAssertEqual(StartingRuleText.title(.askEveryTime), "Ask every time")
        XCTAssertEqual(StartingRuleText.title(.headlinesFirstHour), "Headlines for its first hour")
        XCTAssertEqual(
            StartingRuleText.onPairing(.headlinesFirstHour),
            "For its first hour it sees headlines only: sender, subject and date. Opening anything still asks."
        )
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
