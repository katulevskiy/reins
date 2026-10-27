import Foundation
import UserNotifications

/// Overnight's morning digest (Decisions v2, 01 §11, 03 §5.2): what Overnight parked never notifies on arrival, and the
/// owner gets ONE local notification at the morning time ("3 waiting", "1 from Claude Code (Fix CI), 2 from ChatGPT").
/// It is planned again whenever the waiting list is read (`AppModel.onPendingChanged`), under one fixed identifier, so a
/// newer digest replaces the earlier one and nothing waiting takes it away. Only when something waits: automatic
/// decisions, a task's start or end never notify.
enum OvernightDigestNotice {
    /// The one identifier the digest is filed under (per profile, so a work profile's never replaces the personal one).
    static func identifier(_ profile: WorkProfiles.Active = WorkProfiles.personal) -> String { "overnight.digest.\(profile.id)" }

    /// What to do with the notification (pure, unit-tested).
    enum Plan: Equatable {
        /// One notification at `at` (unix seconds), replacing any earlier one.
        case schedule(at: Int64, title: String, body: String)
        /// No notification to come. `keepDelivered`: the morning came and the digest was shown, and what it lists still
        /// waits; when nothing waits any more the shown one goes too.
        case cancel(keepDelivered: Bool)
    }

    static func plan(_ digest: OvernightDigest?, now: Int64) -> Plan {
        guard let digest, digest.total > 0 else { return .cancel(keepDelivered: false) }
        guard let at = digest.at, at > now else { return .cancel(keepDelivered: true) }
        return .schedule(at: at, title: untrusted(digest.title), body: untrusted(digest.body))
    }

    /// Whether `item` is parked for the morning now: no notification on its arrival, no reminder before then.
    static func isParked(_ item: PendingItem, now: Int64) -> Bool { (item.parkedUntil ?? 0) > now }

    /// The notification: a quiet status line that opens Activity (its "Waiting for you").
    static func content(title: String, body: String, settings: FeedbackSettings, profile: WorkProfiles.Active) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        c.title = title
        c.body = body
        // Not an item's category: the sweep of item notifications (`NotificationSweep`) leaves it alone.
        c.categoryIdentifier = NotificationCategory.status.rawValue
        c.threadIdentifier = WorkProfiles.thread("requests", prefix: profile.threadPrefix)
        c.interruptionLevel = .active
        c.relevanceScore = 0.8
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.attention, .alerts, settings)
        c.userInfo = [NotificationKey.link: DeepLink.home.url.absoluteString, WorkProfiles.notificationKey: profile.id]
        return c
    }

    /// The calendar trigger for `at`, to the second, in the phone's calendar.
    static func trigger(at: Int64, calendar: Calendar = .current) -> UNCalendarNotificationTrigger {
        let date = Date(timeIntervalSince1970: TimeInterval(at))
        let parts = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        return UNCalendarNotificationTrigger(dateMatching: parts, repeats: false)
    }

    /// The scheduling request for a plan; nil for a cancel.
    static func request(_ plan: Plan, settings: FeedbackSettings, profile: WorkProfiles.Active) -> UNNotificationRequest? {
        guard case let .schedule(at, title, body) = plan else { return nil }
        return UNNotificationRequest(
            identifier: identifier(profile), content: content(title: title, body: body, settings: settings, profile: profile),
            trigger: trigger(at: at)
        )
    }

    /// Applies `plan` through `scheduler`; `last` is the plan applied before (an unchanged plan is not applied again).
    /// Returns the plan now in force.
    @discardableResult
    static func apply(_ plan: Plan, last: Plan?, scheduler: DigestScheduling, settings: FeedbackSettings, profile: WorkProfiles.Active) -> Plan {
        guard plan != last else { return plan }
        let id = identifier(profile)
        switch plan {
        case .schedule:
            if let request = request(plan, settings: settings, profile: profile) {
                scheduler.removePending([id])
                scheduler.add(request)
            }
        case let .cancel(keepDelivered):
            scheduler.removePending([id])
            if !keepDelivered { scheduler.removeDelivered([id]) }
        }
        return plan
    }

    /// Reads the digest from `core` and aligns the notification with it. A digest that cannot be read changes nothing.
    @MainActor
    static func sync(
        _ core: any ReinsCoreProtocol, scheduler: DigestScheduling = NotificationCenterScheduler(), profile: WorkProfiles.Active = WorkProfiles.active,
        now: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) async {
        let digest: OvernightDigest?
        do {
            digest = try await core.overnightDigest()
        } catch {
            // Offline or busy: the notification stays as planned; the next read aligns it.
            return
        }
        let next = Self.plan(digest, now: now())
        lastPlan[profile.id] = apply(next, last: lastPlan[profile.id], scheduler: scheduler, settings: FeedbackSettings.load(), profile: profile)
    }

    /// The plan applied last, per profile (this process only: a fresh launch applies again, which is harmless).
    @MainActor private static var lastPlan: [String: Plan] = [:]
}

/// Where the digest's notification is handed to the system (a fake in tests).
protocol DigestScheduling: AnyObject {
    func add(_ request: UNNotificationRequest)
    func removePending(_ identifiers: [String])
    func removeDelivered(_ identifiers: [String])
}

/// The notification center, called off the main thread (it can be slow to answer).
final class NotificationCenterScheduler: DigestScheduling {
    private let center: UNUserNotificationCenter

    init(center: UNUserNotificationCenter = .current()) { self.center = center }

    func add(_ request: UNNotificationRequest) {
        let center = center
        notificationQueue.async { center.add(request) }
    }

    func removePending(_ identifiers: [String]) {
        let center = center
        notificationQueue.async { center.removePendingNotificationRequests(withIdentifiers: identifiers) }
    }

    func removeDelivered(_ identifiers: [String]) {
        let center = center
        notificationQueue.async { center.removeDeliveredNotifications(withIdentifiers: identifiers) }
    }
}
