import Foundation

/// The demo's search (`DemoReinsCore.searchActivity`): the core's rules made simple. Words split as the core splits
/// them; each must be found, case apart, anywhere in what the entry holds; a snippet when a word is in an email's body.
enum DemoSearch {
    /// The words searched, lower case, longest first (as the core returns them).
    static func terms(_ text: String) -> [String] {
        var out: [String] = []
        for word in text.split(whereSeparator: \.isWhitespace) {
            for part in word.split(whereSeparator: { !($0.isLetter || $0.isNumber) }) {
                let p = part.lowercased()
                if !out.contains(p) { out.append(p) }
            }
        }
        return out.sorted { ($0.count, $1) > ($1.count, $0) }
    }

    /// What the row shows: the title and who asked.
    private static func row(_ e: ActivityEntry) -> String {
        [EntryText.title(e), e.label.map(RequestLabelLook.plain) ?? "", e.connectionLabel].joined(separator: "\n").lowercased()
    }

    private static func bodies(_ e: ActivityEntry) -> [String] {
        e.info.messages.map(\.text) + [e.info.email?.body].compactMap { $0 }
    }

    private static func everything(_ e: ActivityEntry) -> String {
        var parts = [row(e), e.detail, e.info.query ?? "", e.info.note ?? ""]
        parts += e.info.messages.flatMap { [$0.from, $0.subject, $0.text] }
        if let email = e.info.email { parts += email.to + email.cc + [email.subject, email.body] }
        return parts.joined(separator: "\n").lowercased()
    }

    static func keeps(_ e: ActivityEntry, _ q: ActivityQuery, _ terms: [String]) -> Bool {
        if let since = q.since, e.at < since { return false }
        if let until = q.until, e.at >= until { return false }
        if !q.outcomes.isEmpty {
            let approved = ["released", "sent", "granted"].contains(e.outcome)
            let kept = q.outcomes.contains { o in
                switch o {
                case .approved: approved
                case .denied: e.outcome == "denied"
                case .failed: e.outcome == "error"
                }
            }
            if !kept { return false }
        }
        if !q.deciders.isEmpty && !q.deciders.contains(ActivityDays.stamp(e).decider) { return false }
        if !q.services.isEmpty && !q.services.contains(e.service) { return false }
        if !q.actions.isEmpty && !q.actions.contains(e.action) { return false }
        if !q.connections.isEmpty && !q.connections.contains(e.connectionId) { return false }
        guard !terms.isEmpty else { return true }
        let text = everything(e)
        return terms.allSatisfy { text.contains($0) }
    }

    /// A few words around the first searched word found in an email's body, when the row does not show every word.
    static func snippet(_ e: ActivityEntry, _ terms: [String]) -> ActivitySnippet? {
        let shown = row(e)
        guard terms.contains(where: { !shown.contains($0) }) else { return nil }
        for body in bodies(e) {
            let flat = body.split(whereSeparator: \.isNewline).joined(separator: " ")
            for term in terms {
                guard let hit = flat.range(of: term, options: .caseInsensitive) else { continue }
                let start = flat.index(hit.lowerBound, offsetBy: -30, limitedBy: flat.startIndex) ?? flat.startIndex
                let end = flat.index(hit.upperBound, offsetBy: 60, limitedBy: flat.endIndex) ?? flat.endIndex
                let before = (start > flat.startIndex ? "…" : "") + flat[start..<hit.lowerBound]
                let after = flat[hit.upperBound..<end] + (end < flat.endIndex ? "…" : "")
                let spans = [
                    ActivityHighlight(text: String(before), hit: false), ActivityHighlight(text: String(flat[hit]), hit: true),
                    ActivityHighlight(text: String(after), hit: false),
                ].filter { !$0.text.isEmpty }
                return ActivitySnippet(field: .body, spans: spans)
            }
        }
        return nil
    }
}
