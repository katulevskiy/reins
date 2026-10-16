import Foundation

// What deserves a closer look inside a push or a list of emails, in words and in order (the Android app's
// Attention.kt). The core marks each commit, file and message; this only words, counts and sorts. Pure, tested in
// AttentionTextTests.

/// One row's mark: the labels on its chip and how closely to look.
struct AttentionMark: Equatable {
    var level: AttentionLevel
    var labels: [String]

    /// "Changes CI · Deletes a file".
    var text: String { labels.joined(separator: " · ") }

    /// What VoiceOver reads: "Look closer: Changes CI, Deletes a file".
    var spoken: String { "\(level == .close ? "Look closer" : "Worth a look"): \(labels.joined(separator: ", "))" }
}

enum AttentionText {
    /// A commit's or a file's mark: the core's labels as they are, less any `shown` apart on the same screen (the
    /// commit page's own "Hidden characters" tag). Nil when nothing is left.
    static func mark(_ attention: Attention?, shown: Set<ReasonCode> = []) -> AttentionMark? {
        guard let attention else { return nil }
        let labels = attention.reasons.filter { !shown.contains($0.code) }.map(\.label)
        return labels.isEmpty ? nil : AttentionMark(level: attention.level, labels: labels)
    }

    /// An email's (or an item's) mark. A sign-in code or a password keeps the row's one word ("Code", "Password")
    /// instead of a second chip for the same fact; a message the core calls sensitive is marked even without reasons.
    static func mail(_ m: MessageView, service: String) -> AttentionMark? {
        let held = service == "vault" ? "Password" : "Code"
        var labels: [String] = []
        for reason in m.attention?.reasons ?? [] {
            let label = switch reason.code {
            case .signInCode, .password: held
            default: reason.label
            }
            if !labels.contains(label) { labels.append(label) }
        }
        if m.sensitive && !labels.contains(held) { labels.insert(held, at: 0) }
        guard !labels.isEmpty else { return nil }
        return AttentionMark(level: m.sensitive ? .close : m.attention?.level ?? .look, labels: labels)
    }

    /// Close first, then Look, then the rest.
    private static func band(_ level: AttentionLevel?) -> Int {
        switch level {
        case .close: 0
        case .look: 1
        case nil: 2
        }
    }

    /// The rows to show, flagged first (Close, then Look, then the rest), each band in its original order. Each
    /// keeps its index in `items`, which is what a row opens: the list itself is never reordered.
    static func ordered<T>(_ items: [T], level: (T) -> AttentionLevel?) -> [(offset: Int, element: T)] {
        items.enumerated()
            .map { (band: band(level($0.element)), offset: $0.offset, element: $0.element) }
            .sorted { ($0.band, $0.offset) < ($1.band, $1.offset) }
            .map { (offset: $0.offset, element: $0.element) }
    }

    /// "2 of 8 to look at"; "2 of 50 shown to look at" when only `listed` of `total` came with the request (those not
    /// listed were not looked at). Nil when nothing is flagged.
    static func summary(flagged: Int, listed: Int, total: Int) -> String? {
        guard flagged > 0 else { return nil }
        return listed < total ? "\(flagged) of \(listed) shown to look at" : "\(flagged) of \(total) to look at"
    }

    /// The summary on a ref's folded row: its commits, or its files when the commits are not listed.
    static func summary(_ ref: GitRefView) -> String? {
        if !ref.commits.isEmpty || ref.commitCount > 0 {
            return summary(flagged: ref.commits.filter { $0.attention != nil }.count, listed: ref.commits.count, total: Int(ref.commitCount))
        }
        return summary(flagged: ref.files.filter { $0.attention != nil }.count, listed: ref.files.count, total: Int(ref.filesChanged))
    }

    /// Whether a row shows its own "Hidden" tag: whenever it has hidden characters, unless its chip already names them.
    /// The core keeps at most three reasons, so heavier ones can leave hidden characters out of the chip.
    static func hiddenApart(_ hidden: Bool, _ attention: Attention?) -> Bool {
        hidden && !(attention?.reasons.contains { $0.code == .hiddenChars } ?? false)
    }

    /// "Rewrites history" on a force push, or one whose history could not be checked, flagged or not.
    static func rewrites(_ ref: GitRefView) -> Bool { ref.force || ref.forceUnknown }

    /// The emails' header: "12 found", "12 found · 2 to look at"; an Activity entry also says how many were shared.
    static func mailHeader(_ messages: [MessageView], service: String, shared: Int? = nil) -> String {
        let flagged = messages.filter { mail($0, service: service) != nil }.count
        return (["\(messages.count) found"] + [shared.map { "\($0) shared" }, flagged > 0 ? "\(flagged) to look at" : nil].compactMap { $0 })
            .joined(separator: " · ")
    }
}
