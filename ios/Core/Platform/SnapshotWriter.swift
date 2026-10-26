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
        case .join: .join
        }
        let title = AskLabels.listTitle(p)
        return Snapshot.Item(
            id: p.id,
            kind: kind,
            title: title,
            subtitle: untrusted(p.subtitle),
            connection: untrusted(p.connectionLabel),
            service: p.service,
            createdAt: p.createdAt,
            expiresAt: p.waitUntil ?? (p.createdAt + 600),
            suggestion: p.suggestion,
            quick: p.kind == .request && p.quick
        )
    }

    static func entry(_ e: ActivityEntry) -> Snapshot.Entry {
        Snapshot.Entry(
            id: e.id,
            title: entryTitle(label: e.connectionLabel, action: e.action, count: Int(e.count), service: e.service, outcome: e.outcome, title: e.opTitle, op: e.op),
            outcome: e.outcome.capitalized,
            approved: !["denied", "failed", "expired"].contains(e.outcome),
            at: e.at,
            service: e.service,
            action: e.action
        )
    }

    static func modeKey(_ m: AutopilotMode) -> String {
        switch m {
        case .manual: "manual"
        case .auto: "auto"
        case .overnight: "overnight"
        case .bypass: "bypass"
        case .lockdown: "lockdown"
        }
    }

    /// Re-reads what widgets show and saves it; keeps the last values of anything that cannot be read.
    static func update(from core: any ReinsCoreProtocol) async {
        // A read that failed says nothing about the session: what the widgets show stays.
        let read: SessionInfo?
        do { read = try await core.session() } catch { return }
        guard let session = read else { Snapshot().save(); return }
        let owner = Snapshot.owner(server: session.serverUrl, email: session.email)
        let stored = Snapshot.load()
        var s = stored
        if s.accountFingerprint != owner { s = Snapshot() }
        s.accountFingerprint = owner
        s.signedIn = true
        if let pending = try? await core.pending() {
            // The connections (and their icons) are a network call away: keep the icons the app last wrote.
            let icons = Dictionary(s.pending.map { ($0.connection, ($0.connectionIcon, $0.connectionBlobatar)) }, uniquingKeysWith: { a, _ in a })
            s.pending = pending.map { p in
                var i = item(p)
                i.connectionIcon = icons[i.connection]?.0 ?? nil
                i.connectionBlobatar = icons[i.connection]?.1 ?? nil
                return i
            }
        }
        if let activity = try? await core.activity(limit: 6) { s.latest = activity.map(entry) }
        if let a = try? await core.autopilotSettings() {
            s.autopilotMode = modeKey(a.mode)
            s.bypassUntil = a.bypassUntil
            s.baseMode = modeKey(a.baseMode)
            s.anyBypassUntil = a.lastBypassEnd
            s.overnightUntil = a.overnightUntil
        }
        if let grants = try? await core.grants() { s.activeGrants = grants.filter(\.active).count }
        guard (try? await core.session()) == session else { return }
        // Only what the widgets show wakes them: nothing to save or reload when it is all as stored.
        s.updatedAt = stored.updatedAt
        guard s != stored else { return }
        s.updatedAt = Int64(Date().timeIntervalSince1970)
        s.save()
        WidgetReloads.shared.request()
    }
}

extension AutopilotSettings {
    /// When the last running bypass ends (the global one or any connection's), nil when none runs.
    func lastBypassEnd(now: Int64 = Int64(Date().timeIntervalSince1970)) -> Int64? {
        ([bypassUntil] + connections.map(\.bypassUntil)).compactMap { $0 }.filter { $0 > now }.max()
    }

    var lastBypassEnd: Int64? { lastBypassEnd() }
}
