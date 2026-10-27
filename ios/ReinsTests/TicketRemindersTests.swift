import UserNotifications
import XCTest
@testable import Reins

/// "Still waiting" reminders (the Android app's `TicketRemindersTest`): at 50 % and 90 % of a ticket's window, never
/// late, taken away when the ticket is gone, and carrying the item so Deny works on them.
final class TicketRemindersTests: XCTestCase {
    private let now: Int64 = 1_700_000_000

    private func ticket(_ id: String, deferredAt: Int64, deadline: Int64, state: String = "deferred", label: String = "Claude") -> PendingItem {
        PendingItem(
            kind: .request, id: id, title: "", subtitle: "", createdAt: deferredAt, connectionId: "c1", connectionLabel: label,
            action: "send", count: 1, service: "gmail", account: nil, waitUntil: deadline, op: "", opTitle: "", suggestion: nil,
            ticket: TicketView(ticket: "rt_\(id)", state: state, deferredAt: deferredAt, deadline: deadline, question: nil, agentReply: nil, stale: nil)
        )
    }

    private func instant(_ id: String) -> PendingItem {
        PendingItem(
            kind: .request, id: id, title: "", subtitle: "", createdAt: now, connectionId: "c1", connectionLabel: "Claude",
            action: "send", count: 1, service: "gmail", account: nil, waitUntil: now + 120, op: "", opTitle: "", suggestion: nil, canDefer: true
        )
    }

    func testRemindersFallAtHalfAndNinetyPercentOfTheWindow() {
        // A 4 h window opened now: 2 h and 3 h 36 min in.
        let plan = TicketReminders.plan([ticket("t1", deferredAt: now, deadline: now + 14_400)], now: now)
        XCTAssertEqual(plan.map(\.percent), [50, 90])
        XCTAssertEqual(plan.map(\.fireAt), [now + 7_200, now + 12_960])
        XCTAssertEqual(plan.map(\.identifier), ["ticket.t1.\(now + 14_400).50", "ticket.t1.\(now + 14_400).90"])
        XCTAssertEqual(TicketReminders.times(deferredAt: 1_000, deadline: 2_000).map(\.at), [1_500, 1_900])
        XCTAssertTrue(TicketReminders.times(deferredAt: 2_000, deadline: 2_000).isEmpty, "no window, no reminder")
    }

    func testNothingIsPlannedForTimesThatHavePassed() {
        // 60 % through: the 50 % reminder is not shown late, the 90 % one still comes.
        let midway = ticket("t1", deferredAt: now - 6_000, deadline: now + 4_000)
        XCTAssertEqual(TicketReminders.plan([midway], now: now).map(\.percent), [90])
        XCTAssertTrue(TicketReminders.plan([ticket("t2", deferredAt: now - 9_500, deadline: now + 500)], now: now).isEmpty)
        XCTAssertTrue(TicketReminders.plan([ticket("t3", deferredAt: now - 9_000, deadline: now - 1)], now: now).isEmpty, "past its deadline")
        XCTAssertTrue(TicketReminders.plan([ticket("t4", deferredAt: now, deadline: now + 9_000, state: "approved")], now: now).isEmpty)
        XCTAssertTrue(TicketReminders.plan([instant("i1")], now: now).isEmpty, "Instant requests are not tickets")
    }

    func testATicketWaitingForTheAgentsAnswerIsNotRemindedOf() {
        let asked = ticket("q", deferredAt: now, deadline: now + 10_000, state: "needs_info")
        XCTAssertTrue(TicketReminders.plan([asked], now: now).isEmpty, "it waits for the agent, not for you")
        // It had reminders before the Why?: they go, pending or shown.
        let before = ticket("q", deferredAt: now, deadline: now + 10_000)
        let scheduled = Set(TicketReminders.all([before], now: now).map(\.identifier))
        let open = Set(TicketReminders.all([asked], now: now).map(\.identifier))
        let (remove, add) = TicketReminders.diff(wanted: TicketReminders.plan([asked], now: now), open: open, scheduled: scheduled)
        XCTAssertEqual(remove, scheduled.sorted())
        XCTAssertTrue(add.isEmpty)
    }

    func testRemindersOfTicketsThatAreGoneAreTakenAway() {
        let a = ticket("a", deferredAt: now, deadline: now + 10_000)
        let b = ticket("b", deferredAt: now, deadline: now + 10_000)
        let scheduled = Set(TicketReminders.all([a, b], now: now).map(\.identifier))
        // b was answered: its reminders go, whether still pending or already shown; a's stay and nothing is added twice.
        let open = Set(TicketReminders.all([a], now: now).map(\.identifier))
        let (remove, add) = TicketReminders.diff(wanted: TicketReminders.plan([a], now: now), open: open, scheduled: scheduled)
        XCTAssertEqual(remove, ["ticket.b.\(now + 10_000).50", "ticket.b.\(now + 10_000).90"])
        XCTAssertTrue(add.isEmpty)
    }

    func testAShownReminderStaysWhileItsTicketWaits() {
        let a = ticket("a", deferredAt: now - 6_000, deadline: now + 4_000)
        let shown = "ticket.a.\(now + 4_000).50"
        let open = Set(TicketReminders.all([a], now: now).map(\.identifier))
        let (remove, add) = TicketReminders.diff(wanted: TicketReminders.plan([a], now: now), open: open, scheduled: [shown])
        XCTAssertTrue(remove.isEmpty, "the 50 % reminder already shown is not taken away")
        XCTAssertEqual(add.map(\.percent), [90])
    }

    func testAMovedDeadlineReplacesItsReminders() {
        let before = ticket("a", deferredAt: now, deadline: now + 10_000)
        let after = ticket("a", deferredAt: now, deadline: now + 20_000)
        let scheduled = Set(TicketReminders.all([before], now: now).map(\.identifier))
        let open = Set(TicketReminders.all([after], now: now).map(\.identifier))
        let (remove, add) = TicketReminders.diff(wanted: TicketReminders.plan([after], now: now), open: open, scheduled: scheduled)
        XCTAssertEqual(remove.count, 2)
        XCTAssertEqual(add.map(\.deadline), [now + 20_000, now + 20_000])
    }

    func testAReminderSaysWhatStillWaitsAndCanBeDenied() {
        let r = TicketReminders.plan([ticket("t1", deferredAt: now, deadline: now + 14_400)], now: now)
        let half = TicketReminders.content(r[0], settings: FeedbackSettings())
        XCTAssertTrue(half.title.hasPrefix("Still waiting: "), half.title)
        XCTAssertEqual(half.body, "Claude · Due in 2 h")
        XCTAssertEqual(half.categoryIdentifier, NotificationCategory.waiting.rawValue, "Deny and Deny with note")
        XCTAssertEqual(half.userInfo[NotificationKey.id] as? String, "t1")
        XCTAssertEqual(PushPayload(userInfo: half.userInfo)?.itemKind, .request)
        XCTAssertEqual(half.interruptionLevel, .active)
        // 90 % of a 4 h window leaves 24 min: under an hour, time-sensitive.
        XCTAssertEqual(TicketReminders.content(r[1], settings: FeedbackSettings()).interruptionLevel, .timeSensitive)
    }
}
