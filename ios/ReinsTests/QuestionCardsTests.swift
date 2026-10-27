import XCTest
@testable import Reins

/// "Who is octo?": Auto's questions read with what waits, answered or put away; only a zone looser than Outside needs
/// the owner.
@MainActor
final class QuestionCardsTests: XCTestCase {
    func testAnswerAndDismiss() async throws {
        let owner = SetupOwnerAnswers()
        let core = DemoReinsCore(signedIn: true, syncCap: 0.3, questions: true)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: owner, demo: true)
        await model.refreshSession()
        await model.refreshPending()
        XCTAssertEqual(model.autoQuestions.map(\.key), ["octo", "bob@acme.io", "ci.acme.io"], "newest first")

        // Work is looser than Outside: the owner says so.
        let failed = await model.answerAutoQuestion("github:octo", zone: .work)
        XCTAssertNil(failed)
        XCTAssertEqual(owner.asked, ["Treat octo as Work"])
        XCTAssertEqual(model.autoQuestions.map(\.key), ["bob@acme.io", "ci.acme.io"])
        let world = try await core.autoSetup().world
        XCTAssertEqual(world.first { $0.key == "octo" }?.zone, .work)

        // Outside, and putting one away, need nothing.
        owner.asked = []
        let outside = await model.answerAutoQuestion("computers:ci.acme.io", zone: .outside)
        XCTAssertNil(outside)
        let dismissed = await model.dismissAutoQuestion("email:bob@acme.io")
        XCTAssertNil(dismissed)
        XCTAssertTrue(owner.asked.isEmpty)
        XCTAssertTrue(model.autoQuestions.isEmpty)

        // Refused: the question stays.
        let again = DemoReinsCore(signedIn: true, syncCap: 0.3, questions: true)
        let refusing = SetupOwnerAnswers()
        refusing.answer = false
        let other = AppModel(core: again, feedback: NoFeedback.shared, authenticator: refusing, demo: true)
        await other.refreshSession()
        await other.refreshPending()
        let refused = await other.answerAutoQuestion("github:octo", zone: .mine)
        XCTAssertNotNil(refused)
        XCTAssertTrue(other.autoQuestions.contains { $0.id == "github:octo" })
    }

    func testQuestionTitle() {
        func q(_ kind: FactChoice, _ key: String, from: String = "Claude") -> AutoQuestion {
            AutoQuestion(id: key, kind: kind, key: key, service: DemoReinsCore.service(kind), connectionLabel: from,
                         requestId: nil, at: 0, proposed: .outside)
        }
        XCTAssertEqual(QuestionText.title(q(.owner, "octo")), "Who is octo on GitHub?")
        XCTAssertEqual(QuestionText.title(q(.person, "bob@acme.io")), "Who is bob@acme.io?")
        XCTAssertEqual(QuestionText.title(q(.host, "ci.acme.io")), "Who is ci.acme.io?")
        XCTAssertEqual(QuestionText.runs(q(.owner, "octo")).filter(\.target).map(\.text), ["octo"], "the thing is the target")
        // From outside: no hidden reordering, one short line.
        XCTAssertEqual(QuestionText.title(q(.person, "a\u{202E}b\nc")), "Who is ab c?")
        XCTAssertEqual(QuestionText.from(q(.owner, "octo")), "“Claude”")
        XCTAssertNil(QuestionText.from(q(.owner, "octo", from: " ")))
        // The matrix's line on an entry.
        let shadow = MatrixView(outcome: .ask, hard: false, zone: .work, level: 2, line: "Ask · Work · Reaches people", shadow: true)
        XCTAssertEqual(MatrixLook.text(shadow), "Would be: Ask · Work · Reaches people")
        var decided = shadow
        decided.shadow = false
        XCTAssertEqual(MatrixLook.text(decided), "Ask · Work · Reaches people")
    }
}
