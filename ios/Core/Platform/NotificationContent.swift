import Foundation
import UserNotifications

/// The notification categories the app registers (`NotificationRouter.registerCategories`) and every notification
/// names: the push payload's `aps.category` is one of the first three.
enum NotificationCategory: String, CaseIterable {
    /// A request waits: "Deny" from the notification, a tap opens the approval sheet.
    case request
    /// A new AI connection waits.
    case pairing
    /// A file an AI uploaded waits.
    case blob
    /// Another phone asks for the account's keys.
    case join
    /// Autopilot, a bypass or Lockdown decided by itself: "Report" opens the activity entry.
    case autopilot
    /// A grant ends soon.
    case grant
    /// This phone's role, Autopilot pausing itself.
    case status

    /// What the lock screen says instead of the text while previews are hidden (the Android app's public versions).
    var hiddenPlaceholder: String {
        switch self {
        case .request, .pairing, .blob, .join: NotificationText.genericBody
        case .autopilot: "Open Reins to see what it was"
        case .grant: "A grant ends soon"
        case .status: "Open Reins to see more"
        }
    }

    var thread: String {
        switch self {
        case .request, .pairing, .blob, .join: "requests"
        case .autopilot: "autopilot"
        case .grant: "grants"
        case .status: "status"
        }
    }

    init(_ kind: Snapshot.Item.Kind) {
        switch kind {
        case .request: self = .request
        case .pairing: self = .pairing
        case .blob: self = .blob
        case .join: self = .join
        }
    }

    init(_ kind: PendingKind) {
        switch kind {
        case .request: self = .request
        case .pairing: self = .pairing
        case .blob: self = .blob
        case .join: self = .join
        }
    }
}

/// Action identifiers of the categories.
enum NotificationAction {
    static let deny = "deny"
    static let report = "report"
}

/// The `userInfo` keys of every notification the app or the extension shows. Pushes carry `t` and `id` themselves.
enum NotificationKey {
    static let kind = "t"
    static let id = "id"
    /// A `reins://` link, the tap's destination when present.
    static let link = "link"
    /// The activity entry of an automatic decision ("Report").
    static let activity = "activity"
}

/// The chimes (converted from the Android app's `fx_chime_*`), in the app bundle where the system finds them.
enum NotificationSoundFile {
    static let request = "reins_request.caf"
    static let attention = "reins_attention.caf"
    /// The soft click of an automatic denial (Android's `fx_close`).
    static let denied = "reins_denied.caf"
    /// A tenth of a second of silence. A notification without a sound neither sounds nor vibrates; one with a sound
    /// gets the system's alert haptic, as Settings > Sounds & Haptics sets it for the ring and silent modes. So with
    /// the sound off and haptics on, this is the sound: the phone vibrates and nothing is heard.
    static let silent = "reins_silent.caf"

    /// The file under the in-app switches: the chime when its sound is on, else the silence when haptics are on, else
    /// nothing (Sounds & haptics off silences and stills everything).
    static func file(_ file: String, _ category: CueCategory, _ settings: FeedbackSettings) -> String? {
        if settings.allows(category) { return file }
        return settings.hapticsOn ? silent : nil
    }

    static func sound(_ file: String, _ category: CueCategory, _ settings: FeedbackSettings) -> UNNotificationSound? {
        self.file(file, category, settings).map { UNNotificationSound(named: UNNotificationSoundName($0)) }
    }
}

/// What notifications say, kept apart so it can be tested (the Android app's `AppNotifier` wording and the
/// notification half of `AutopilotText`). Everything that came from an AI or a service goes through `untrusted`.
enum NotificationText {
    static let genericTitle = "Reins"
    static let genericBody = "Something is waiting for you"

    static func title(_ item: PendingItem) -> String {
        switch item.kind {
        case .request: item.action == "grant" ? "Permission requested" : "Approval needed"
        case .pairing: "Connect an AI"
        case .blob: "File to check"
        case .join: "Add a phone"
        }
    }

    /// "Claude: Send email to 2"; a pairing names the client.
    static func headline(_ item: PendingItem) -> String {
        if item.kind == .pairing || item.kind == .join { return untrusted(item.title) }
        return fullTitle(label: item.connectionLabel, action: item.action, count: Int(item.count), service: item.service, title: item.opTitle, op: item.op)
    }

    /// The headline, and under it what Autopilot would do (Assisted).
    /// What waits, then its detail (the account, the query) and Autopilot's suggestion, one per line. The detail is
    /// not the notification's subtitle: iOS sets that in bold above the body, which would put the query before what
    /// the AI wants to do.
    static func body(_ item: PendingItem) -> String {
        [headline(item), untrusted(item.subtitle), item.suggestion.map(untrusted) ?? ""]
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }

    static func decisionTitle(_ d: AutoDecisionView) -> String {
        let approved = d.verdict == .approve
        switch d.decidedBy {
        case "bypass": return approved ? "Approved by Bypass" : "Denied in Bypass"
        case "lockdown": return "Denied by Lockdown"
        default: return approved ? "Autopilot approved" : "Autopilot denied"
        }
    }

    /// "Push to a branch · dkat/reins — Claude Code".
    static func decisionText(_ d: AutoDecisionView) -> String { "\(untrusted(d.title)) — \(untrusted(d.connectionLabel))" }

    /// "97% sure" for Autopilot's own decisions; bypass and lockdown do not judge.
    static func decisionDetail(_ d: AutoDecisionView) -> String? {
        d.decidedBy == "autopilot" && d.confidence > 0 ? "\(percent(d.confidence)) sure" : nil
    }

    static func percent(_ p: Float) -> String { "\(Int((min(max(p, 0), 1) * 100).rounded()))%" }

    static func pausedTitle(_ label: String) -> String { "Autopilot paused for \(untrusted(label))" }

    static func pausedBody(_ reason: String) -> String {
        var why = untrusted(reason).trimmingCharacters(in: .whitespaces)
        while why.hasSuffix(".") { why.removeLast() }
        if why.isEmpty { why = "unusual volume" }
        return "Its requests wait for you again: \(why). Autopilot carries on by itself once things calm down."
    }

    static let replacedTitle = "This phone is no longer your approval device"
    static let replacedBody = "Another phone took over. Open Reins to register this one again."

    /// A push whose item turned out to be answered already (a grant, another device, its time ran out).
    static let handledTitle = "Already handled"
    static let handledBody = "Nothing is waiting for you."
}

/// Builds notification content; the app (`AppNotifier`, `GrantReminders`) and the notification extension use the same
/// so a push and a local notification for the same item look alike.
enum NotificationContent {
    /// An item that waits for the user.
    static func pending(_ item: PendingItem, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        let category = NotificationCategory(item.kind)
        c.title = NotificationText.title(item)
        c.body = NotificationText.body(item)
        c.categoryIdentifier = category.rawValue
        c.threadIdentifier = category.thread
        c.interruptionLevel = .timeSensitive
        c.relevanceScore = item.kind == .request ? 1 : 0.9
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.request, .requests, settings)
        let kind: Snapshot.Item.Kind = switch item.kind {
        case .request: .request
        case .pairing: .pairing
        case .blob: .blob
        case .join: .join
        }
        c.userInfo = [
            NotificationKey.kind: PushPayload.kind(of: kind),
            NotificationKey.id: item.id,
            NotificationKey.link: DeepLink.item(kind: kind, id: item.id).url.absoluteString,
        ]
        return c
    }

    /// What Autopilot (or a bypass, or Lockdown) decided by itself: approvals are listed and never heard, denials
    /// get the soft click.
    static func decision(_ d: AutoDecisionView, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        let approved = d.verdict == .approve
        c.title = NotificationText.decisionTitle(d)
        // The detail ("97% sure") goes under the text, not in the bold subtitle above it.
        c.body = [NotificationText.decisionText(d), NotificationText.decisionDetail(d) ?? ""].filter { !$0.isEmpty }.joined(separator: "\n")
        c.categoryIdentifier = NotificationCategory.autopilot.rawValue
        c.threadIdentifier = NotificationCategory.autopilot.thread
        c.interruptionLevel = approved ? .passive : .active
        c.relevanceScore = approved ? 0.1 : 0.3
        c.sound = approved ? nil : NotificationSoundFile.sound(NotificationSoundFile.denied, .autopilot, settings)
        var info: [String: Any] = [NotificationKey.link: DeepLink.home.url.absoluteString]
        if let entry = d.activityId {
            info[NotificationKey.activity] = entry
            info[NotificationKey.link] = DeepLink.activity(id: entry).url.absoluteString
        }
        c.userInfo = info
        return c
    }

    /// The push as the server sent it ("Reins", "Something is waiting for you"), with the sound the in-app switches
    /// allow and the category, thread and urgency of its kind. Used whenever the item cannot be read in time.
    static func generic(_ original: UNNotificationContent, payload: PushPayload?, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = (original.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        if c.title.isEmpty { c.title = NotificationText.genericTitle }
        if c.body.isEmpty { c.body = NotificationText.genericBody }
        let category = payload?.itemKind.map(NotificationCategory.init) ?? .request
        c.categoryIdentifier = category.rawValue
        c.threadIdentifier = category.thread
        c.interruptionLevel = .timeSensitive
        c.relevanceScore = 1
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.request, .requests, settings)
        if let payload, let link = payload.link {
            var info = c.userInfo
            info[NotificationKey.link] = link.url.absoluteString
            c.userInfo = info
        }
        return c
    }

    /// A push whose item is no longer waiting: a quiet line instead of an alarm for nothing.
    static func handled(_ original: UNNotificationContent) -> UNMutableNotificationContent {
        let c = (original.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        c.title = NotificationText.handledTitle
        c.subtitle = ""
        c.body = NotificationText.handledBody
        c.sound = nil
        c.interruptionLevel = .passive
        c.relevanceScore = 0
        c.categoryIdentifier = NotificationCategory.status.rawValue
        c.threadIdentifier = NotificationCategory.request.thread
        c.userInfo = [NotificationKey.link: DeepLink.home.url.absoluteString]
        return c
    }

    /// A change to this phone's role or to Autopilot; `silent` in front, where the app chimes itself.
    static func status(title: String, body: String, link: DeepLink, settings: FeedbackSettings, silent: Bool) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        c.title = title
        c.body = body
        c.categoryIdentifier = NotificationCategory.status.rawValue
        c.threadIdentifier = NotificationCategory.status.thread
        c.interruptionLevel = .active
        c.relevanceScore = 0.5
        c.sound = silent ? nil : NotificationSoundFile.sound(NotificationSoundFile.attention, .alerts, settings)
        c.userInfo = [NotificationKey.link: link.url.absoluteString]
        return c
    }
}

/// What the notification extension leaves to the app.
enum ExtensionPolicy {
    /// Any connection in a mode that judges with the model (Assisted, Auto), with the model there to run. Only the app
    /// runs it, so the extension parks such a push without Autopilot's pass (`handlePushDeferringAutopilot`) and the
    /// app judges it; handling it fully here would settle the item as "could not judge" for good.
    static func needsModel(_ s: AutopilotSettings) -> Bool {
        guard s.model.state == .installed else { return false }
        let modes = [s.mode] + s.connections.map(\.mode)
        return modes.contains { $0 == .assisted || $0 == .auto }
    }
}
