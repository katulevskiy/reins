import Foundation
import UserNotifications

/// "This grant is about to end" reminders (the Android app's `GrantReminders`): one local notification per running grant
/// that ends by itself, due shortly before it does (`leadSeconds`), with the attention chime. The plan is rebuilt
/// whenever the grants are read (`AppModel.onGrantsChanged`), so resumed, deleted or used-up grants never remind; a
/// reminder is per end time, so resuming a grant makes a new one. The system keeps scheduled notifications across
/// launches and restarts, so nothing needs re-arming. Tapping one opens `reins://grant?id=`.
enum GrantReminders {
    struct Reminder: Equatable {
        var id: String
        var fireAt: Int64
        var expiresAt: Int64
        var label: String
        var summary: String

        /// The notification's identifier: per grant and end time.
        var identifier: String { "\(GrantReminders.prefix)\(id).\(expiresAt)" }
    }

    static let prefix = "grant."
    private static let scheduledKey = "grantReminders.scheduled.v1"

    private static let minute: Int64 = 60
    private static let hour: Int64 = 3_600

    /// How long before it ends a grant counts as ending soon (and the user is reminded): a tenth of its life, 5 min to
    /// 1 h (Android's `expiryLeadSeconds`).
    static func leadSeconds(createdAt: Int64, expiresAt: Int64) -> Int64 {
        min(max((expiresAt - createdAt) / 10, 5 * minute), hour)
    }

    /// The moment the reminder is due, or nil for a grant that does not run out on its own.
    static func reminderAt(createdAt: Int64, expiresAt: Int64?) -> Int64? {
        expiresAt.map { $0 - leadSeconds(createdAt: createdAt, expiresAt: $0) }
    }

    /// The reminders the running `grants` call for (those that have not ended yet).
    static func plan(_ grants: [GrantView], now: Int64) -> [Reminder] {
        grants.filter(\.active).compactMap { g in
            guard let end = g.expiresAt, end > now, let at = reminderAt(createdAt: g.createdAt, expiresAt: end) else { return nil }
            return Reminder(id: g.id, fireAt: at, expiresAt: end, label: g.connectionLabel, summary: g.summary)
        }
    }

    /// A grant already inside its last stretch reminds at once (a second from now).
    static func dueAt(_ r: Reminder, now: Int64) -> Int64 { max(r.fireAt, now + 1) }

    /// What changes: identifiers to take away, reminders to add. `scheduled` are the identifiers already handed to the
    /// system (pending or fired); those are never scheduled twice.
    static func diff(wanted: [Reminder], scheduled: Set<String>) -> (remove: [String], add: [Reminder]) {
        let ids = Set(wanted.map(\.identifier))
        return (scheduled.subtracting(ids).sorted(), wanted.filter { !scheduled.contains($0.identifier) })
    }

    /// "47m", "3h", "5d", "10w", "2y": the time left in its largest unit (seconds only for the last minute).
    static func compactDuration(_ seconds: Int64) -> String {
        let s = max(seconds, 0)
        switch s {
        case ..<minute: return "\(s)s"
        case ..<hour: return "\(s / minute)m"
        case ..<86_400: return "\(s / hour)h"
        case ..<(7 * 86_400): return "\(s / 86_400)d"
        case ..<(365 * 86_400): return "\(s / (7 * 86_400))w"
        default: return "\(s / (365 * 86_400))y"
        }
    }

    static func content(_ r: Reminder, now: Int64, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        c.title = "Access ends in \(compactDuration(r.expiresAt - dueAt(r, now: now)))"
        c.body = "\(untrusted(r.label)): \(untrusted(r.summary))"
        c.categoryIdentifier = NotificationCategory.grant.rawValue
        c.threadIdentifier = NotificationCategory.grant.thread
        c.interruptionLevel = .active
        c.relevanceScore = 0.6
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.attention, .alerts, settings)
        c.userInfo = [NotificationKey.link: DeepLink.grant(id: r.id).url.absoluteString]
        return c
    }

    /// Aligns the scheduled reminders with `grants`.
    @MainActor
    static func sync(_ grants: [GrantView], now: Int64 = Int64(Date().timeIntervalSince1970), center: UNUserNotificationCenter = .current(), defaults: UserDefaults = .standard) {
        let wanted = plan(grants, now: now)
        let scheduled = Set(defaults.stringArray(forKey: scheduledKey) ?? [])
        let (remove, add) = diff(wanted: wanted, scheduled: scheduled)
        let settings = FeedbackSettings.load()
        let requests = add.map { r in
            let trigger = UNTimeIntervalNotificationTrigger(timeInterval: TimeInterval(max(dueAt(r, now: now) - now, 1)), repeats: false)
            return UNNotificationRequest(identifier: r.identifier, content: content(r, now: now, settings: settings), trigger: trigger)
        }
        // Off the main thread: the notification center's calls go through a system service that can be slow to
        // answer (and never answers on a simulator whose notification service has not started).
        notificationQueue.async {
            if !remove.isEmpty {
                center.removePendingNotificationRequests(withIdentifiers: remove)
                center.removeDeliveredNotifications(withIdentifiers: remove)
            }
            requests.forEach { center.add($0) }
        }
        defaults.set(wanted.map(\.identifier).sorted(), forKey: scheduledKey)
    }
}

/// Where the app talks to the notification center when it does not need the answer.
let notificationQueue = DispatchQueue(label: "com.reins2fa.app.notifications", qos: .utility)
