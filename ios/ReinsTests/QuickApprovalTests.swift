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
        XCTAssertEqual(bursts[0].title, "Claude asked 3 times")
        XCTAssertEqual(bursts[0].heldNote, "1 of them needs a closer look and stays in the list.")
        XCTAssertEqual(Burst.of([item("r1", quick: true), item("r2", quick: false)]), [])
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
}
