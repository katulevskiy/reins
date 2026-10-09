import SwiftUI

/// Several requests from one AI at once (an agent run): `quick` can be approved together without opening each, `all` is
/// everything of it that waits. Only a burst with at least two routine requests is offered (the Android app's Burst).
struct Burst: Equatable {
    var connectionId: String
    var label: String
    var quick: [String]
    var all: [String]

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

    var title: String { "\(untrusted(label)) asked \(all.count) times" }

    /// What stays in the list for a closer look, or nil.
    var heldNote: String? {
        let held = all.count - quick.count
        if held <= 0 { return nil }
        return held == 1 ? "1 of them needs a closer look and stays in the list." : "\(held) of them need a closer look and stay in the list."
    }

    /// "Approve all" (one Face ID, Touch ID or passcode check for the lot, then each routine request as its sheet
    /// would approve it untouched; anything asked every time keeps waiting) or "Deny all" (every request of that AI).
    @MainActor
    func answer(_ app: AppModel, approve: Bool) async {
        let ids = approve ? quick : all
        if approve {
            switch await OwnerCheck.confirm(app, reason: "Approve \(ids.count) requests from \(untrusted(label))") {
            case .confirmed: break
            case .cancelled: return
            case .unavailable:
                app.feedback.play(.error)
                app.notice = OwnerCheck.unavailableMessage
                return
            }
        }
        app.feedback.play(approve ? .approved : .denied)
        var failed = 0
        var reason: String?
        for id in ids {
            do {
                if approve {
                    try await app.core.approveQuick(requestId: id)
                } else {
                    try await app.core.deny(requestId: id)
                }
                app.dropPending(id)
            } catch {
                failed += 1
                reason = reason ?? decisionErrorMessage(error)
            }
        }
        app.refreshPendingSoon()
        if failed > 0 {
            app.feedback.play(.error)
            app.notice = "\(failed) of \(ids.count) could not be \(approve ? "approved" : "denied"): \(reason ?? "")"
        }
    }
}

/// Approve the routine requests of one AI together, or deny everything it asked. What is asked every time stays in the
/// list below and is opened one by one.
struct BurstBar: View {
    var burst: Burst
    @Environment(AppModel.self) private var model
    @State private var busy = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 8) {
                ConnectionIcon(connectionId: burst.connectionId, label: burst.label, size: 22)
                Text(burst.title).font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.text).lineLimit(2)
            }
            if let note = burst.heldNote {
                Text(note).font(RFont.sans(13)).foregroundStyle(Palette.secondary).padding(.top, 4)
            }
            HStack(spacing: 10) {
                Button { run(approve: false) } label: {
                    Text("Deny all").font(RFont.sans(15, .semibold)).foregroundStyle(Palette.text).frame(maxWidth: .infinity, minHeight: 32)
                }
                .buttonStyle(.glass)
                .accessibilityIdentifier("denyAll")
                Button { run(approve: true) } label: {
                    Text("Approve \(burst.quick.count)").font(RFont.sans(15, .semibold)).foregroundStyle(.white).frame(maxWidth: .infinity, minHeight: 32)
                }
                .buttonStyle(.glassProminent)
                .tint(Palette.accent)
                .accessibilityIdentifier("approveAll")
            }
            .disabled(busy)
            .padding(.top, 10)
        }
        .padding(14)
        .background(Palette.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("burst:\(burst.connectionId)")
    }

    private func run(approve: Bool) {
        busy = true
        Task {
            await burst.answer(model, approve: approve)
            busy = false
        }
    }
}
