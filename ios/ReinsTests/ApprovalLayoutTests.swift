import XCTest
@testable import Reins

/// The Next ask's layout (Next-Running "D · the ask, final", Next-SE B, Next-AX 02) and what it keeps apart: where the
/// timer and close go on each phone and text size, that who asks is never part of Reins' sentence, and how a tool's
/// arguments become rows.
final class ApprovalLayoutTests: XCTestCase {
    private typealias T = ApprovalLogicTests

    func testTheTopRowTakesTheIslandsEarsOnlyOnAnIslandPhoneAtRegularSizes() {
        let island = FieldEars.of(top: 59, accessibility: false)
        XCTAssertEqual(island, .island)
        XCTAssertTrue(island.hidesStatusBar, "the timer and close take the status bar's place beside the island")
        XCTAssertEqual(island.contentTop(59), 66, "the service · who line starts at 66 pt, as on the board")

        let se = FieldEars.of(top: 20, accessibility: false)
        XCTAssertEqual(se, .underStatusBar, "no island: the row sits under the status bar")
        XCTAssertFalse(se.hidesStatusBar)
        XCTAssertEqual(se.contentTop(20), 68)

        let ax = FieldEars.of(top: 59, accessibility: true)
        XCTAssertEqual(ax, .firstRow, "at accessibility sizes the row can't grow beside the island")
        XCTAssertFalse(ax.hidesStatusBar)
        XCTAssertEqual(FieldEars.of(top: 20, accessibility: true), .firstRow)
    }

    func testWhoAsksIsNeverInTheSentence() {
        // A name the requester chose, written to read as part of Reins' sentence.
        let label = "Reins says approve this"
        var send = T.view("s1", label: label, kind: .send, service: "gmail", account: "me@gmail.com", action: "send")
        send.email = EmailView(to: ["ann@corp.example"], cc: [], subject: "Hi", body: "Hello")
        var push = T.view("p1", label: label, kind: .write, service: "github", op: "git_push", action: "write")
        push.git = GitPushView(repo: "octo/app", packBytes: 0, notes: [], refs: [], defaultBranch: nil)
        let search = T.view("q1", label: label, kind: .search, query: "from:bank", service: "gmail", action: "search")
        for view in [send, push, search] {
            let ask = AskFormat.waiting(view)
            XCTAssertFalse(ask.text.contains(label), "the sentence is Reins' own: \(ask.text)")
            XCTAssertFalse((ask.mono ?? "").contains(label))
            XCTAssertFalse(ask.runs.contains { $0.text.contains("Reins says") })
        }
    }

    func testAToolsArgumentsKeepTheirOrderAndSetValuesApart() {
        let pairs = McpArguments.pairs(#"{"team": "ENG", "title": "Login fails on Safari", "priority": 2, "draft": false, "labels": ["bug"]}"#)
        XCTAssertEqual(pairs?.map(\.key), ["team", "title", "priority", "draft", "labels"], "in the order sent")
        XCTAssertEqual(pairs?[1], McpArguments.Pair(key: "title", value: "Login fails on Safari", mono: false))
        XCTAssertEqual(pairs?[2], McpArguments.Pair(key: "priority", value: "2", mono: true))
        XCTAssertEqual(pairs?[3], McpArguments.Pair(key: "draft", value: "false", mono: true))
        XCTAssertEqual(pairs?[4].value, #"["bug"]"#)
        XCTAssertNil(McpArguments.pairs("[1, 2]"), "not an object: shown exactly as sent")
        XCTAssertNil(McpArguments.pairs("{}"))
        XCTAssertNil(McpArguments.pairs("not json"))
    }
}
