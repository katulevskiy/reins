import SwiftUI

/// Several requests from one AI at once (an agent run): `quick` can be approved together without opening each, `all` is
/// everything of it that waits. Only a burst with at least two routine requests is offered (the Android app's Burst).
/// Bursts of several AIs are one (`ais` > 1, `connectionId` empty): their routine requests together, and "Deny" takes
/// back only those.
struct Burst: Equatable {
    var connectionId: String
    var label: String
    var quick: [String]
    var all: [String]
    var ais = 1

    /// The one bar Activity shows above the requests: one AI's burst, or the bursts of several folded together.
    static func bar(_ pending: [PendingItem]) -> Burst? {
        let bursts = of(pending)
        guard bursts.count > 1 else { return bursts.first }
        let quick = bursts.flatMap(\.quick)
        return Burst(connectionId: "", label: names(bursts.map(\.label)), quick: quick, all: quick, ais: bursts.count)
    }

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

    /// Says who asks: one AI's name, or for several their names.
    var title: String { ais > 1 ? "\(quick.count) routine · \(untrusted(label))" : "\(untrusted(label)) · \(all.count) waiting" }

    /// What stays in the list for a closer look, or nil.
    var heldNote: String? {
        let held = all.count - quick.count
        if held <= 0 { return nil }
        return held == 1 ? "1 needs a closer look" : "\(held) need a closer look"
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

/// Several requests at once, in one compact row above the list: approve the routine ones together (one Face ID, Touch
/// ID or passcode check), or deny them. One AI's bar denies everything it asked; a bar for several AIs only their
/// routine requests. What is asked every time stays in the list below and is opened one by one.
struct BurstBar: View {
    var burst: Burst
    @Environment(AppModel.self) private var model
    @State private var busy = false

    var body: some View {
        HStack(spacing: 8) {
            if burst.ais == 1 { ConnectionIcon(connectionId: burst.connectionId, label: burst.label, size: 22) }
            VStack(alignment: .leading, spacing: 1) {
                Text(burst.title).font(RFont.sans(14.5, .semibold)).foregroundStyle(Palette.text).lineLimit(1)
                if let note = burst.heldNote {
                    Text(note).font(RFont.sans(12.5)).foregroundStyle(Palette.secondary).lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button(burst.ais > 1 ? "Deny" : "Deny all") { run(approve: false) }
                .font(RFont.sans(14.5, .semibold))
                .foregroundStyle(Palette.secondary)
                .buttonStyle(.plain)
                .accessibilityIdentifier("denyAll")
            Button { run(approve: true) } label: {
                Text("Approve \(burst.quick.count)").font(RFont.sans(14.5, .semibold)).foregroundStyle(.white)
            }
            .buttonStyle(.glassProminent)
            .tint(Palette.accent)
            .accessibilityIdentifier("approveAll")
        }
        .disabled(busy)
        .padding(.leading, 14)
        .padding(.trailing, 10)
        .padding(.vertical, 10)
        .background(Palette.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("burst")
    }

    private func run(approve: Bool) {
        busy = true
        Task {
            await burst.answer(model, approve: approve)
            busy = false
        }
    }
}
