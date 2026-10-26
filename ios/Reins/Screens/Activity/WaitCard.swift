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
