import UserNotifications
import XCTest
@testable import Reins

/// Overnight's morning digest (the Android app's `OvernightDigestTest`): one local notification at the morning time,
/// replaced under one identifier, taken away when nothing waits; what Overnight parked never notifies on arrival and is
/// not reminded of before the morning.
@MainActor
final class OvernightDigestTests: XCTestCase {
    private let now: Int64 = 1_700_000_000

    private func digest(at: Int64?, total: UInt32 = 3) -> OvernightDigest {
        OvernightDigest(
            at: at, total: total, title: "\(total) waiting", body: "1 from Claude Code (Fix CI), 2 from ChatGPT",
            groups: [DigestGroup(connectionId: "c1", connectionLabel: "Claude Code", taskId: "t1", taskTitle: "Fix CI", requestIds: ["r1"], batchable: 1)]
        )
    }

    private func item(_ id: String, parkedUntil: Int64?, ticket: Bool = true, deferredAt: Int64? = nil, deadline: Int64? = nil) -> PendingItem {
        let deferred = deferredAt ?? now - 600
        let due = deadline ?? now + 20 * 3_600
        return PendingItem(
            kind: .request, id: id, title: "", subtitle: "", createdAt: deferred, connectionId: "c1", connectionLabel: "Claude",
            action: "send", count: 1, service: "gmail", account: nil, waitUntil: due, op: "", opTitle: "", suggestion: nil,
            quick: true,
            ticket: ticket ? TicketView(ticket: "rt_\(id)", state: "deferred", deferredAt: deferred, deadline: due, question: nil, agentReply: nil, stale: nil) : nil,
            parkedUntil: parkedUntil, batchable: true
        )
    }

    private func profile() -> WorkProfiles.Active {
        WorkProfiles.Active(id: "test-\(UUID().uuidString)", label: "Test", managed: false, threadPrefix: "", keyAlias: "")
    }

    // MARK: The plan

    func testADigestWithAMorningIsScheduledAtIt() {
        XCTAssertEqual(
            OvernightDigestNotice.plan(digest(at: now + 6 * 3_600), now: now),
            .schedule(at: now + 6 * 3_600, title: "3 waiting", body: "1 from Claude Code (Fix CI), 2 from ChatGPT")
        )
    }

    func testNothingWaitingRemovesItAndAMorningThatCameKeepsWhatWasShown() {
        XCTAssertEqual(OvernightDigestNotice.plan(nil, now: now), .cancel(keepDelivered: false), "nothing waits: nothing to tell")
        XCTAssertEqual(OvernightDigestNotice.plan(digest(at: nil), now: now), .cancel(keepDelivered: true), "the morning came: shown when opened")
        XCTAssertEqual(OvernightDigestNotice.plan(digest(at: now - 1), now: now), .cancel(keepDelivered: true))
        XCTAssertEqual(OvernightDigestNotice.plan(digest(at: now + 60, total: 0), now: now), .cancel(keepDelivered: false))
    }

    func testTheAgentsWordsInTheDigestAreCleaned() {
        var d = digest(at: now + 60)
        d.body = "1 from Cl\u{202E}aude"
        guard case let .schedule(_, _, body) = OvernightDigestNotice.plan(d, now: now) else { return XCTFail("scheduled") }
        XCTAssertFalse(body.unicodeScalars.contains("\u{202E}"), "a bidi override shows, never acts")
    }

    // MARK: Scheduling

    func testOneNotificationAtTheMorningReplacedUnderOneIdentifier() throws {
        let fake = FakeDigestScheduler()
        let p = profile()
        let morning = now + 6 * 3_600
        let first = OvernightDigestNotice.plan(digest(at: morning), now: now)
        var last = OvernightDigestNotice.apply(first, last: nil, scheduler: fake, settings: FeedbackSettings(), profile: p)
        XCTAssertEqual(fake.added.count, 1)
        let request = try XCTUnwrap(fake.added.first)
        XCTAssertEqual(request.identifier, OvernightDigestNotice.identifier(p))
        XCTAssertEqual(request.content.title, "3 waiting")
        XCTAssertEqual(request.content.body, "1 from Claude Code (Fix CI), 2 from ChatGPT")
        XCTAssertEqual(request.content.categoryIdentifier, NotificationCategory.status.rawValue, "not an item: the sweep leaves it")
        let trigger = try XCTUnwrap(request.trigger as? UNCalendarNotificationTrigger)
        XCTAssertFalse(trigger.repeats)
        let fires = try XCTUnwrap(Calendar.current.date(from: trigger.dateComponents))
        XCTAssertEqual(Int64(fires.timeIntervalSince1970), morning, "at the digest's `at`, to the second")

        // The same plan again (another read of the list): nothing is handed to the system twice.
        last = OvernightDigestNotice.apply(first, last: last, scheduler: fake, settings: FeedbackSettings(), profile: p)
        XCTAssertEqual(fake.added.count, 1)

        // Something else parked: the one notification is replaced, under the same identifier.
        var later = digest(at: morning, total: 4)
        later.title = "4 waiting"
        last = OvernightDigestNotice.apply(OvernightDigestNotice.plan(later, now: now), last: last, scheduler: fake, settings: FeedbackSettings(), profile: p)
        XCTAssertEqual(fake.added.map(\.identifier), [OvernightDigestNotice.identifier(p), OvernightDigestNotice.identifier(p)])
        XCTAssertEqual(fake.added.last?.content.title, "4 waiting")
        XCTAssertTrue(fake.removedPending.contains(OvernightDigestNotice.identifier(p)), "the earlier one gives way")

        // Nothing waits any more: it goes, delivered or not.
        _ = OvernightDigestNotice.apply(OvernightDigestNotice.plan(nil, now: now), last: last, scheduler: fake, settings: FeedbackSettings(), profile: p)
        XCTAssertEqual(fake.removedDelivered, [OvernightDigestNotice.identifier(p)])
        XCTAssertEqual(fake.added.count, 2)
    }

    func testAMorningThatCameKeepsTheShownDigestButSchedulesNothing() {
        let fake = FakeDigestScheduler()
        let p = profile()
        _ = OvernightDigestNotice.apply(.cancel(keepDelivered: true), last: nil, scheduler: fake, settings: FeedbackSettings(), profile: p)
        XCTAssertEqual(fake.removedPending, [OvernightDigestNotice.identifier(p)])
        XCTAssertTrue(fake.removedDelivered.isEmpty)
        XCTAssertTrue(fake.added.isEmpty)
    }

    func testTheDigestIsReadFromTheCoreAfterTheListChanges() async throws {
        let core = DemoReinsCore(signedIn: true, syncCap: 0.3, overnight: true)
        let fake = FakeDigestScheduler()
        let p = profile()
        await OvernightDigestNotice.sync(core, scheduler: fake, profile: p, now: { self.now })
        let digest = try await core.overnightDigest()
        XCTAssertEqual(digest?.total, 4)
        XCTAssertEqual(digest?.title, "4 waiting")
        XCTAssertEqual(fake.added.count, 1, "one notification for everything parked")
        XCTAssertEqual(fake.added.first?.content.title, "4 waiting")

        // Everything parked is answered: the digest goes.
        for id in digest?.groups.flatMap(\.requestIds) ?? [] { try await core.deny(requestId: id) }
        let after = try await core.overnightDigest()
        XCTAssertNil(after)
        await OvernightDigestNotice.sync(core, scheduler: fake, profile: p, now: { self.now })
        XCTAssertEqual(fake.removedDelivered, [OvernightDigestNotice.identifier(p)])
    }

    func testTheDemoDigestGroupsByAgentAndTask() throws {
        let items = [
            item("a", parkedUntil: now + 100),
            {
                var i = item("b", parkedUntil: now + 50)
                i.taskId = "t1"
                i.taskTitle = "Fix CI"
                return i
            }(),
            item("c", parkedUntil: nil),
        ]
        let d = try XCTUnwrap(DemoReinsCore.digest(items, now: now))
        XCTAssertEqual(d.total, 2, "only what Overnight parked")
        XCTAssertEqual(d.at, now + 50)
        XCTAssertEqual(d.groups.map(\.taskId), [nil, "t1"])
        XCTAssertEqual(d.body, "1 from Claude, 1 from Claude (Fix CI)")
        XCTAssertNil(DemoReinsCore.digest([item("c", parkedUntil: nil)], now: now))
    }

    // MARK: A parked item is quiet

    func testAParkedItemDoesNotNotify() {
        let parked = item("p", parkedUntil: now + 6 * 3_600)
        XCTAssertTrue(OvernightDigestNotice.isParked(parked, now: now))
        XCTAssertFalse(OvernightDigestNotice.isParked(parked, now: now + 6 * 3_600), "the morning came")
        XCTAssertFalse(OvernightDigestNotice.isParked(item("q", parkedUntil: nil), now: now))
        // What the notification extension posts for it (it cannot drop a push): no sound, no lit screen.
        let c = NotificationContent.pending(parked, settings: FeedbackSettings(), now: now)
        XCTAssertEqual(c.interruptionLevel, .passive)
        XCTAssertNil(c.sound)
        XCTAssertEqual(c.categoryIdentifier, NotificationCategory.waiting.rawValue, "never Approve from the notification")
        // Not even a ticket yet: still under "Waiting for you", never popped up.
        let noTicket = item("n", parkedUntil: now + 3_600, ticket: false)
        XCTAssertEqual(NotificationCategory.of(noTicket), .waiting)
        XCTAssertNil(AskGate.next([noTicket, parked], presented: []))
        XCTAssertEqual(AskGate.next([noTicket, item("i", parkedUntil: nil, ticket: false)], presented: [])?.id, "i")
    }

    func testAParkedTicketIsRemindedOfOnlyAfterTheMorning() {
        // Parked at 01:00 for a morning at 07:00, expiring 12 h after it.
        let morning = now + 6 * 3_600
        let parked = item("p", parkedUntil: morning, deferredAt: now, deadline: morning + 12 * 3_600)
        let reminders = TicketReminders.plan([parked], now: now)
        XCTAssertEqual(reminders.map(\.percent), [50, 90])
        XCTAssertTrue(reminders.allSatisfy { $0.fireAt > morning }, "nothing before the morning")
        XCTAssertEqual(reminders.map(\.fireAt), [morning + 6 * 3_600, morning + 12 * 3_600 * 9 / 10])
        // Once the morning came, the same ticket keeps those reminders.
        XCTAssertEqual(TicketReminders.plan([parked], now: morning + 1).map(\.fireAt), reminders.map(\.fireAt))
    }
}

private final class FakeDigestScheduler: DigestScheduling {
    var added: [UNNotificationRequest] = []
    var removedPending: [String] = []
    var removedDelivered: [String] = []

    func add(_ request: UNNotificationRequest) { added.append(request) }
    func removePending(_ identifiers: [String]) { removedPending += identifiers }
    func removeDelivered(_ identifiers: [String]) { removedDelivered += identifiers }
}
