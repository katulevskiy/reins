import XCTest
@testable import Reins

/// "Waiting for you" (the Android app's `TicketLogicTest`): tickets apart from the stack, the soonest due first, by
/// who asks, and batch answers that never approve what is not routine.
final class TicketInboxTests: XCTestCase {
    private let now: Int64 = 1_700_000_000

    private func item(
        _ id: String, conn: String = "c1", label: String = "Claude", action: String = "send", service: String = "gmail", op: String = "",
        quick: Bool = true, deadline: Int64? = nil, created: Int64 = 0, state: String = "deferred", stale: String? = nil,
        question: String? = nil, reply: String? = nil, canDefer: Bool = false
    ) -> PendingItem {
        PendingItem(
            kind: .request, id: id, title: "", subtitle: "", createdAt: created, connectionId: conn, connectionLabel: label,
            action: action, count: 1, service: service, account: nil, waitUntil: deadline, op: op, opTitle: "", suggestion: nil,
            headline: "", quick: quick, label: nil,
            ticket: deadline.map {
                TicketView(ticket: "rt_\(id)", state: state, deferredAt: now - 3_600, deadline: $0, question: question, agentReply: reply, stale: stale)
            },
            canDefer: canDefer
        )
    }

    func testTicketsAreSortedByDeadlineAndInstantRequestsStayInTheStack() {
        let pending = [
            item("instant", deadline: nil, canDefer: true),
            item("late", deadline: now + 7_200),
            item("soon", deadline: now + 600),
            item("tie-new", deadline: now + 3_600, created: 20),
            item("tie-old", deadline: now + 3_600, created: 10),
        ]
        XCTAssertEqual(TicketInbox.tickets(pending).map(\.id), ["soon", "tie-old", "tie-new", "late"])
        XCTAssertEqual(TicketInbox.instant(pending).map(\.id), ["instant"], "only what is not a ticket is a card")
        XCTAssertFalse(TicketInbox.groups(pending).flatMap(\.items).contains { $0.id == "instant" }, "the inbox has no Instant request")
    }

    func testTicketsAreGroupedByWhoAsksTheSoonestGroupFirst() {
        let pending = [
            item("a1", conn: "c1", label: "Claude", deadline: now + 9_000),
            item("b1", conn: "c2", label: "Codex", deadline: now + 600),
            item("a2", conn: "c1", label: "Claude", deadline: now + 1_200),
            // The same name on another connection is another group: batches never mix two AIs.
            item("x1", conn: "c3", label: "Claude", deadline: now + 2_000),
        ]
        let groups = TicketInbox.groups(pending)
        XCTAssertEqual(groups.map(\.connectionId), ["c2", "c1", "c3"])
        XCTAssertEqual(groups.map(\.label), ["Codex", "Claude", "Claude"])
        XCTAssertEqual(groups[1].items.map(\.id), ["a2", "a1"], "inside a group, the soonest due first")
    }

    func testBatchesAreOneKindFromOneAiAndAllowOnlyWhatIsRoutine() {
        let pending = [
            item("s1", deadline: now + 600),
            item("s2", quick: false, deadline: now + 700),
            item("s3", deadline: now + 800),
            item("s4", deadline: now + 900, state: "stale", stale: "main moved"),
            item("s5", deadline: now + 950, state: "needs_info", question: "Why so many?"),
            item("r1", action: "read", deadline: now + 1_000),
            item("p1", action: "write", service: "github", op: "push", deadline: now + 1_100),
            item("p2", action: "write", service: "github", op: "merge", deadline: now + 1_200),
        ]
        let batches = TicketInbox.groups(pending)[0].batches
        XCTAssertEqual(batches.count, 1, "a single read, and two writes of different operations, make no batch")
        let send = batches[0]
        XCTAssertEqual(send.all, ["s1", "s2", "s3", "s4", "s5"], "deny may include anything")
        XCTAssertEqual(send.allow, ["s1", "s3"], "never what is asked every time, went stale or waits for the agent's answer")
        XCTAssertEqual(TicketInbox.allowTitle(send), "Allow 2")
        XCTAssertEqual(TicketInbox.denyTitle(send), "Deny all 5")
        let routine = TicketInbox.batches([item("a", deadline: now + 1), item("b", deadline: now + 2), item("c", deadline: now + 3)])
        XCTAssertEqual(routine.map(TicketInbox.allowTitle), ["Allow all 3"])
    }

    func testBatchApprovalNeedsARoutineRequest() {
        XCTAssertTrue(TicketInbox.allowsBatchApproval(item("a", deadline: now + 60)))
        XCTAssertFalse(TicketInbox.allowsBatchApproval(item("b", quick: false, deadline: now + 60)))
        XCTAssertFalse(TicketInbox.allowsBatchApproval(item("c", deadline: now + 60, state: "stale")))
        XCTAssertFalse(TicketInbox.allowsBatchApproval(item("d", deadline: now + 60, stale: "main moved from 1a2b3c4 to 5d6e7f8")))
        XCTAssertFalse(TicketInbox.allowsBatchApproval(item("e", deadline: now + 60, state: "needs_info", question: "Why?")), "asked Why?, not answered")
    }

    func testDueText() {
        XCTAssertEqual(TicketText.due(deadline: now + 3 * 3_600 + 59 * 60, now: now), "Due in 3 h", "rounded down")
        XCTAssertEqual(TicketText.due(deadline: now + 45 * 60, now: now), "Due in 45 min")
        XCTAssertEqual(TicketText.due(deadline: now + 20, now: now), "Due in 1 min")
        XCTAssertEqual(TicketText.due(deadline: now + 47 * 3_600, now: now), "Due in 47 h")
        XCTAssertEqual(TicketText.due(deadline: now + 3 * 86_400, now: now), "Due in 3 d")
        XCTAssertEqual(TicketText.due(deadline: now, now: now), "Overdue")
        XCTAssertEqual(TicketText.due(deadline: now - 600, now: now), "Overdue")
        XCTAssertEqual(TicketInbox.due(item("t", deadline: now + 7_200), now: now), "Due in 2 h")
        XCTAssertNil(TicketInbox.due(item("i"), now: now))
        XCTAssertTrue(TicketText.urgent(deadline: now + 3_599, now: now))
        XCTAssertFalse(TicketText.urgent(deadline: now + 3_600, now: now))
    }

    func testStateWordsAndTheAgentsAnswerIsLabelledAndCleaned() {
        func ticket(_ i: PendingItem) -> TicketView { i.ticket! }
        XCTAssertNil(TicketText.state(ticket(item("a", deadline: now + 1))))
        XCTAssertEqual(TicketText.state(ticket(item("b", deadline: now + 1, state: "needs_info", question: "Why?"))), "Why?")
        XCTAssertEqual(TicketText.state(ticket(item("c", deadline: now + 1, question: "Why?", reply: "Because"))), "Answered")
        XCTAssertEqual(TicketText.state(ticket(item("d", deadline: now + 1, state: "stale", stale: "main moved"))), "Stale")
        let answered = ticket(item("e", deadline: now + 1, question: "Why?", reply: "  For the release\u{202E}  "))
        XCTAssertEqual(TicketText.agentSaysLabel, "Agent says")
        XCTAssertNotNil(TicketText.agentSays(answered))
        XCTAssertFalse(TicketText.agentSays(answered)!.contains("\u{202E}"), "the agent's words are cleaned")
        XCTAssertNil(TicketText.agentSays(ticket(item("f", deadline: now + 1, reply: "   "))))
    }

    func testLaterNotesAndQuestions() {
        XCTAssertTrue(TicketInbox.canLater(item("i", canDefer: true)))
        XCTAssertFalse(TicketInbox.canLater(item("j", canDefer: false)), "the server cannot park it")
        XCTAssertFalse(TicketInbox.canLater(item("t", deadline: now + 60, canDefer: true)), "already a ticket")
        XCTAssertNil(TicketInbox.note("  \n "), "an empty note is a plain denial")
        XCTAssertNil(TicketInbox.note(nil))
        XCTAssertEqual(TicketInbox.note("  Use the staging bucket "), "Use the staging bucket")
        XCTAssertEqual(TicketInbox.note(String(repeating: "a", count: 900))?.count, TicketInbox.maxText)
        XCTAssertNil(TicketInbox.question(""))
    }

    func testTicketsNeverPopUpByThemselves() {
        let pending = [item("t", deadline: now + 60), item("i")]
        XCTAssertEqual(AskGate.next(pending, presented: [])?.id, "i")
        XCTAssertNil(AskGate.next([item("t", deadline: now + 60)], presented: []))
    }

    // MARK: Overnight's parks and tasks (Decisions v2 P5)

    private func parked(
        _ id: String, conn: String = "c1", label: String = "Claude", task: String? = nil, title: String? = nil, batchable: Bool = true,
        deadline: Int64? = nil, until: Int64? = nil, created: Int64 = 0
    ) -> PendingItem {
        var i = item(id, conn: conn, label: label, deadline: deadline, created: created)
        i.parkedUntil = until ?? now + 6 * 3_600
        i.taskId = task
        i.taskTitle = title
        i.batchable = batchable
        return i
    }

    func testTheInboxGroupsByAgentAndTask() {
        let pending = [
            parked("a1", task: "t1", title: "Fix CI", deadline: now + 9_000),
            parked("a2", deadline: now + 8_000),
            parked("a3", task: "t1", title: "Fix CI", batchable: false, deadline: now + 7_000),
            parked("b1", conn: "c2", label: "ChatGPT", task: "t1", title: "Fix CI", deadline: now + 10_000),
        ]
        let groups = TicketInbox.groups(pending)
        XCTAssertEqual(groups.map(\.id), ["c1|t1", "c1|", "c2|t1"], "one AI under one task each; the same task id on another AI never mixes")
        XCTAssertEqual(groups[0].items.map(\.id), ["a3", "a1"])
        XCTAssertEqual(groups[0].taskTitle, "Fix CI")
        XCTAssertEqual(TicketInbox.taskTitle(groups[0]), "Fix CI")
        XCTAssertNil(TicketInbox.taskTitle(groups[1]))
        XCTAssertEqual(groups[0].approvable, ["a1"], "what is hard stays one by one")
        XCTAssertEqual(TicketInbox.approveAllTitle(groups[0]), "Approve all 1")
        XCTAssertEqual(groups[1].approvable, ["a2"])
    }

    func testOnlyParkedSoftItemsAreApprovedTogether() {
        XCTAssertTrue(TicketInbox.approvableTogether(parked("a")))
        XCTAssertFalse(TicketInbox.approvableTogether(parked("b", batchable: false)), "hard or L3")
        var notParked = item("c", deadline: now + 60)
        notParked.batchable = true
        XCTAssertFalse(TicketInbox.approvableTogether(notParked), "Approve all covers what Overnight parked")
        XCTAssertTrue(TicketInbox.groups([notParked])[0].approvable.isEmpty)
    }

    func testParkedItemsWaitInTheInboxEvenBeforeTheyHaveATicket() {
        let early = parked("p", deadline: nil, until: now + 3_600)
        XCTAssertNil(early.ticket)
        XCTAssertTrue(TicketInbox.inInbox(early))
        XCTAssertTrue(TicketInbox.instant([early]).isEmpty, "not a waiting card")
        XCTAssertEqual(TicketInbox.groups([early, parked("q", deadline: now + 600)]).flatMap { $0.items.map(\.id) }, ["q", "p"],
                       "sorted by its ticket's deadline, else by the morning")
        XCTAssertNil(TicketInbox.due(early, now: now))
    }

    func testATaskProposalIsItsOwnCard() {
        var proposal = item("tp")
        proposal.taskProposal = TaskView(
            id: "", connectionId: "c1", connectionLabel: "Claude", title: "Release", reason: nil, repos: ["octo/app"], branches: [],
            paths: [], people: [], operations: [], ceiling: 1, budget: nil, usesLeft: nil, startedAt: 0, endsAt: 3_600, endedAt: nil,
            startedBy: "agent", auto: 0, asked: 0, parked: 0
        )
        XCTAssertEqual(TicketInbox.proposals([proposal, item("i")]).map(\.id), ["tp"])
        XCTAssertEqual(TicketInbox.instant([proposal, item("i")]).map(\.id), ["i"])
        proposal.parkedUntil = now + 60
        XCTAssertTrue(TicketInbox.groups([proposal]).isEmpty, "even parked, it is decided on its card")
        XCTAssertNil(AskGate.next([proposal], presented: []), "never pops up by itself")
    }

    @MainActor
    func testApproveAllApprovesTheGroupsRoutineParksAfterOneCheck() async throws {
        let owner = SetupOwnerAnswers()
        let core = DemoReinsCore(signedIn: true, syncCap: 0.3, overnight: true)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: owner, demo: true)
        await model.refreshSession()
        await model.refreshPending()
        let groups = TicketInbox.groups(model.pending)
        let cursor = try XCTUnwrap(groups.first { $0.connectionId == "c5" && $0.taskId == DemoReinsCore.demoTaskId })
        XCTAssertEqual(Set(cursor.items.map(\.id)), ["req10", "req11"])
        XCTAssertEqual(cursor.approvable, ["req10"])

        owner.answer = false
        let refused = await InboxActions.approveAll(model, group: cursor)
        XCTAssertNil(refused)
        XCTAssertTrue(model.pending.contains { $0.id == "req10" }, "nothing without the owner's check")

        owner.answer = true
        let approved = await InboxActions.approveAll(model, group: cursor)
        XCTAssertEqual(approved, 1)
        XCTAssertEqual(owner.asked.last, "Approve the request from Cursor")
        XCTAssertFalse(model.pending.contains { $0.id == "req10" })
        XCTAssertTrue(model.pending.contains { $0.id == "req11" }, "the hard one stays, one by one")
    }
}
