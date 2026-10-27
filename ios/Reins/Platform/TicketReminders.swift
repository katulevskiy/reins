import Foundation
import UserNotifications

/// "Still waiting" reminders for tickets (the Android app's `TicketReminders`): two local notifications per ticket, at
/// 50 % and 90 % of its window (`[deferredAt, deadline]`). The plan is rebuilt whenever the waiting list is read
/// (`AppModel.onPendingChanged`), so a ticket that was answered, denied, expired or decided elsewhere never reminds; a
/// reminder is per deadline, so a window that moved (a Why?, an escalation) gets new ones. They carry the item, so
/// "Deny" and "Deny with note" work on them as on the arrival, and the sweep takes them away once the item is gone.
enum TicketReminders {
    struct Reminder: Equatable {
        var id: String
        /// 50 or 90: how far into the window it falls.
        var percent: Int
        var fireAt: Int64
        var deadline: Int64
        /// The request, as everywhere ("Push 3 commits to octo/app").
        var title: String
        /// Who asks, its own name.
        var who: String

        /// The notification's identifier: per ticket, deadline and mark.
        var identifier: String { "\(TicketReminders.prefix)\(id).\(deadline).\(percent)" }
    }

    static let prefix = "ticket."
    static let marks = [50, 90]
    private static let scheduledKey = "ticketReminders.scheduled.v1"

    /// The moments the reminders fall: `percent` of the way from `deferredAt` to `deadline`. None for an empty window.
    static func times(deferredAt: Int64, deadline: Int64) -> [(percent: Int, at: Int64)] {
        guard deadline > deferredAt else { return [] }
        return marks.map { ($0, deferredAt + (deadline - deferredAt) * Int64($0) / 100) }
    }

    /// Every reminder of the tickets in `pending` that wait for the reviewer (not past their deadline, not approved,
    /// not waiting for the agent's answer to a Why?), whether its time has come or not. A ticket Overnight parked is
    /// reminded of only after the morning (`parkedUntil`).
    static func all(_ pending: [PendingItem], now: Int64) -> [Reminder] {
        pending.flatMap { item -> [Reminder] in
            guard item.kind == .request, let t = item.ticket, t.deadline > now, !["approved", "needs_info"].contains(t.state) else { return [] }
            let title = AskLabels.pending(item, field: FieldKind.of(ActionKind.of(item.action))).text
            let who = untrustedName(item.connectionLabel, limit: 40)
            // Parked by Overnight: the reminders fall in what is left of the window after the morning, never before.
            let start = max(t.deferredAt, item.parkedUntil ?? t.deferredAt)
            return times(deferredAt: start, deadline: t.deadline).map { mark in
                Reminder(id: item.id, percent: mark.percent, fireAt: mark.at, deadline: t.deadline, title: title, who: who)
            }
        }
    }

    /// The reminders to schedule: those of open tickets still ahead of `now`. One whose time has passed is never shown
    /// late.
    static func plan(_ pending: [PendingItem], now: Int64) -> [Reminder] {
        all(pending, now: now).filter { $0.fireAt > now }
    }

    /// What changes: identifiers to take away (`scheduled` ones whose ticket is gone, decided or moved its deadline,
    /// pending or already shown), and reminders to add (`wanted` ones not handed to the system yet). `open` are the
    /// identifiers of every reminder of the open tickets (`all`): one already shown stays while its ticket waits.
    static func diff(wanted: [Reminder], open: Set<String>, scheduled: Set<String>) -> (remove: [String], add: [Reminder]) {
        (scheduled.subtracting(open).sorted(), wanted.filter { !scheduled.contains($0.identifier) })
    }

    /// "Still waiting: Push 3 commits to octo/app", "Claude · Due in 2 h". Less than an hour left: time-sensitive.
    static func content(_ r: Reminder, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        c.title = "Still waiting: \(r.title)"
        c.body = [r.who, TicketText.due(deadline: r.deadline, now: r.fireAt)].filter { !$0.isEmpty }.joined(separator: " · ")
        c.categoryIdentifier = NotificationCategory.waiting.rawValue
        c.threadIdentifier = NotificationCategory.waiting.thread
        let urgent = TicketText.urgent(deadline: r.deadline, now: r.fireAt)
        c.interruptionLevel = urgent ? .timeSensitive : .active
        c.relevanceScore = urgent ? 0.9 : 0.6
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.attention, .alerts, settings)
        c.userInfo = [
            NotificationKey.kind: PushPayload.kind(of: .request),
            NotificationKey.id: r.id,
            NotificationKey.link: DeepLink.item(kind: .request, id: r.id).url.absoluteString,
        ]
        return c
    }

    /// Aligns the scheduled reminders with `pending`: those of tickets gone, decided or now waiting for the agent are
    /// taken away.
    @MainActor
    static func sync(
        _ pending: [PendingItem], now: Int64 = Int64(Date().timeIntervalSince1970), center: UNUserNotificationCenter = .current(),
        defaults: UserDefaults = .standard
    ) {
        let open = Set(all(pending, now: now).map(\.identifier))
        let wanted = plan(pending, now: now)
        let scheduled = Set(defaults.stringArray(forKey: scheduledKey) ?? [])
        let (remove, add) = diff(wanted: wanted, open: open, scheduled: scheduled)
        guard !remove.isEmpty || !add.isEmpty else { return }
        let settings = FeedbackSettings.load()
        let requests = add.map { r in
            let trigger = UNTimeIntervalNotificationTrigger(timeInterval: TimeInterval(max(r.fireAt - now, 1)), repeats: false)
            return UNNotificationRequest(identifier: r.identifier, content: content(r, settings: settings), trigger: trigger)
        }
        // Off the main thread, as `GrantReminders`: the notification center can be slow to answer.
        notificationQueue.async {
            if !remove.isEmpty {
                center.removePendingNotificationRequests(withIdentifiers: remove)
                center.removeDeliveredNotifications(withIdentifiers: remove)
            }
            requests.forEach { center.add($0) }
        }
        defaults.set(scheduled.intersection(open).union(add.map(\.identifier)).sorted(), forKey: scheduledKey)
    }
}
