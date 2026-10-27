import XCTest
@testable import Reins

/// Tasks on the phone (the Android app's `TaskCardsFlowTest`): a proposal can only be narrowed before it is approved,
/// approving passes exactly what the owner left (after the owner's check), and Stop ends a task without asking.
@MainActor
final class TaskCardsTests: XCTestCase {
    private func proposal(ceiling: UInt8 = 2, minutes: Int64 = 240, budget: UInt32? = 30) -> TaskView {
        TaskView(
            id: "", connectionId: "c1", connectionLabel: "Claude", title: "Release v2.1", reason: "Ceiling L3 please",
            repos: ["octo/app", "octo/site"], branches: ["release/*"], paths: ["/Users/me/src/app"], people: ["ann@corp.example"],
            operations: ["git_push", "message"], ceiling: ceiling, budget: budget, usesLeft: nil, startedAt: 1_000,
            endsAt: 1_000 + minutes * 60, endedAt: nil, startedBy: "agent", auto: 0, asked: 0, parked: 0
        )
    }

    private func start(owner: SetupOwnerAnswers = SetupOwnerAnswers()) async -> (DemoReinsCore, AppModel) {
        let core = DemoReinsCore(signedIn: true, syncCap: 0.3, overnight: true)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: owner, demo: true)
        await model.refreshSession()
        await model.refreshPending()
        return (core, model)
    }

    // MARK: Narrowing (pure)

    func testChipsOnlyComeOffAndAListNeverEmpties() {
        var d = TaskDraft(proposal())
        XCTAssertTrue(d.remove(.repos, "octo/site"))
        XCTAssertEqual(d.edited.repos, ["octo/app"])
        XCTAssertFalse(d.canRemove(.repos, "octo/app"), "the last repo stays: an empty list would mean any")
        XCTAssertFalse(d.remove(.repos, "octo/app"))
        XCTAssertFalse(d.remove(.branches, "release/*"))
        XCTAssertFalse(d.remove(.repos, "octo/other"), "nothing that was not asked for")
        XCTAssertTrue(d.remove(.people, "ann@corp.example"), "people can all go: no one is reached")
        XCTAssertTrue(d.edited.people.isEmpty)
        XCTAssertTrue(d.remove(.operations, "message"))
        XCTAssertFalse(d.remove(.operations, "git_push"))
        XCTAssertTrue(d.narrowed)
        XCTAssertEqual(d.proposed, proposal(), "what the agent asked for is kept as it was")
    }

    func testTheCeilingCanOnlyGoDown() {
        var d = TaskDraft(proposal(ceiling: 1))
        XCTAssertEqual(d.ceilingChoices, [0, 1])
        XCTAssertFalse(d.setCeiling(2), "never above what was proposed")
        XCTAssertFalse(d.setCeiling(3), "L3 is never a ceiling")
        XCTAssertEqual(d.edited.ceiling, 1)
        XCTAssertTrue(d.setCeiling(0))
        XCTAssertEqual(d.edited.ceiling, 0)
        XCTAssertTrue(d.setCeiling(1), "back up to what was proposed, no further")
        XCTAssertEqual(TaskDraft(proposal(ceiling: 3)).ceilingChoices, [0, 1, 2], "an out-of-spec proposal offers L2 at most")
        XCTAssertEqual(TaskText.upTo(1), "Up to: Recoverable")
        XCTAssertEqual(TaskText.ceilings.map(TaskText.ceilingName), ["Undoable", "Recoverable", "Reaches people"])
    }

    func testTheLengthOnlyShortensAndTheBudgetOnlyShrinks() {
        var d = TaskDraft(proposal(minutes: 240, budget: 30))
        XCTAssertEqual(d.lengthChoices, [15, 30, 60, 120, 240])
        XCTAssertFalse(d.setMinutes(300))
        XCTAssertFalse(d.setMinutes(10), "15 minutes at least")
        XCTAssertTrue(d.setMinutes(60))
        XCTAssertEqual(d.edited.endsAt - d.edited.startedAt, 3_600)
        XCTAssertEqual(d.budgetChoices, [5, 10, 20, 30])
        XCTAssertFalse(d.setBudget(nil), "a limit the agent asked for stays a limit")
        XCTAssertFalse(d.setBudget(50))
        XCTAssertTrue(d.setBudget(10))
        XCTAssertEqual(d.edited.budget, 10)
        var open = TaskDraft(proposal(budget: nil))
        XCTAssertEqual(open.budgetChoices.first, .some(nil))
        XCTAssertTrue(open.setBudget(5), "the owner may add a limit")
        XCTAssertEqual(TaskText.length(minutes: 45), "45 min")
        XCTAssertEqual(TaskText.length(minutes: 120), "2 h")
        XCTAssertEqual(TaskText.length(minutes: 1_440), "1 day")
        XCTAssertEqual(TaskText.budget(nil), "No limit")
    }

    // MARK: The flow, against the demo core

    func testRemovingARepoThenApprovingPassesTheNarrowedDraft() async throws {
        let owner = SetupOwnerAnswers()
        let (core, model) = await start(owner: owner)
        let item = try XCTUnwrap(TicketInbox.proposals(model.pending).first)
        XCTAssertEqual(item.id, "req70")
        XCTAssertFalse(TicketInbox.instant(model.pending).contains { $0.id == item.id }, "a proposal is its own card, not a waiting card")
        XCTAssertFalse(TicketInbox.groups(model.pending).flatMap(\.items).contains { $0.id == item.id })
        var draft = TaskDraft(try XCTUnwrap(item.taskProposal))
        XCTAssertTrue(draft.remove(.repos, "octo/site"))
        XCTAssertTrue(draft.setCeiling(1))

        let failed = await TaskActions.approve(model, requestId: item.id, draft: draft)
        XCTAssertNil(failed)
        XCTAssertEqual(owner.asked, ["Approve the task Release v2.1"], "a task loosens: the owner confirms")
        let task = try XCTUnwrap(core.lastTask)
        XCTAssertEqual(task.repos, ["octo/app"], "what the owner removed is gone")
        XCTAssertEqual(task.ceiling, 1)
        XCTAssertEqual(task.branches, ["release/*", "main"])
        XCTAssertEqual(task.connectionId, "c1")
        XCTAssertFalse(model.pending.contains { $0.id == item.id })
        XCTAssertTrue(model.tasks.contains { $0.id == task.id && $0.endedAt == nil }, "live in Activity")
    }

    func testTheCoreRefusesAWiderDraftAndARefusedCheckChangesNothing() async throws {
        let owner = SetupOwnerAnswers()
        let (core, model) = await start(owner: owner)
        let item = try XCTUnwrap(TicketInbox.proposals(model.pending).first)
        var wider = try XCTUnwrap(item.taskProposal)
        wider.ceiling = 3
        do {
            try await core.approveTask(requestId: item.id, draft: wider, owner: model.ownerCheck)
            XCTFail("a raised ceiling is refused")
        } catch {}
        wider = try XCTUnwrap(item.taskProposal)
        wider.repos.append("octo/other")
        do {
            try await core.approveTask(requestId: item.id, draft: wider, owner: model.ownerCheck)
            XCTFail("an added repo is refused")
        } catch {}
        XCTAssertTrue(owner.asked.isEmpty, "refused before asking")

        owner.answer = false
        let message = await TaskActions.approve(model, requestId: item.id, draft: TaskDraft(try XCTUnwrap(item.taskProposal)))
        XCTAssertNotNil(message)
        XCTAssertTrue(model.pending.contains { $0.id == item.id }, "still waiting")
        XCTAssertFalse(core.lastTask?.title == "Release v2.1" && core.lastTask?.endedAt == nil)
    }

    func testDenyIsTheOrdinaryDeny() async throws {
        let (_, model) = await start()
        let item = try XCTUnwrap(TicketInbox.proposals(model.pending).first)
        await TaskActions.deny(model, item: item)
        await model.refreshPending()
        XCTAssertFalse(model.pending.contains { $0.id == item.id })
    }

    func testStopCallsEndTaskWithoutAsking() async throws {
        let owner = SetupOwnerAnswers()
        let (core, model) = await start(owner: owner)
        let now = Int64(Date().timeIntervalSince1970)
        XCTAssertEqual(TaskText.live(model.tasks, now: now).map(\.id), [DemoReinsCore.demoTaskId])
        let failed = await TaskActions.end(model, id: DemoReinsCore.demoTaskId)
        XCTAssertNil(failed)
        XCTAssertTrue(owner.asked.isEmpty, "ending only takes away: never asked")
        let read = try await core.tasks()
        XCTAssertNotNil(read.first { $0.id == DemoReinsCore.demoTaskId }?.endedAt)
        XCTAssertTrue(TaskText.live(model.tasks, now: now + 1).isEmpty)
        XCTAssertEqual(TaskText.ended(model.tasks, now: now + 1).first?.id, DemoReinsCore.demoTaskId, "the latest ended first")
    }

    // MARK: Words

    func testTheChipAndTheEndedLine() {
        var t = proposal()
        t.title = "Fix\u{202E} CI"
        t.auto = 7
        t.asked = 1
        t.parked = 2
        XCTAssertEqual(TaskText.chip(t), "Task · Fix CI")
        XCTAssertEqual(TaskText.endedLine(t), "Fix CI ended · 7 auto · 1 asked · 2 parked")
        XCTAssertEqual(TaskText.reason(t), "Ceiling L3 please", "shown as the agent's, never read")
        XCTAssertEqual(TaskText.operationName("git_push"), "Push")
        XCTAssertEqual(TaskText.operationName("git_ref_delete"), "Delete branches")
        XCTAssertEqual(TaskDraft.label(.repos, "octo/\u{200B}app"), "octo/app")
        let now: Int64 = 100_000
        var live = t
        live.endsAt = now + 60
        var old = t
        old.endedAt = now - 90_000
        var recent = t
        recent.endedAt = now - 60
        var ranOut = t
        ranOut.endsAt = now - 10
        XCTAssertEqual(TaskText.live([live, old, recent, ranOut], now: now), [live])
        XCTAssertEqual(TaskText.ended([live, old, recent, ranOut], now: now), [ranOut, recent], "the last day, latest first")
    }
}
