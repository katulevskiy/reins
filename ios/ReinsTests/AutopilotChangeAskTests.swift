import XCTest
@testable import Reins

/// The two changes of Autopilot a paired computer can only ask for (the core's `desktop_autopilot_mode` and
/// `desktop_autopilot_correct`): Reins' own sentence (the core words the question from the phone's state), held to
/// approve, asked every time, the computer only as who asks.
final class AutopilotChangeAskTests: XCTestCase {
    private func change(_ op: String, question: String, detail: String) -> ApprovalView {
        DemoData.view(
            "req60", conn: "c6", label: "M2A", kind: .write, action: "grant", service: "desktop", account: nil,
            createdAt: 1_000, waitUntil: 1_300, op: op, resources: [ResourceView(id: "autopilot:\(op)", label: "Autopilot", wider: false)],
            preview: [question], noStanding: true, opTitle: op == "autopilot_mode" ? "Change Autopilot's mode" : "Correct Autopilot",
            ask: AskView(question: question, detail: detail, topic: nil, destructive: true)
        )
    }

    private var mode: ApprovalView {
        change("autopilot_mode", question: "Turn on Auto and leave Lockdown?", detail: "Asked from M2A. It changes the mode of every AI on this phone.")
    }

    private var correct: ApprovalView {
        change("autopilot_correct", question: "Teach Autopilot to approve requests like this one?", detail: "Request: Push 3 commits to dkat/reins")
    }

    func testBothAreHeldToApproveAndAskedEveryTime() {
        for view in [mode, correct] {
            XCTAssertTrue(ApprovalWords.autopilotChange(view))
            XCTAssertTrue(ApprovalWords.held(view), "held to approve")
            XCTAssertEqual(ApprovalWords.approveLabel(view, ApprovalDraft()), "Approve", "the dock reads Hold to approve")
            XCTAssertTrue(ApprovalWords.rememberLocked(view), "never remembered: Every time")
            XCTAssertEqual(ApprovalWords.denyLabel(view), "Deny")
            XCTAssertEqual(ApprovalWords.dangerTags(view).map(\.id), ["ownerOnly"], "only the owner can, not \"can't be undone\"")
        }
    }

    func testTheSentenceIsTheCoresQuestionAndTheComputerStaysApart() {
        let line = AskFormat.waiting(mode, core: nil, computer: "M2A")
        XCTAssertEqual(line.text, "Turn on Auto and leave Lockdown?")
        XCTAssertNil(line.mono)
        XCTAssertFalse(line.text.contains("M2A"), "who asks is its own line, never in the sentence")
        XCTAssertEqual(AskFormat.waiting(correct, core: nil, computer: "M2A").text, "Teach Autopilot to approve requests like this one?")
        XCTAssertEqual(AskFormat.label(mode), "Autopilot · Mode")
        XCTAssertEqual(AskFormat.label(correct), "Autopilot · Correct")
    }

    func testAnOrdinaryDesktopQuestionIsNotOne() {
        var view = mode
        view.op = "ask"
        XCTAssertFalse(ApprovalWords.autopilotChange(view))
    }
}
