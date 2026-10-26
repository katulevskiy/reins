import Foundation
import SwiftUI

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

    /// Who asks, as the who line shows it beside the avatar: one line, at most 28 characters (untrusted text).
    static func who(_ connection: String) -> String { untrustedName(connection, limit: 28) }

    /// The verb of a label ("GitHub · Push" -> "Push"), lowercased; nil without one.
    static func verb(_ label: String?) -> String? {
        guard let label, !label.isEmpty else { return nil }
        return (label.components(separatedBy: " · ").last ?? label).lowercased()
    }

    /// The glyph on a request icon's badge: the action's (from a label's verb or the core's action name), else the
    /// field's own.
    static func actionGlyph(_ verb: String?, field: FieldKind?) -> Glyph {
        let v = (verb ?? "").lowercased()
        let table: [(keys: [String], glyph: Glyph)] = [
            (["push", "commit", "merge", "branch"], .gitBranch),
            (["send", "reply", "forward", "email", "message"], .send),
            (["search", "find"], .search),
            (["read", "get", "list", "view", "open", "fetch"], .eye),
            (["delete", "remove", "trash", "archive"], .trash),
            (["create", "add", "new"], .plus),
            (["edit", "update", "change", "rename", "label"], .pencil),
            (["run", "exec", "command", "shell"], .play),
            (["connect", "pair", "connection"], .link),
            (["phone"], .phone),
            (["file", "upload"], .file),
            (["secret", "password", "vault"], .vault),
            (["pay", "purchase", "buy"], .card),
        ]
        for row in table where row.keys.contains(where: { v.contains($0) }) { return row.glyph }
        return field?.glyph ?? .help
    }

    /// The field an activity entry's badge is painted in, from its glyph: writes and sends push, reads read.
    static func entryField(_ action: String?) -> FieldKind {
        switch actionGlyph(action, field: nil) {
        case .eye, .search: .read
        case .trash: .delete
        case .link: .pair
        case .phone: .accounts
        case .vault: .grant
        default: .push
        }
    }

    /// An item's badge glyph.
    static func actionGlyph(_ item: Snapshot.Item) -> Glyph {
        switch item.kind {
        case .pairing: return .link
        case .join: return .phone
        case .blob: return .file
        case .request: return actionGlyph(verb(item.label), field: field(item))
        }
    }

    /// "now", "2m", "25m", "2h", "3d": how long ago, for a widget row.
    static func shortAgo(_ at: Int64, now: Int64) -> String {
        let s = max(0, now - at)
        switch s {
        case ..<60: return "now"
        case ..<3_600: return "\(s / 60)m"
        case ..<86_400: return "\(s / 3_600)h"
        default: return "\(s / 86_400)d"
        }
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

    /// The moments a widget has to redraw by itself: now, and whenever an answer window closes or a bypass ends (one
    /// that runs until turned off never does). Everything else (a new request, an answer) comes with a fresh
    /// snapshot, and the app reloads the timelines.
    static func timelineDates(_ s: Snapshot, now: Int64, limit: Int = 40) -> [Int64] {
        var later = Set(s.pending.map(\.expiresAt).filter { $0 > now })
        for b in [s.bypassUntil, s.anyBypassUntil, s.overnightUntil].compactMap({ $0 }) where b > now && !AutopilotText.isUntilOff(b) {
            later.insert(b)
        }
        return [now] + later.sorted().prefix(limit)
    }

    // MARK: Autopilot

    enum Mode: String, CaseIterable {
        case manual, auto, overnight, bypass, lockdown

        var name: String {
            switch self {
            case .manual: "Manual"
            case .auto: "Auto"
            case .overnight: AutopilotText.overnightName
            case .bypass: "Bypass"
            case .lockdown: "Lockdown"
            }
        }

        /// What the mode does, in one line (the Android app's `AutopilotText.line`).
        var line: String {
            switch self {
            case .manual: "Waits for you, with a suggestion"
            case .auto: "Decides what it is sure of, asks the rest"
            case .overnight: "Decides like Auto, holds what is too risky"
            case .bypass: "Approves everything, for a while"
            case .lockdown: "Denies everything at once"
            }
        }
    }

    /// The global mode in force at `now`: a bypass that ran out before the next snapshot shows the mode it went back to;
    /// an Overnight that ran out, the mode under it when the snapshot knew it, else Manual (the core's default).
    static func mode(_ s: Snapshot, now: Int64) -> Mode {
        let mode = Mode(rawValue: s.autopilotMode) ?? .manual
        if mode == .bypass, (s.bypassUntil ?? 0) <= now {
            let base = Mode(rawValue: s.baseMode ?? "") ?? .manual
            if base == .overnight, let end = s.overnightUntil, end <= now { return .manual }
            return base
        }
        if mode == .overnight, let end = s.overnightUntil, end <= now {
            return .manual
        }
        return mode
    }

    /// When the last running bypass (global or a connection's) ends, nil when none runs at `now`.
    static func bypassEnd(_ s: Snapshot, now: Int64) -> Int64? {
        let end = max(s.anyBypassUntil ?? 0, s.bypassUntil ?? 0)
        return end > now ? end : nil
    }

    /// A bypass that runs until the user turns it off: no countdown, "until you stop it".
    static func bypassUntilOff(_ s: Snapshot, now: Int64) -> Bool {
        bypassEnd(s, now: now).map(AutopilotText.isUntilOff) ?? false
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
            connectionBlobatar: item.connectionBlobatar,
            quick: item.quick,
            field: field(item).rawValue,
            label: label(item)
        )
    }

    /// An item's field: the one the app picked from the core's kind; for a snapshot from before Signal, its kind's.
    static func field(_ item: Snapshot.Item) -> FieldKind {
        field(item.field, kind: item.kind)
    }

    static func field(_ raw: String?, kind: Snapshot.Item.Kind) -> FieldKind {
        if let raw, let field = FieldKind(rawValue: raw) { return field }
        switch kind {
        case .pairing: return .pair
        case .join: return .accounts
        case .blob, .request: return .push
        }
    }

    /// The label line over the ask ("GitHub · Push"), or the kind's word when the snapshot has none.
    static func label(_ item: Snapshot.Item) -> String {
        if let label = item.label, !label.isEmpty { return label }
        switch item.kind {
        case .pairing: return "New connection"
        case .join: return "Account · New phone"
        case .blob: return "File"
        case .request: return item.service.isEmpty ? "Request" : serviceName(item.service)
        }
    }

    /// The length (seconds) a bypass with `left` seconds to go was most likely given (`AutopilotText.bypassLength`),
    /// when its start is not known. The countdown ring starts from there.
    static func bypassLength(left: Int64) -> Int64 { AutopilotText.bypassLength(leftSeconds: left) }

    /// "every AI", "Claude", "3 AIs": whose requests a bypass approves.
    static func bypassScope(global: Bool, connections: [String]) -> String {
        if global { return "every AI" }
        if connections.count == 1, let one = connections.first { return one }
        return "\(connections.count) AIs"
    }

    /// The Live Activity's state for a running bypass, nil when none runs. `previous` is what the activity shows now:
    /// the same end keeps its start (so the ring does not jump), a new or restarted bypass starts a new ring. `since`
    /// is when the bypass that ends last started, when known. One that runs until turned off has no end to count down
    /// to: its state says so (`untilOff`), and `until` is a date far off rather than the core's `i64::MAX`.
    /// `overnight` makes it an Overnight's state (the same activity, in Overnight's look and with its Stop); a previous
    /// state of the other kind starts a new ring.
    static func bypassState(
        global: Int64?,
        connections: [(label: String, until: Int64?)],
        previous: BypassActivityAttributes.ContentState?,
        approvedSince: (Date) -> Int,
        now: Int64,
        since: Int64? = nil,
        overnight: Bool = false
    ) -> BypassActivityAttributes.ContentState? {
        let globalUntil = global.flatMap { $0 > now ? $0 : nil }
        let running = connections.filter { ($0.until ?? 0) > now }
        guard let end = ([globalUntil] + running.map(\.until)).compactMap({ $0 }).max() else { return nil }
        let untilOff = AutopilotText.isUntilOff(end)
        let until = untilOff ? Date.distantFuture : Date(timeIntervalSince1970: TimeInterval(end))
        let startedAt: Date
        if let previous, previous.until == until, (previous.untilOff ?? false) == untilOff, (previous.overnight ?? false) == overnight {
            startedAt = previous.startedAt
        } else if let since, since <= now {
            startedAt = Date(timeIntervalSince1970: TimeInterval(since))
        } else if untilOff {
            startedAt = Date(timeIntervalSince1970: TimeInterval(now))
        } else {
            startedAt = Date(timeIntervalSince1970: TimeInterval(end - bypassLength(left: end - now)))
        }
        return BypassActivityAttributes.ContentState(
            until: until,
            startedAt: startedAt,
            scope: bypassScope(global: globalUntil != nil, connections: running.map(\.label)),
            approvedCount: approvedSince(startedAt),
            untilOff: untilOff ? true : nil,
            overnight: overnight ? true : nil
        )
    }

    /// "126 / 370 MB".
    static func downloadLine(_ s: ModelDownloadActivityAttributes.ContentState) -> String {
        if s.failed { return "The download stopped" }
        if s.finished { return "Autopilot can judge now" }
        guard s.total > 0 else { return "Starting" }
        let mb = { (bytes: Int64) in Int((Double(bytes) / 1_000_000).rounded()) }
        return "\(mb(s.downloaded)) / \(mb(s.total)) MB"
    }

    /// What the widget gallery and WidgetKit's placeholders show: two requests, a pairing, a little history.
    static func sample(now: Int64) -> Snapshot {
        var s = Snapshot()
        s.signedIn = true
        s.approvalDevice = true
        s.pending = [
            Snapshot.Item(id: "s1", kind: .request, title: "Claude Code: Push 8 commits to octo/app", subtitle: "feature/passkeys", connection: "Claude Code",
                          service: "github", createdAt: now - 20, expiresAt: now + 100, suggestion: nil, connectionIcon: "claude", field: "push", label: "GitHub · Push"),
            Snapshot.Item(id: "s2", kind: .request, title: "My ChatGPT: Email Ann", subtitle: "work@corp.example",
                          connection: "My ChatGPT", service: "gmail", createdAt: now - 60, expiresAt: now + 240, suggestion: nil,
                          connectionIcon: "openai", field: "push", label: "Gmail · Send"),
            Snapshot.Item(id: "s3", kind: .pairing, title: "Connect Gemini to Reins?", subtitle: "gemini.google.com",
                          connection: "Gemini", service: "", createdAt: now - 90, expiresAt: now + 510, suggestion: nil),
        ]
        s.latest = [
            Snapshot.Entry(id: 3, title: "Claude read 3 emails", outcome: "Released", approved: true, at: now - 120, service: "gmail", action: "read"),
            Snapshot.Entry(id: 2, title: "Cursor pushed to octo-cat/app", outcome: "Sent", approved: true, at: now - 1_500, service: "github", action: "push"),
            Snapshot.Entry(id: 1, title: "notes-bot searched email", outcome: "Denied", approved: false, at: now - 7_200, service: "gmail", action: "search"),
        ]
        s.autopilotMode = "manual"
        s.baseMode = "manual"
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

/// How the Control Center controls look: SF Symbols (controls take only symbols) as close as there are to the glyphs
/// the boards draw, the state line under the title, and a tint only while the control's state is on.
enum ControlLook {
    /// Lockdown's circle with a slash.
    static let lockdownSymbol = "nosign"
    static let waitingSymbol = "tray"
    /// Bypass's shield with a mark.
    static let bypassSymbol = "exclamationmark.shield"

    static func modeSymbol(_ mode: Glance.Mode) -> String {
        switch mode {
        case .manual: "person"
        case .auto: "bolt.fill"
        case .overnight: "moon"
        case .bypass: bypassSymbol
        case .lockdown: lockdownSymbol
        }
    }

    /// The mode control's tint: none for the calm modes (Manual, Auto), the mode's field colour for the others.
    static func modeTint(_ mode: Glance.Mode) -> Color? {
        switch mode {
        case .manual, .auto: nil
        case .overnight: Palette.modeOvernight
        case .bypass: Palette.fieldDelete
        case .lockdown: Palette.fieldWarn
        }
    }

    /// The Stop bypass control's state line: "42 min left", "until you stop it", or "Off" with none running.
    static func stopState(_ s: Snapshot, now: Int64) -> String {
        guard let end = Glance.bypassEnd(s, now: now) else { return "Off" }
        return AutopilotText.timeLeft(until: end, now: now)
    }
}
