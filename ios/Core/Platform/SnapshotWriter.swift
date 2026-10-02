import Foundation
import WidgetKit

/// Writes the widget snapshot from a core, for processes without an `AppModel` (the notification extension after it
/// handled a push). Same rows as `AppModel.publish()`: keep the two in step.
enum SnapshotWriter {
    static func item(_ p: PendingItem) -> Snapshot.Item {
        let kind: Snapshot.Item.Kind = switch p.kind {
        case .request: .request
        case .pairing: .pairing
        case .blob: .blob
        }
        let title: String = switch p.kind {
        case .pairing: untrusted(p.title)
        case .blob: "\(untrusted(p.connectionLabel)): Share a file"
        case .request: fullTitle(label: p.connectionLabel, action: p.action, count: Int(p.count), service: p.service, title: p.opTitle, op: p.op)
        }
        return Snapshot.Item(
            id: p.id,
            kind: kind,
            title: title,
            subtitle: untrusted(p.subtitle),
            connection: untrusted(p.connectionLabel),
            service: p.service,
            createdAt: p.createdAt,
            expiresAt: p.waitUntil ?? (p.createdAt + 600),
            suggestion: p.suggestion
        )
    }

    static func entry(_ e: ActivityEntry) -> Snapshot.Entry {
        Snapshot.Entry(
            id: e.id,
            title: entryTitle(label: e.connectionLabel, action: e.action, count: Int(e.count), service: e.service, outcome: e.outcome, title: e.opTitle, op: e.op),
            outcome: e.outcome.capitalized,
            approved: !["denied", "failed", "expired"].contains(e.outcome),
            at: e.at
        )
    }

    static func modeKey(_ m: AutopilotMode) -> String {
        switch m {
        case .manual: "manual"
        case .assisted: "assisted"
        case .auto: "auto"
        case .bypass: "bypass"
        case .lockdown: "lockdown"
        }
    }

    /// Re-reads what widgets show and saves it; keeps the last values of anything that cannot be read.
    static func update(from core: any RewardenCoreProtocol) async {
        var s = Snapshot.load()
        s.signedIn = await core.session() != nil
        if let pending = try? await core.pending() {
            // The connections (and their logo picks) are a network call away: keep the picks the app last wrote.
            let icons = Dictionary(s.pending.map { ($0.connection, $0.connectionIcon) }, uniquingKeysWith: { a, _ in a })
            s.pending = pending.map { p in
                var i = item(p)
                i.connectionIcon = icons[i.connection] ?? nil
                return i
            }
        }
        if let activity = try? await core.activity(limit: 6) { s.latest = activity.map(entry) }
        if let a = try? await core.autopilotSettings() {
            s.autopilotMode = modeKey(a.mode)
            s.bypassUntil = a.bypassUntil
            s.baseMode = modeKey(a.baseMode)
            s.anyBypassUntil = a.lastBypassEnd
        }
        if let grants = try? await core.grants() { s.activeGrants = grants.filter(\.active).count }
        s.updatedAt = Int64(Date().timeIntervalSince1970)
        s.save()
        WidgetCenter.shared.reloadAllTimelines()
        ControlCenter.shared.reloadAllControls()
    }
}

extension AutopilotSettings {
    /// When the last running bypass ends (the global one or any connection's), nil when none runs.
    func lastBypassEnd(now: Int64 = Int64(Date().timeIntervalSince1970)) -> Int64? {
        ([bypassUntil] + connections.map(\.bypassUntil)).compactMap { $0 }.filter { $0 > now }.max()
    }

    var lastBypassEnd: Int64? { lastBypassEnd() }
}
