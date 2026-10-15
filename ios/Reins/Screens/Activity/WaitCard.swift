import SwiftUI

/// A waiting request in one sentence, from the AI's side: "Wants to **read 2 emails**", "Wants to **search Gmail** for
/// **“invoice”**". Pieces marked `strong` are the key nouns (pure, unit-tested).
enum WantsText {
    struct Piece: Equatable {
        var text: String
        var strong = false
    }

    static func lowerFirst(_ s: String) -> String { s.prefix(1).lowercased() + s.dropFirst() }

    /// What the request does, in a few words: "read 2 emails", "send an email", "push".
    static func verb(action: String, count: Int, service: String, opTitle: String, op: String) -> String {
        let mail = service.isEmpty || service == "gmail"
        if mail && opTitle.isEmpty {
            switch ActionKind.of(action) {
            case .send: return count > 1 ? "send an email to \(count)" : "send an email"
            case .read where count == 1: return "read an email"
            default: break
            }
        }
        return lowerFirst(operationTitle(action: action, count: count, service: service, title: opTitle, op: op))
    }

    static func of(_ item: PendingItem) -> [Piece] {
        let label = untrusted(item.connectionLabel)
        let subtitle = untrusted(item.subtitle).trimmingCharacters(in: .whitespacesAndNewlines)
        switch item.kind {
        case .pairing: return [Piece(text: label, strong: true), Piece(text: " wants to connect")]
        case .join: return [Piece(text: label, strong: true), Piece(text: " asks to join your account")]
        case .blob:
            return [Piece(text: "Wants to "), Piece(text: "share a file", strong: true)]
                + (subtitle.isEmpty ? [] : [Piece(text: " · \(subtitle)")])
        case .request: break
        }
        var out = [Piece(text: "Wants to "), Piece(text: verb(action: item.action, count: Int(item.count), service: item.service, opTitle: item.opTitle, op: item.op), strong: true)]
        guard !subtitle.isEmpty else { return out }
        let mail = item.service.isEmpty || item.service == "gmail"
        switch ActionKind.of(item.action) {
        case .search: out += [Piece(text: " for "), Piece(text: "“\(subtitle)”", strong: true)]
        case .send: out += [Piece(text: ": "), Piece(text: "“\(subtitle)”", strong: true)]
        // "2 messages found" says again what the verb said; the accounts' count too.
        case .read where mail, .accounts: break
        case .grant: out += [Piece(text: " · \(subtitle)")]
        default: out += [Piece(text: " · "), Piece(text: subtitle, strong: true)]
        }
        return out
    }

    /// A burst's routine requests in one sentence: "Wants to **read 5 emails**", "Wants to **search Gmail** **3 times**",
    /// else "Wants to make **4 requests**".
    static func of(burst items: [PendingItem]) -> [Piece] {
        guard let first = items.first else { return [] }
        let same = items.allSatisfy { $0.action == first.action && $0.service == first.service && $0.op == first.op }
        guard same else { return [Piece(text: "Wants to make "), Piece(text: "\(items.count) requests", strong: true)] }
        let total = items.reduce(0) { $0 + Int($1.count) }
        let mail = first.service.isEmpty || first.service == "gmail"
        if ActionKind.of(first.action) == .read && mail {
            return [Piece(text: "Wants to "), Piece(text: verb(action: first.action, count: total, service: first.service, opTitle: first.opTitle, op: first.op), strong: true)]
        }
        return [
            Piece(text: "Wants to "),
            Piece(text: verb(action: first.action, count: Int(first.count), service: first.service, opTitle: first.opTitle, op: first.op), strong: true),
            Piece(text: " "),
            Piece(text: "\(items.count) times", strong: true),
        ]
    }

    /// The pieces as one text: key nouns in the text colour and semibold, the rest secondary.
    static func text(_ pieces: [Piece], size: CGFloat = 16) -> Text {
        var out = AttributedString()
        for piece in pieces {
            var run = AttributedString(piece.text)
            run.font = RFont.sans(size, piece.strong ? .semibold : .medium)
            run.foregroundColor = piece.strong ? Palette.ink : Palette.ink2
            out += run
        }
        return Text(out)
    }

    static func plain(_ pieces: [Piece]) -> String { pieces.map(\.text).joined() }
}

/// A request, connection, join or upload that waits, or a burst's routine requests together: tinted by kind like the
/// approval hero, the service with the AI's avatar, one sentence, the time left, and Autopilot's pick beside Review.
struct WaitCard: View {
    var group: WaitGroup
    var advice: WaitAdvice
    /// Floor requests get the red wash.
    var danger: Bool
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false

    private var item: PendingItem { group.first }

    var body: some View {
        let tint = KindTint.of(kind, danger: danger)
        let shape = RoundedRectangle(cornerRadius: 24, style: .continuous)
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .center, spacing: 12) {
                // What it touches with what it does, as its Activity row and its page will show it; requests the core
                // gave no label (pairings, other phones, uploads) keep the service tile.
                if let label = item.label {
                    PendingTile(item: item, label: label)
                } else {
                    ServiceMini(
                        service: item.kind == .request ? item.service : "",
                        symbol: kind.symbol,
                        connectionId: item.connectionId,
                        label: item.connectionLabel,
                        known: item.kind == .request || item.kind == .blob
                    )
                }
                Group {
                    if let label = item.label, !group.isBurst {
                        LabelText(label: label, size: 16, lines: 3)
                    } else {
                        WantsText.text(group.isBurst ? WantsText.of(burst: group.items) : WantsText.of(item))
                            .lineSpacing(1)
                            .lineLimit(3)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("pendingTitle:\(item.id)")
                TimerPill(createdAt: item.createdAt, waitUntil: item.kind == .blob ? nil : group.items.compactMap(\.waitUntil).min())
            }
            if group.isBurst { peek }
            actions
                .padding(.top, 12)
        }
        .padding(14)
        .background(KindTint.wash(tint), in: shape)
        .overlay(shape.strokeBorder(tint.opacity(0.24), lineWidth: 1))
        .contentShape(shape)
        // The card itself opens the full screen; its buttons answer.
        .onTapGesture(perform: review)
        .disabled(busy)
        .contextMenu {
            Button("Review", systemImage: "checkmark.shield", action: review)
            if group.isBurst {
                Button("Approve all", systemImage: BiometricSymbol.name) { answer(approve: true) }
                Button("Deny all", systemImage: "nosign") { answer(approve: false) }
            }
            Button("Copy", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = group.isBurst || item.label == nil
                    ? WantsText.plain(group.isBurst ? WantsText.of(burst: group.items) : WantsText.of(item))
                    : item.label.map(RequestLabelLook.plain)
                feedback.play(.copied)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(group.isBurst ? "burst" : "pending:\(item.id)")
    }

    private var kind: ActionKind {
        switch item.kind {
        case .pairing: .pair
        case .join: .join
        case .blob: .upload
        case .request: ActionKind.of(item.action)
        }
    }

    /// A burst lists what is in it, one line each.
    private var peek: some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(group.items.prefix(4), id: \.id) { i in
                HStack(spacing: 7) {
                    Image(systemName: ActionKind.of(i.action).symbol)
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(Palette.inkMeta)
                        .frame(width: 14)
                    Text(line(i))
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.ink2)
                        .lineLimit(1)
                }
            }
            if group.items.count > 4 {
                Text("and \(group.items.count - 4) more")
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.leading, 21)
            }
        }
        .padding(.top, 10)
        .padding(.leading, 62)
        .accessibilityElement(children: .combine)
    }

    private func line(_ i: PendingItem) -> String {
        let subtitle = untrusted(i.subtitle).trimmingCharacters(in: .whitespacesAndNewlines)
        return subtitle.isEmpty ? operationTitle(action: i.action, count: Int(i.count), service: i.service, title: i.opTitle, op: i.op) : subtitle
    }

    @ViewBuilder private var actions: some View {
        let n = group.items.count
        HStack(spacing: 8) {
            switch advice {
            case .allow:
                Button { answer(approve: true) } label: {
                    Label(n > 1 ? "Allow \(n)" : "Allow", systemImage: BiometricSymbol.name)
                }
                .buttonStyle(DockButtonStyle(kind: .success, height: 42, fontSize: 14.5))
                .accessibilityIdentifier(group.isBurst ? "approveAll" : "allow:\(item.id)")
                reviewButton
            case .deny:
                Button { answer(approve: false) } label: {
                    Label(n > 1 ? "Deny \(n)" : "Deny", systemImage: "nosign")
                }
                .buttonStyle(DockButtonStyle(kind: .danger, height: 42, fontSize: 14.5))
                .accessibilityIdentifier(group.isBurst ? "denyAll" : "deny:\(item.id)")
                reviewButton
            case let .unsure(p):
                ThirdsRow(spacing: 8) {
                    HStack(spacing: 7) {
                        ArcRing(fraction: p, color: Palette.kindRead, track: Palette.kindRead.opacity(0.2), lineWidth: 2.5)
                            .frame(width: 16, height: 16)
                        Text("Unsure")
                    }
                    .note()
                    .accessibilityIdentifier("unsure:\(item.id)")
                    reviewButton
                }
            case .alwaysAsks:
                ThirdsRow(spacing: 8) {
                    HStack(spacing: 6) {
                        Image(systemName: "lock").font(.system(size: 13, weight: .semibold))
                        Text("Always asks").lineLimit(1).minimumScaleFactor(0.8)
                    }
                    .note()
                    .accessibilityIdentifier("alwaysAsks:\(item.id)")
                    reviewButton
                }
            case .review:
                reviewButton
            }
        }
    }

    private var reviewButton: some View {
        Button("Review", action: review)
            .buttonStyle(DockButtonStyle(kind: .light, height: 42, fontSize: 14.5))
            .accessibilityIdentifier("review:\(item.id)")
    }

    private func review() {
        model.openSheet(item.sheetTarget)
        feedback.defaultTap()
    }

    private func answer(approve: Bool) {
        busy = true
        let ids = group.items.map(\.id)
        Task {
            await Burst.answer(model, ids: ids, label: item.connectionLabel, approve: approve)
            busy = false
        }
    }
}

private extension View {
    /// The plain label in the left third of a card's bottom row ("Unsure", "Always asks").
    func note() -> some View {
        font(RFont.sans(14, .semibold))
            .foregroundStyle(Palette.ink2)
            .frame(maxWidth: .infinity, minHeight: 42)
            .accessibilityElement(children: .combine)
    }
}

/// Two views side by side: the first a third of the width, the second the rest, as tall as the taller one.
private struct ThirdsRow: Layout {
    var spacing: CGFloat

    private func widths(_ total: CGFloat) -> (CGFloat, CGFloat) {
        let free = max(total - spacing, 0)
        return (free / 3, free * 2 / 3)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 320
        let (a, b) = widths(width)
        let heights = zip(subviews, [a, b]).map { $0.sizeThatFits(ProposedViewSize(width: $1, height: nil)).height }
        return CGSize(width: width, height: heights.max() ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let (a, b) = widths(bounds.width)
        var x = bounds.minX
        for (view, w) in zip(subviews, [a, b]) {
            view.place(at: CGPoint(x: x, y: bounds.midY), anchor: .leading, proposal: ProposedViewSize(width: w, height: bounds.height))
            x += w + spacing
        }
    }
}
