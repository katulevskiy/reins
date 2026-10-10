import Foundation

/// What widgets, controls and Live Activities make of the snapshot, apart from the views so it can be tested. Every
/// function takes `now` (unix seconds): a timeline entry is drawn for a moment that is not the present.
enum Glance {
    static func now() -> Int64 { Int64(Date().timeIntervalSince1970) }

    // MARK: Waiting

    /// Items whose answer window is still open, newest first.
    static func waiting(_ items: [Snapshot.Item], now: Int64) -> [Snapshot.Item] {
        items.filter { $0.expiresAt > now }.sorted { ($0.createdAt, $0.id) > ($1.createdAt, $1.id) }
    }

    static func waiting(_ s: Snapshot, now: Int64) -> [Snapshot.Item] { waiting(s.pending, now: now) }

    /// The operation without the AI's name in front ("Claude: Send email to 2" -> "Send email to 2"); widgets show
    /// the AI on its own line or as an avatar.
    static func operation(_ item: Snapshot.Item) -> String {
        let prefix = item.connection + ": "
        if !item.connection.isEmpty, item.title.hasPrefix(prefix), item.title.count > prefix.count {
            return String(item.title.dropFirst(prefix.count))
        }
        return item.title
    }

    /// "Claude · me@gmail.com", or whichever of the two there is.
    static func byline(_ item: Snapshot.Item) -> String {
        [item.kind == .pairing || item.kind == .join ? "" : item.connection, item.subtitle].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// The first letter of the AI's name, for the round avatar.
    static func initial(_ label: String) -> String {
        String(label.trimmingCharacters(in: .whitespacesAndNewlines).prefix(1)).uppercased()
    }

    static func link(_ item: Snapshot.Item) -> URL { DeepLink.item(kind: item.kind, id: item.id).url }

    /// Where a tap on the "Waiting" widget goes: the newest item, or the app's home when nothing waits.
    static func waitingLink(_ s: Snapshot, now: Int64) -> URL {
        waiting(s, now: now).first.map(link) ?? DeepLink.home.url
    }

    /// "3 waiting", "1 waiting", "Nothing waits".
    static func countLine(_ n: Int) -> String { n == 0 ? "Nothing waits" : "\(n) waiting" }

    // MARK: Timelines

    /// The moments a widget has to redraw by itself: now, and whenever an answer window closes or a bypass ends.
    /// Everything else (a new request, an answer) comes with a fresh snapshot, and the app reloads the timelines.
    static func timelineDates(_ s: Snapshot, now: Int64, limit: Int = 40) -> [Int64] {
        var later = Set(s.pending.map(\.expiresAt).filter { $0 > now })
        for b in [s.bypassUntil, s.anyBypassUntil].compactMap({ $0 }) where b > now { later.insert(b) }
        return [now] + later.sorted().prefix(limit)
    }

    // MARK: Autopilot

    enum Mode: String, CaseIterable {
        case manual, assisted, auto, bypass, lockdown

        var name: String {
            switch self {
            case .manual: "Manual"
            case .assisted: "Assisted"
            case .auto: "Auto"
            case .bypass: "Bypass"
            case .lockdown: "Lockdown"
            }
        }

        /// What the mode does, in one line (the Android app's `AutopilotText.line`).
        var line: String {
            switch self {
            case .manual: "Every request waits for you"
            case .assisted: "Waits for you, with a suggestion"
            case .auto: "Decides what it is sure of, asks the rest"
            case .bypass: "Approves all but the riskiest, for a while"
            case .lockdown: "Denies everything at once"
            }
        }

        var symbol: String {
            switch self {
            case .manual: "hand.raised.fill"
            case .assisted: "sparkles"
            case .auto: "location.north.fill"
            case .bypass: "bolt.fill"
            case .lockdown: "lock.fill"
            }
        }
    }

    /// The global mode in force at `now`: a bypass that ran out before the next snapshot shows the mode it went back to.
    static func mode(_ s: Snapshot, now: Int64) -> Mode {
        let mode = Mode(rawValue: s.autopilotMode) ?? .manual
        if mode == .bypass, (s.bypassUntil ?? 0) <= now {
            return Mode(rawValue: s.baseMode ?? "") ?? .manual
        }
        return mode
    }

    /// When the last running bypass (global or a connection's) ends, nil when none runs at `now`.
    static func bypassEnd(_ s: Snapshot, now: Int64) -> Int64? {
        let end = max(s.anyBypassUntil ?? 0, s.bypassUntil ?? 0)
        return end > now ? end : nil
    }

    /// The bypass that runs is a connection's, not the global one: the widget says so under the global mode.
    static func connectionBypassOnly(_ s: Snapshot, now: Int64) -> Bool {
        bypassEnd(s, now: now) != nil && (s.bypassUntil ?? 0) <= now
    }

    // MARK: Activity

    /// "now", "5 min", "3 h", "2 d": how long ago an entry happened, for a line that has no room for more.
    static func ago(_ at: Int64, now: Int64) -> String {
        let s = max(0, now - at)
        switch s {
        case ..<60: return "now"
        case ..<3_600: return "\(s / 60) min"
        case ..<86_400: return "\(s / 3_600) h"
        default: return "\(s / 86_400) d"
        }
    }

    // MARK: Live Activities

    /// What the requests Live Activity shows: the newest waiting item and how many wait; nil when nothing does.
    static func approvalState(_ items: [Snapshot.Item], now: Int64) -> ApprovalActivityAttributes.ContentState? {
        let list = waiting(items, now: now)
        guard let item = list.first else { return nil }
        return ApprovalActivityAttributes.ContentState(
            itemId: item.id,
            kind: item.kind,
            title: operation(item),
            subtitle: item.subtitle,
            connection: item.connection,
            service: item.service,
            count: list.count,
            expiresAt: Date(timeIntervalSince1970: TimeInterval(item.expiresAt)),
            createdAt: Date(timeIntervalSince1970: TimeInterval(min(item.createdAt, item.expiresAt))),
            suggestion: item.suggestion,
            connectionIcon: item.connectionIcon,
            quick: item.quick
        )
    }

    /// The bypass lengths the app offers, in minutes (the core allows 1 to 60).
    static let bypassMinutes: [Int64] = [15, 30, 60]

    /// The length (seconds) a bypass with `left` seconds to go was most likely given: the shortest choice that fits
    /// (Android's `AutopilotText.bypassLength`). The countdown ring starts from there.
    static func bypassLength(left: Int64) -> Int64 {
        bypassMinutes.map { $0 * 60 }.first { left <= $0 } ?? (bypassMinutes.last ?? 60) * 60
    }

    /// "every AI", "Claude", "3 AIs": whose requests a bypass approves.
    static func bypassScope(global: Bool, connections: [String]) -> String {
        if global { return "every AI" }
        if connections.count == 1, let one = connections.first { return one }
        return "\(connections.count) AIs"
    }

    /// The Live Activity's state for a running bypass, nil when none runs. `previous` is what the activity shows now:
    /// the same end keeps its start (so the ring does not jump), a new or restarted bypass starts a new ring.
    static func bypassState(
        global: Int64?,
        connections: [(label: String, until: Int64?)],
        previous: BypassActivityAttributes.ContentState?,
        approvedSince: (Date) -> Int,
        now: Int64
    ) -> BypassActivityAttributes.ContentState? {
        let globalUntil = global.flatMap { $0 > now ? $0 : nil }
        let running = connections.filter { ($0.until ?? 0) > now }
        guard let end = ([globalUntil] + running.map(\.until)).compactMap({ $0 }).max() else { return nil }
        let until = Date(timeIntervalSince1970: TimeInterval(end))
        let startedAt: Date
        if let previous, previous.until == until {
            startedAt = previous.startedAt
        } else {
            startedAt = Date(timeIntervalSince1970: TimeInterval(end - bypassLength(left: end - now)))
        }
        return BypassActivityAttributes.ContentState(
            until: until,
            startedAt: startedAt,
            scope: bypassScope(global: globalUntil != nil, connections: running.map(\.label)),
            approvedCount: approvedSince(startedAt)
        )
    }

    /// "123 MB of 370 MB".
    static func downloadLine(_ s: ModelDownloadActivityAttributes.ContentState) -> String {
        if s.failed { return "The download stopped" }
        if s.finished { return "Assisted and Auto can decide now" }
        let f = ByteCountFormatter()
        f.countStyle = .file
        f.allowedUnits = [.useMB]
        return s.total > 0 ? "\(f.string(fromByteCount: s.downloaded)) of \(f.string(fromByteCount: s.total))" : "Starting"
    }

    /// What the widget gallery and WidgetKit's placeholders show: two requests, a pairing, a little history.
    static func sample(now: Int64) -> Snapshot {
        var s = Snapshot()
        s.signedIn = true
        s.approvalDevice = true
        s.pending = [
            Snapshot.Item(id: "s1", kind: .request, title: "Claude: Search email", subtitle: "me@gmail.com", connection: "Claude",
                          service: "gmail", createdAt: now - 20, expiresAt: now + 100, suggestion: nil),
            Snapshot.Item(id: "s2", kind: .request, title: "My ChatGPT: Send email to Ann", subtitle: "work@corp.example",
                          connection: "My ChatGPT", service: "gmail", createdAt: now - 60, expiresAt: now + 240, suggestion: nil),
            Snapshot.Item(id: "s3", kind: .pairing, title: "Connect Gemini to Reins?", subtitle: "gemini.google.com",
                          connection: "Gemini", service: "", createdAt: now - 90, expiresAt: now + 510, suggestion: nil),
        ]
        s.latest = [
            Snapshot.Entry(id: 3, title: "Claude: Read 3 emails", outcome: "Released", approved: true, at: now - 120),
            Snapshot.Entry(id: 2, title: "Cursor: Commit to octo-cat/app", outcome: "Sent", approved: true, at: now - 1_500),
            Snapshot.Entry(id: 1, title: "notes-bot: Search email", outcome: "Denied", approved: false, at: now - 7_200),
        ]
        s.autopilotMode = "assisted"
        s.baseMode = "assisted"
        s.activeGrants = 2
        s.updatedAt = now
        return s
    }

    static func downloadFraction(_ s: ModelDownloadActivityAttributes.ContentState) -> Double {
        if s.finished { return 1 }
        guard s.total > 0 else { return 0 }
        return min(1, max(0, Double(s.downloaded) / Double(s.total)))
    }
}
