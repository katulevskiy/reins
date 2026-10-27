import XCTest
@testable import Reins

/// Auto's setup (01 §9.1) against the `-demo` core: picking Auto before it was done guides through it, loosening is the
/// owner's, and its end turns Auto on.
@MainActor
final class AutoSetupTests: XCTestCase {
    private var owner: SetupOwnerAnswers!
    private var core: DemoReinsCore!
    private var model: AppModel!

    private func start(done: Bool = false) async {
        owner = SetupOwnerAnswers()
        core = DemoReinsCore(signedIn: true, syncCap: 0.3, autoSetupDone: done)
        model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: owner, demo: true)
        await model.refreshSession()
        await model.refreshAutopilot()
    }

    func testTurningOnAutoGuidesFirst() async throws {
        await start()
        let ap = AutopilotModel(app: model)
        await ap.setMode(.auto)
        XCTAssertTrue(model.autoSetupOpen, "the setup opens instead")
        XCTAssertEqual(model.autopilot?.mode, .manual, "and Auto is not on yet")
        let afterPick = try await core.autoSetup()
        XCTAssertFalse(afterPick.done)

        // One AI's own mode is not the setup's business.
        await ap.setMode(.auto, connectionId: "c2")
        XCTAssertEqual(ap.modeOf("c2"), .auto)

        // Closing leaves everything as it was.
        let setup = AutoSetupModel(app: model)
        await setup.load()
        XCTAssertEqual(setup.step, .world)
        XCTAssertEqual(setup.sections.map(\.service), ["github", "email", "messages", "computers"])
        setup.close()
        XCTAssertFalse(model.autoSetupOpen)

        // Once it was done, Auto is a plain pick again.
        await start(done: true)
        let again = AutopilotModel(app: model)
        await again.setMode(.auto)
        XCTAssertFalse(model.autoSetupOpen)
        XCTAssertEqual(model.autopilot?.mode, .auto)
        XCTAssertTrue(owner.asked.isEmpty)
    }

    func testLooseningAsksTheOwner() async throws {
        await start()
        let setup = AutoSetupModel(app: model)
        await setup.load()
        let acme = try XCTUnwrap(setup.setup?.world.first { $0.key == "acme" })
        XCTAssertFalse(acme.confirmed, "a proposal")

        // Toward Mine: the owner first; refused, nothing changes.
        owner.answer = false
        await setup.setZone(acme, .mine)
        XCTAssertEqual(owner.asked, ["Treat acme as Mine"])
        XCTAssertNotNil(setup.error)
        XCTAssertEqual(setup.setup?.world.first { $0.key == "acme" }?.zone, .work)
        owner.answer = true
        await setup.setZone(acme, .mine)
        XCTAssertEqual(setup.setup?.world.first { $0.key == "acme" }?.zone, .mine)
        XCTAssertEqual(setup.setup?.world.first { $0.key == "acme" }?.confirmed, true)
        XCTAssertNil(setup.error)

        // Toward Outside needs nothing.
        owner.asked = []
        let dkat = try XCTUnwrap(setup.setup?.world.first { $0.key == "dkat" })
        await setup.setZone(dkat, .outside)
        XCTAssertTrue(owner.asked.isEmpty)
        XCTAssertEqual(setup.setup?.world.first { $0.key == "dkat" }?.zone, .outside)

        // The matrix: Auto → Ask first → Ask is stricter, one tap each; Ask → Auto asks the owner.
        setup.next()
        XCTAssertEqual(setup.step, .matrix)
        func cell(_ zone: ZoneChoice, _ level: Int) -> CellChoice? { setup.setup?.matrix.first { $0.zone == zone }?.cells[level] }
        XCTAssertEqual(cell(.mine, 0), .auto)
        await setup.cycle(.mine, level: 0)
        XCTAssertEqual(cell(.mine, 0), .askFirst)
        await setup.cycle(.mine, level: 0)
        XCTAssertEqual(cell(.mine, 0), .ask)
        XCTAssertTrue(owner.asked.isEmpty, "tightening is one tap")
        await setup.cycle(.mine, level: 0)
        XCTAssertEqual(owner.asked, ["Let Auto do more in Mine"])
        XCTAssertEqual(cell(.mine, 0), .auto)

        // "Can't be undone" always asks: a tap changes nothing.
        owner.asked = []
        await setup.cycle(.mine, level: 3)
        XCTAssertEqual(cell(.mine, 3), .ask)
        XCTAssertTrue(owner.asked.isEmpty)
        XCTAssertFalse(AutoSetupText.editable(level: 3))
        XCTAssertEqual(AutoSetupText.nextCell(.ask), .auto)
    }

    func testFinishTurnsAutoOn() async throws {
        await start()
        let setup = AutoSetupModel(app: model)
        model.autoSetupOpen = true
        await setup.load()
        setup.next()
        setup.next()
        XCTAssertEqual(setup.step, .turnOn)

        // Refused: still Manual, still to do.
        owner.answer = false
        await setup.finish()
        XCTAssertEqual(owner.asked, ["Turn on Auto"])
        XCTAssertFalse(setup.finished)
        XCTAssertTrue(model.autoSetupOpen)
        XCTAssertEqual(model.autopilot?.mode, .manual)

        owner.answer = true
        await setup.finish()
        XCTAssertTrue(setup.finished)
        XCTAssertFalse(model.autoSetupOpen, "back to where Auto was picked")
        XCTAssertEqual(model.autopilot?.mode, .auto)
        let after = try await core.autoSetup()
        XCTAssertTrue(after.done)
        XCTAssertTrue(after.world.allSatisfy(\.confirmed), "every proposal is kept as shown")
    }
}

/// Answers the owner's check as told and remembers what it was asked.
final class SetupOwnerAnswers: Authenticating {
    var answer = true
    var asked: [String] = []

    func confirm(_ reason: String) async -> Bool {
        asked.append(reason)
        return answer
    }
}
