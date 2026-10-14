import SwiftUI

/// Several requests from one AI at once (an agent run): `quick` can be approved together without opening each, `all` is
/// everything of it that waits. Only a burst with at least two routine requests is offered (the Android app's Burst).
/// On Activity a burst's routine requests are one waiting card; what needs a closer look keeps its own card.
struct Burst: Equatable {
    var connectionId: String
    var label: String
    var quick: [String]
    var all: [String]

    /// "Claude", "Claude and Codex", "Claude, Codex and 2 more".
    static func names(_ labels: [String]) -> String {
        switch labels.count {
        case 0: ""
        case 1: labels[0]
        case 2: "\(labels[0]) and \(labels[1])"
        default: "\(labels[0]), \(labels[1]) and \(labels.count - 2) more"
        }
    }

    static func of(_ pending: [PendingItem]) -> [Burst] {
        var order: [String] = []
        var groups: [String: [PendingItem]] = [:]
        for item in pending where item.kind == .request {
            if groups[item.connectionId] == nil { order.append(item.connectionId) }
            groups[item.connectionId, default: []].append(item)
        }
        return order.compactMap { connection in
            let items = groups[connection] ?? []
            let quick = items.filter(\.quick).map(\.id)
            guard quick.count >= 2, let first = items.first else { return nil }
            return Burst(connectionId: connection, label: first.connectionLabel, quick: quick, all: items.map(\.id))
        }
    }

    /// Approves `ids` (one Face ID, Touch ID or passcode check for the lot, then each as its sheet would approve it
    /// untouched: once, no standing grant), or denies them (no check: it only takes something away).
    @MainActor
    static func answer(_ app: AppModel, ids: [String], label: String, approve: Bool) async {
        guard !ids.isEmpty else { return }
        if approve {
            let what = ids.count == 1 ? "the request" : "\(ids.count) requests"
            switch await OwnerCheck.confirm(app, reason: "Approve \(what) from \(untrusted(label))") {
            case .confirmed: break
            case .cancelled: return
            case .unavailable:
                app.feedback.play(.error)
                app.notice = OwnerCheck.unavailableMessage
                return
            }
        }
        app.feedback.play(approve ? .approved : .denied)
        // The cards go now; the answers go out in the background, and any that do not come back with a few words.
        let core = app.core
        for id in ids {
            app.answerInBackground(id, failed: approve ? "Not approved" : "Not denied") {
                if approve {
                    try await core.approveQuick(requestId: id)
                } else {
                    try await core.deny(requestId: id)
                }
            }
        }
    }

    /// "Approve all": the routine requests. "Deny all": everything of that AI.
    @MainActor
    func answer(_ app: AppModel, approve: Bool) async {
        await Self.answer(app, ids: approve ? quick : all, label: label, approve: approve)
    }
}

/// What a waiting card offers besides Review, from Autopilot's suggestion (pure, unit-tested).
enum WaitAdvice: Equatable {
    /// Autopilot would approve and it can be approved untouched: one tap, the owner's check, approved once.
    case allow
    /// Autopilot would deny: one tap, no check.
    case deny
    /// Autopilot looked but is not sure: its approve likelihood as a small ring.
    case unsure(Double)
    /// Asked every time (the hard floor, or nothing in it can be approved untouched).
    case alwaysAsks
    /// Nothing to suggest (no model, a pairing, an upload): Review only.
    case review

    static func of(_ item: PendingItem, _ suggestion: SuggestionView?) -> WaitAdvice {
        guard item.kind == .request, let s = suggestion else { return .review }
        if s.floor { return .alwaysAsks }
        if s.judged && s.verdict == .deny { return .deny }
        if s.judged && s.verdict == .approve { return item.quick ? .allow : .alwaysAsks }
        return .unsure(Double(s.pApprove))
    }

    /// A group's advice: one tap only when Autopilot says the same of every request in it.
    static func of(_ items: [PendingItem], _ suggestions: [String: SuggestionView]) -> WaitAdvice {
        let each = items.map { of($0, suggestions[$0.id]) }
        guard let first = each.first else { return .review }
        if each.allSatisfy({ $0 == first }) { return first }
        if each.contains(.alwaysAsks) { return .alwaysAsks }
        if each.allSatisfy({ $0 == .review }) { return .review }
        let p = items.compactMap { suggestions[$0.id].map { Double($0.pApprove) } }.min() ?? 0
        return .unsure(p)
    }
}

/// One waiting card: a single request, pairing, join or upload, or a burst's routine requests together.
struct WaitGroup: Identifiable, Equatable {
    var items: [PendingItem]

    var id: String { items.count > 1 ? "burst:\(items[0].connectionId)" : items[0].id }
    var first: PendingItem { items[0] }
    var isBurst: Bool { items.count > 1 }

    /// The waiting list as cards: each burst's routine requests fold into one card where the first of them was;
    /// everything else keeps its own.
    static func of(_ pending: [PendingItem]) -> [WaitGroup] {
        let bursts = Burst.of(pending)
        var folded: [String: Burst] = [:]
        for burst in bursts { for id in burst.quick { folded[id] = burst } }
        var placed: Set<String> = []
        var out: [WaitGroup] = []
        for item in pending {
            guard let burst = folded[item.id] else {
                out.append(WaitGroup(items: [item]))
                continue
            }
            guard placed.insert(burst.connectionId).inserted else { continue }
            let ids = Set(burst.quick)
            out.append(WaitGroup(items: pending.filter { ids.contains($0.id) }))
        }
        return out
    }
}
