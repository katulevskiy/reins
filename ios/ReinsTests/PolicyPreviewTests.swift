import XCTest
@testable import Reins

/// A change to Auto previewed on this phone's own history before it's made (Decisions v2, 02 §4), against the `-demo`
/// core.
@MainActor
final class PolicyPreviewTests: XCTestCase {
    func testPreviewLine() async throws {
        let owner = SetupOwnerAnswers()
        let core = DemoReinsCore(signedIn: true, syncCap: 0.3, autoSetupDone: false)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: owner, demo: true)
        await model.refreshSession()
        await model.refreshAutopilot()
        let setup = AutoSetupModel(app: model)
        await setup.load()
        setup.next()
        func cell(_ zone: ZoneChoice, _ level: Int) -> CellChoice? { setup.setup?.matrix.first { $0.zone == zone }?.cells[level] }

        // Stricter: changed at once, nothing previewed.
        await setup.tap(.mine, level: 0)
        await setup.tap(.mine, level: 0)
        XCTAssertEqual(cell(.mine, 0), .ask)
        XCTAssertNil(setup.loosen)
        XCTAssertTrue(core.previewedChanges.isEmpty)

        // Looser: the preview first, nothing changed and nobody asked yet.
        await setup.tap(.mine, level: 0)
        XCTAssertEqual(setup.loosen?.line, "Last 30 days: 0 Auto → Ask, 4 Ask → Auto")
        XCTAssertEqual(core.previewedChanges, [.setCell(zone: .mine, level: 0, cell: .auto)])
        XCTAssertEqual(cell(.mine, 0), .ask)
        XCTAssertTrue(owner.asked.isEmpty)

        // Cancel keeps it; Continue asks the owner, then changes it.
        setup.cancelLoosen()
        XCTAssertNil(setup.loosen)
        XCTAssertEqual(cell(.mine, 0), .ask)
        await setup.tap(.mine, level: 0)
        await setup.confirmLoosen()
        XCTAssertNil(setup.loosen)
        XCTAssertEqual(owner.asked, ["Let Auto do more in Mine"])
        XCTAssertEqual(cell(.mine, 0), .auto)

        // "Ask me next time" has its own line; no line reads as no history.
        let line = await PolicyPreview.line(core, .askMeNextTime(requestId: "entry-1"))
        XCTAssertEqual(line, "Last 30 days: 3 Auto → Ask, 0 Ask → Auto")
        XCTAssertEqual(PolicyPreview.shown(""), "No history yet")
        XCTAssertEqual(PolicyPreview.shown(line), line)
    }
}
