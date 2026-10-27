import Foundation
import UserNotifications

/// The notification categories the app registers (`NotificationRouter.registerCategories`) and every notification
/// names: the push payload's `aps.category` is one of the first three.
enum NotificationCategory: String, CaseIterable {
    /// A request waits: "Deny" from the notification, a tap opens the approval sheet.
    case request
    /// A routine request waits (`PendingItem.quick`): "Deny" and "Approve" from the notification. Never one that is
    /// asked every time; the core refuses `approveQuick` for those anyway.
    case quickRequest
    /// A request that became a ticket waits under "Waiting for you" (the agent carries on): it arrives quietly unless
    /// little time is left, and its reminders say so. "Deny" and "Deny with note" from the notification, a tap opens it.
    case waiting
    /// A new AI connection waits.
    case pairing
    /// A file an AI uploaded waits.
    case blob
    /// Another phone asks for the account's keys.
    case join
    /// A pushed item the extension could not read in time (the app holds the store, the network failed): its generic
    /// text, with no action, since it may be one a bypass or Autopilot answers. A tap opens it if it waits.
    case arriving
    /// A request Autopilot (Auto, Overnight) has still to decide, which only the app can: no action, a tap opens Reins.
    case deciding
    /// Autopilot, a bypass or Lockdown decided by itself: "Report" opens the activity entry.
    case autopilot
    /// A grant ends soon.
    case grant
    /// This phone's role, Autopilot pausing itself.
    case status
    /// An org review waits for this member's signature (a work profile's, O2): a tap opens its reviews.
    case orgReview = "org_review"
    /// Circles: another account's request waits for this phone ("Sam: purchase $42 waiting"), or a guardianship ended.
    /// No action: a tap opens Reins, where "For others" answers it.
    case circle

    /// What the lock screen says instead of the text while previews are hidden (the Android app's public versions).
    var hiddenPlaceholder: String {
        switch self {
        case .request, .quickRequest, .waiting, .pairing, .blob, .join, .arriving: NotificationText.genericBody
        case .deciding: NotificationText.decidingBody
        case .autopilot: "Open Reins to see what it was"
        case .grant: "A grant ends soon"
        case .status: "Open Reins to see more"
        case .orgReview: "A review waits for you"
        case .circle: NotificationText.circleHidden
        }
    }

    /// The thread notifications of this category join: the active profile's own (`WorkProfiles.thread`).
    var thread: String {
        let base = switch self {
        case .request, .quickRequest, .waiting, .pairing, .blob, .join, .arriving, .deciding: "requests"
        case .autopilot: "autopilot"
        case .grant: "grants"
        case .status: "status"
        case .orgReview: "reviews"
        case .circle: "circles"
        }
        return WorkProfiles.thread(base)
    }

    init(_ kind: Snapshot.Item.Kind) {
        switch kind {
        case .request: self = .request
        case .pairing: self = .pairing
        case .blob: self = .blob
        case .join: self = .join
        }
    }

    /// The category of a waiting item's notification: a ticket's own, a routine request's with Approve beside Deny;
    /// what is asked every time only opens.
    static func of(_ item: PendingItem) -> NotificationCategory {
        guard item.kind == .request else { return NotificationCategory(item.kind) }
        if item.ticket != nil || item.parkedUntil != nil { return .waiting }
        return item.quick ? .quickRequest : .request
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
    /// "Deny with note" (`waiting`): a text input whose words reach the agent with the denial.
    static let denyNote = "denyNote"
    static let approve = "approve"
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

    /// The bold line (the Outside board): for a request or a file, the ask itself with who asks in front ("Claude: Read
    /// 3 emails", ASK-TEXT's list form: the name is its own run before the colon, capped, never inside Reins' sentence);
    /// for a pairing or a new phone, Reins' own words.
    static func title(_ item: PendingItem) -> String {
        if asksInTitle(item) { return headline(item) }
        return kindTitle(item)
    }

    /// Whether the title is the ask (who asks and what), rather than the kind's words.
    static func asksInTitle(_ item: PendingItem) -> Bool { item.kind == .request || item.kind == .blob }

    /// "Approval needed", "Connect an AI": what kind of thing waits.
    static func kindTitle(_ item: PendingItem) -> String {
        switch item.kind {
        case .request: item.action == "grant" ? "Permission requested" : "Approval needed"
        case .pairing: "Connect an AI"
        case .blob: "File to check"
        case .join: "Add a phone"
        }
    }

    /// The request as everywhere else (ASK-TEXT): "Claude: Push 3 commits to octo/app". A pairing or a new phone in
    /// Reins' own words: the other side's name is never the sentence.
    static func headline(_ item: PendingItem) -> String { AskLabels.listTitle(item) }

    /// What waits, its machine details (a branch, a command, an address, the other side's quoted name), what approving
    /// does (the core's sentence), its detail (the account) and Autopilot's suggestion, one per line. The detail is not
    /// the notification's subtitle: iOS sets that in bold above the body, which would put it before what is asked.
    static func body(_ item: PendingItem) -> String {
        let subtitle = untrusted(item.subtitle).trimmingCharacters(in: .whitespacesAndNewlines)
        // The details under the sentence, unless they are the summary that comes after the core's sentence anyway.
        let mono = line(item).mono.flatMap { $0 == subtitle ? nil : $0 } ?? ""
        return [asksInTitle(item) ? "" : headline(item), mono, untrusted(item.headline), subtitle, item.suggestion.map(untrusted) ?? ""]
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }

    private static func line(_ item: PendingItem) -> AskLine { AskLabels.pending(item, field: FieldKind.of(ActionKind.of(item.action))) }

    /// Who decided, and how: "Approved by Autopilot", "Denied in Bypass".
    static func decisionTitle(_ d: AutoDecisionView) -> String {
        let approved = d.verdict == .approve
        switch d.decidedBy {
        case "bypass": return approved ? "Approved by Bypass" : "Denied in Bypass"
        case "overnight": return approved ? "Approved by \(AutopilotText.overnightName)" : "Denied by \(AutopilotText.overnightName)"
        case "lockdown": return "Denied by Lockdown"
        default: return approved ? "Approved by Autopilot" : "Denied by Autopilot"
        }
    }

    /// "Claude Code: Push to a branch · dkat/reins": what was asked, who asked in front (its own run, capped).
    static func decisionText(_ d: AutoDecisionView) -> String {
        let who = untrustedName(d.connectionLabel, limit: 40)
        let what = untrusted(d.title)
        return who.isEmpty ? what : "\(who): \(what)"
    }

    /// "97%" for Autopilot's own decisions (Overnight's too); bypass and lockdown do not judge.
    static func decisionDetail(_ d: AutoDecisionView) -> String? {
        ["autopilot", "overnight"].contains(d.decidedBy) && d.confidence > 0 ? percent(d.confidence) : nil
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

    /// A request Autopilot has still to decide, which needs the app (its model runs there only).
    static let decidingBody = "Open Reins so Autopilot can decide"

    /// A Circle notification's lock-screen text while previews are hidden.
    static let circleHidden = "Someone needs your answer"

    /// A Circle push the extension could not read: "Reins", and that someone needs an answer.
    static let circleGenericBody = "Someone in your Circle needs your answer"

    /// Under a review's title: what it is about and when it is due ("Lego set · amazon.com", "Due in 3 h").
    static func circleBody(_ r: CircleReviewView, now: Int64) -> String {
        [untrusted(r.subtitle), CircleLogic.due(r.deadline, now: now)].filter { !$0.isEmpty }.joined(separator: "\n")
    }

    /// A guardianship that ended or needs a new invitation, as the guardian is told (08 R7).
    static func circleAlertTitle(_ p: CirclePerson) -> String { CircleLogic.name(p.peer) }

    static func circleAlertBody(_ p: CirclePerson) -> String {
        switch p.state {
        case .ended: "\(CircleLogic.roleName(p.role)) ended"
        case .suspended: "Their phone changed. Invite them again."
        case .paused: "\(CircleLogic.roleName(p.role)) paused"
        case .active: "\(CircleLogic.roleName(p.role)) changed"
        }
    }
}

/// Builds notification content; the app (`AppNotifier`, `GrantReminders`) and the notification extension use the same
/// so a push and a local notification for the same item look alike.
enum NotificationContent {
    /// An item that waits for the user.
    static func pending(
        _ item: PendingItem, settings: FeedbackSettings, now: Int64 = Int64(Date().timeIntervalSince1970)
    ) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        let category = NotificationCategory.of(item)
        c.title = NotificationText.title(item)
        c.body = NotificationText.body(item)
        c.categoryIdentifier = category.rawValue
        c.threadIdentifier = category.thread
        c.interruptionLevel = .timeSensitive
        c.relevanceScore = item.kind == .request ? 1 : 0.9
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.request, .requests, settings)
        if let ticket = item.ticket {
            // Nobody waits on it: quiet (no sound, no buzz) unless less than an hour is left.
            let urgent = TicketText.urgent(deadline: ticket.deadline, now: now)
            c.body = [c.body, TicketText.due(deadline: ticket.deadline, now: now), TicketText.agentSays(ticket).map { "\(TicketText.agentSaysLabel): \($0)" } ?? ""]
                .filter { !$0.isEmpty }
                .joined(separator: "\n")
            c.interruptionLevel = urgent ? .timeSensitive : .active
            c.relevanceScore = urgent ? 0.9 : 0.5
            if !urgent { c.sound = nil }
        }
        if item.parked {
            // Parked by Overnight for the morning: it never alerts (the extension cannot drop a push, so it lands in
            // the list without a sound or a lit screen); the morning digest is the one that rings.
            c.interruptionLevel = .passive
            c.relevanceScore = 0.1
            c.sound = nil
        }
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

    /// Another account's request waits for this phone (04 §6.2): "Sam: purchase $42 waiting". No action: answering
    /// is done in the app, with the card in front.
    static func circleReview(
        _ r: CircleReviewView, settings: FeedbackSettings, now: Int64 = Int64(Date().timeIntervalSince1970)
    ) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        c.title = CircleLogic.notificationTitle(r)
        c.body = NotificationText.circleBody(r, now: now)
        c.categoryIdentifier = NotificationCategory.circle.rawValue
        c.threadIdentifier = NotificationCategory.circle.thread
        c.interruptionLevel = .timeSensitive
        c.relevanceScore = 0.9
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.request, .requests, settings)
        c.userInfo = circleInfo(reviewId(r.id))
        return c
    }

    /// A guardianship ended (or its other side's phone changed): the guardian is told at once.
    static func circleAlert(_ p: CirclePerson, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        c.title = NotificationText.circleAlertTitle(p)
        c.body = NotificationText.circleAlertBody(p)
        c.categoryIdentifier = NotificationCategory.circle.rawValue
        c.threadIdentifier = NotificationCategory.circle.thread
        c.interruptionLevel = .active
        c.relevanceScore = 0.5
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.attention, .alerts, settings)
        c.userInfo = circleInfo("alert:\(p.linkId)")
        return c
    }

    /// A Circle notification's `userInfo`: what it is about as the push names it (`review:<id>`, `alert:<link>`, so the
    /// extension's and the app's match), and the tap's destination.
    static func circleInfo(_ pushId: String) -> [String: Any] {
        [NotificationKey.kind: "circle", NotificationKey.id: pushId, NotificationKey.link: DeepLink.home.url.absoluteString]
    }

    /// The push id of a review's notification.
    static func reviewId(_ id: String) -> String { "review:\(id)" }

    /// What Autopilot (or a bypass, or Lockdown) decided by itself: approvals are listed and never heard, denials
    /// get the soft click.
    static func decision(_ d: AutoDecisionView, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        let approved = d.verdict == .approve
        // What happened in the bold line, who decided and how sure under it ("Approved by Autopilot · 97%").
        c.title = NotificationText.decisionText(d)
        c.body = [NotificationText.decisionTitle(d), NotificationText.decisionDetail(d) ?? ""].filter { !$0.isEmpty }.joined(separator: " · ")
        c.categoryIdentifier = NotificationCategory.autopilot.rawValue
        c.threadIdentifier = NotificationCategory.autopilot.thread
        // A decision needs nothing from the owner, so it never alerts: no sound, no banner, no lit screen. The extension
        // has to show something for a push it can't silence (that needs Apple's filtering entitlement), and the app
        // sweeps these away as soon as it runs (`NotificationSweep.isStale`).
        _ = approved
        _ = settings
        c.interruptionLevel = .passive
        c.relevanceScore = 0
        c.sound = nil
        var info: [String: Any] = [NotificationKey.link: DeepLink.home.url.absoluteString]
        if let entry = d.activityId {
            info[NotificationKey.activity] = entry
            info[NotificationKey.link] = DeepLink.activity(id: entry).url.absoluteString
        }
        c.userInfo = info
        return c
    }

    /// The push as the server sent it ("Reins", "Something is waiting for you"), with the sound the in-app switches
    /// allow and the thread and urgency of its kind. Used whenever the item cannot be read in time. A request gets no
    /// action (`arriving`): unread, it may be one a bypass, Lockdown or Autopilot answers, and the app takes this away
    /// once it knows (`AppNotifier`). Pairings, files and other phones keep their own (action-free) categories.
    static func generic(_ original: UNNotificationContent, payload: PushPayload?, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = (original.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        if c.title.isEmpty { c.title = NotificationText.genericTitle }
        if c.body.isEmpty { c.body = NotificationText.genericBody }
        let kind = payload?.itemKind.map(NotificationCategory.init) ?? .request
        let category: NotificationCategory = payload?.circle != nil ? .circle : kind == .request ? .arriving : kind
        if category == .circle, c.body == NotificationText.genericBody { c.body = NotificationText.circleGenericBody }
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

    /// A request Autopilot (Auto, Overnight) has still to decide, which only the app can: who asks for what, and
    /// that opening Reins lets Autopilot decide. Never an ask: no action, and a tap opens Reins, not the request; if
    /// Autopilot leaves it to the owner, the app asks then (and takes this away).
    static func deciding(_ item: PendingItem, settings: FeedbackSettings) -> UNMutableNotificationContent {
        let c = UNMutableNotificationContent()
        c.title = NotificationText.headline(item)
        c.body = NotificationText.decidingBody
        c.categoryIdentifier = NotificationCategory.deciding.rawValue
        c.threadIdentifier = NotificationCategory.deciding.thread
        c.interruptionLevel = .active
        c.relevanceScore = 0.6
        c.sound = NotificationSoundFile.sound(NotificationSoundFile.attention, .alerts, settings)
        let kind: Snapshot.Item.Kind = switch item.kind {
        case .request: .request
        case .pairing: .pairing
        case .blob: .blob
        case .join: .join
        }
        c.userInfo = [
            NotificationKey.kind: PushPayload.kind(of: kind),
            NotificationKey.id: item.id,
            NotificationKey.link: DeepLink.home.url.absoluteString,
        ]
        return c
    }

    /// What the extension shows for a push, from what the core said about its item (`PushOutcome`).
    static func outcome(_ outcome: PushOutcome, original: UNNotificationContent, settings: FeedbackSettings) -> UNMutableNotificationContent {
        switch outcome {
        case let .decided(d): decision(d, settings: settings)
        case let .waiting(item): pending(item, settings: settings)
        case let .deciding(item): deciding(item, settings: settings)
        case .handled: handled(original)
        }
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

/// What a pushed item turned out to be, once the core handled the push: what the extension shows. Only an item that
/// waits for its owner is an ask; what a mode decided is said as decided, and what Autopilot has still to decide is
/// never an ask.
enum PushOutcome {
    /// A bypass, Lockdown, Overnight or Autopilot decided it.
    case decided(AutoDecisionView)
    /// It waits for the owner (the core's `pending`).
    case waiting(PendingItem)
    /// Autopilot has still to decide it, which only the app can (the core's `deciding`).
    case deciding(PendingItem)
    /// Answered before it could be shown (a grant, another device, its time ran out).
    case handled

    /// A decision first (it is final), then the owner's list, then what is still to be decided.
    static func of(decision: AutoDecisionView?, waiting: PendingItem?, deciding: PendingItem?) -> PushOutcome {
        if let decision { return .decided(decision) }
        if let waiting { return .waiting(waiting) }
        if let deciding { return .deciding(deciding) }
        return .handled
    }
}

/// Which delivered notifications no longer have anything to offer: an item's ask once the item stops waiting (answered
/// here or elsewhere, decided by a mode), and the extension's own texts once the app knows better.
enum NotificationSweep {
    /// The categories of notifications about one item that the sweep looks at (another phone's ask is left alone).
    static let itemCategories: Set<String> = Set(
        [NotificationCategory.request, .quickRequest, .waiting, .pairing, .blob, .arriving, .deciding].map(\.rawValue)
    )

    /// How long a push the extension could not read is left alone for the app to fetch it.
    static let unreadGrace: TimeInterval = 60
    /// How long "Autopilot has still to decide" stays without the item being decided or asked: as long as the server
    /// holds a request.
    static let decidingLifetime: TimeInterval = 600

    /// Whether a delivered notification of `category` about item `id` (delivered at `date`) is stale, `waiting` being the
    /// owner's list. One about an item that waits is kept. "Autopilot has still to decide" is never in that list: it goes
    /// when the app decides or asks (`AppNotifier`), or when the request's time is up.
    static func isStale(category: String, id: String, date: Date, waiting: Set<String>, now: Date) -> Bool {
        // A mode's or Autopilot's decision has nothing to offer once the app runs: it is in Activity.
        if category == NotificationCategory.autopilot.rawValue { return true }
        guard itemCategories.contains(category), !waiting.contains(id) else { return false }
        if category == NotificationCategory.deciding.rawValue { return date < now.addingTimeInterval(-decidingLifetime) }
        return date < now.addingTimeInterval(-unreadGrace)
    }

    /// The delivered notifications about item `id` (by their identifier or the item they name), but `keeping`.
    static func about(_ id: String, delivered: [(identifier: String, item: String?)], keeping: String? = nil) -> [String] {
        delivered.filter { ($0.identifier == id || $0.item == id) && $0.identifier != keeping }.map(\.identifier)
    }
}

/// Which delivered Circle notifications have nothing left to offer.
enum CircleSweep {
    /// A review's notification once the review is known settled (`settled`), or is not among those waiting a minute
    /// after it came (the app may not have read it yet). A guardianship's notice is never swept.
    static func isSettled(info: [AnyHashable: Any], date: Date, waiting: Set<String>, settled: Set<String>, now: Date) -> Bool {
        guard let raw = info[NotificationKey.id] as? String, let push = CirclePush(raw), push.wake == .review else { return false }
        if settled.contains(push.id) { return true }
        return !waiting.contains(push.id) && date < now.addingTimeInterval(-NotificationSweep.unreadGrace)
    }
}

/// What the notification extension leaves to the app.
enum ExtensionPolicy {
    /// Any connection in a mode that judges with the model (Manual, Auto, Overnight), with the model there to run. Only
    /// the app runs it, so the extension handles such a push with `handlePushDeferringAutopilot`: Bypass and Lockdown
    /// still decide here (they need no model, so a bypass answers while the app is closed), and what Manual, Auto or
    /// Overnight would judge is left for the app; handling it fully here would settle it as "could not judge" for good
    /// (or load the model in a process that cannot hold it).
    static func needsModel(_ s: AutopilotSettings) -> Bool {
        guard s.model.state == .installed else { return false }
        let modes = [s.mode] + s.connections.map(\.mode)
        return modes.contains { $0 == .manual || $0 == .auto || $0 == .overnight }
    }
}
