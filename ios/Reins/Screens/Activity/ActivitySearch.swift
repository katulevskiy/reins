import SwiftUI

// Activity's search and its days (pure, unit-tested): what the filter chips hold and the query they make, the river
// grouped by local day with each day's count, the words that set a match apart in a row's title, and the words for the
// chips and the snippet's field. The search itself runs in the core, on the phone; nothing here sends it anywhere.

/// "When" on the filter row: any time, or since the start of today, the last 7 or 30 days.
enum ActivityWhen: String, CaseIterable {
    case any, today, week, month

    var title: String {
        switch self {
        case .any: "Any time"
        case .today: "Today"
        case .week: "7 days"
        case .month: "30 days"
        }
    }

    /// The query's `since` (Unix seconds): the local start of today, or `now` less 7 or 30 days; nil for any time.
    func since(now: Date, calendar: Calendar) -> Int64? {
        switch self {
        case .any: nil
        case .today: Int64(calendar.startOfDay(for: now).timeIntervalSince1970)
        case .week: Int64(now.timeIntervalSince1970) - 7 * 86_400
        case .month: Int64(now.timeIntervalSince1970) - 30 * 86_400
        }
    }
}

/// What the filter chips hold. Filters combine; the values within one are alternatives (the core's rule).
struct ActivityFilters: Equatable {
    var outcomes: [ActivityOutcome] = []
    var deciders: [ActivityDecider] = []
    /// Integrations ("gmail", "github", ...).
    var services: [String] = []
    /// Kinds ("search", "send", ...).
    var actions: [String] = []
    /// Connection ids.
    var connections: [String] = []
    var when: ActivityWhen = .any

    static let outcomeOrder: [ActivityOutcome] = [.approved, .denied, .failed]
    static let deciderOrder: [ActivityDecider] = [.you, .autopilot, .bypass, .lockdown]

    var isEmpty: Bool {
        outcomes.isEmpty && deciders.isEmpty && services.isEmpty && actions.isEmpty && connections.isEmpty && when == .any
    }

    mutating func toggle(_ outcome: ActivityOutcome) {
        outcomes = Self.toggled(outcomes, outcome).sorted { Self.rank(Self.outcomeOrder, $0) < Self.rank(Self.outcomeOrder, $1) }
    }

    mutating func toggle(_ decider: ActivityDecider) {
        deciders = Self.toggled(deciders, decider).sorted { Self.rank(Self.deciderOrder, $0) < Self.rank(Self.deciderOrder, $1) }
    }

    mutating func toggle(service: String) { services = Self.toggled(services, service).sorted() }

    mutating func toggle(action: String) { actions = Self.toggled(actions, action).sorted() }

    mutating func toggle(connection: String) { connections = Self.toggled(connections, connection).sorted() }

    /// The core's query for `text` (trimmed) under these filters, `since` worked out from `now` in `calendar`.
    func query(text: String, now: Date, calendar: Calendar = .current) -> ActivityQuery {
        ActivityQuery(
            text: text.trimmingCharacters(in: .whitespacesAndNewlines), outcomes: outcomes, deciders: deciders, services: services,
            actions: actions, connections: connections, since: when.since(now: now, calendar: calendar), until: nil
        )
    }

    /// Searching rather than browsing: words to find, or any filter on.
    static func searching(text: String, filters: ActivityFilters) -> Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !filters.isEmpty
    }

    private static func toggled<T: Equatable>(_ list: [T], _ value: T) -> [T] {
        list.contains(value) ? list.filter { $0 != value } : list + [value]
    }

    private static func rank<T: Equatable>(_ order: [T], _ value: T) -> Int { order.firstIndex(of: value) ?? order.count }
}

/// One local day of the river: where it starts and ends, and how many entries it holds, denied and failed.
struct ActivityDay: Equatable, Identifiable {
    /// "2026-10-15", the day in its calendar.
    var key: String
    var start: Date
    /// The next day's start (23 or 25 hours later across a daylight-saving change).
    var end: Date
    var count = 0
    var denied = 0
    var failed = 0
    /// Decided by Autopilot and not denied (the header's bolt count).
    var auto = 0

    var id: String { key }
    /// Approved by you, a grant, a bypass or anything but Autopilot (the header's check count).
    var approved: Int { max(count - denied - failed - auto, 0) }
    /// The day's query bounds: `since` inclusive, `until` exclusive (Unix seconds).
    var since: Int64 { Int64(start.timeIntervalSince1970.rounded(.down)) }
    var until: Int64 { Int64(end.timeIntervalSince1970.rounded(.down)) }

    func holds(_ at: Int64) -> Bool { at >= since && at < until }
}

enum ActivityDays {
    /// The stamps by local day in `calendar`, newest day first.
    static func group(_ stamps: [ActivityStamp], calendar: Calendar) -> [ActivityDay] {
        var days: [Date: ActivityDay] = [:]
        for stamp in stamps {
            let start = calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(stamp.at)))
            var day = days[start] ?? ActivityDay(key: key(start, calendar: calendar), start: start, end: next(start, calendar: calendar))
            day.count += 1
            if stamp.outcome == .denied { day.denied += 1 }
            if stamp.outcome == .failed { day.failed += 1 }
            if stamp.decider == .autopilot && stamp.outcome == .approved { day.auto += 1 }
            days[start] = day
        }
        return days.values.sorted { $0.start > $1.start }
    }

    /// "2026-10-15": the day `date` falls on in `calendar`.
    static func key(_ date: Date, calendar: Calendar) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", locale: FileText.posix, c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    /// The start of the day after the one starting at `start`.
    static func next(_ start: Date, calendar: Calendar) -> Date {
        let later = calendar.date(byAdding: .day, value: 1, to: start) ?? start.addingTimeInterval(86_400)
        return calendar.startOfDay(for: later)
    }

    /// The header's count: "14", "14 · 2 denied", "14 · 2 denied · 1 failed".
    static func summary(_ day: ActivityDay) -> String {
        var parts = ["\(day.count)"]
        if day.denied > 0 { parts.append("\(day.denied) denied") }
        if day.failed > 0 { parts.append("\(day.failed) \(day.failed == 1 ? "needs" : "need") attention") }
        return parts.joined(separator: " · ")
    }

    /// The header for VoiceOver: "Today, 14 entries, 2 denied".
    static func spoken(_ title: String, _ day: ActivityDay) -> String {
        var parts = [title, day.count == 1 ? "1 entry" : "\(day.count) entries"]
        if day.denied > 0 { parts.append("\(day.denied) denied") }
        if day.failed > 0 { parts.append("\(day.failed) \(day.failed == 1 ? "needs" : "need") attention") }
        return parts.joined(separator: ", ")
    }

    /// An entry as the core stamps it (how it ended, who decided), for days worked out from what is loaded.
    static func stamp(_ e: ActivityEntry) -> ActivityStamp {
        let outcome: ActivityOutcome = switch e.outcome {
        // As the core files them: an undone send with the denied, one not sent with the failed.
        case "denied", "undone": .denied
        case "error", "started", "unknown", "succeeded_withheld", "not_sent": .failed
        default: .approved
        }
        let decider: ActivityDecider = switch e.decidedBy {
        case "": .you
        case "bypass": .bypass
        case "lockdown": .lockdown
        default: .autopilot
        }
        return ActivityStamp(id: e.id, at: e.at, outcome: outcome, decider: decider)
    }

    /// The core's days (nil until read), with any day of the entries loaded at once they do not have yet (an entry
    /// that came after the stamps were read).
    static func merged(_ days: [ActivityDay]?, recent: [ActivityEntry], calendar: Calendar) -> [ActivityDay] {
        let loaded = group(recent.map(stamp), calendar: calendar)
        guard let days else { return loaded }
        let have = Set(days.map(\.key))
        let missing = loaded.filter { !have.contains($0.key) }
        return missing.isEmpty ? days : (days + missing).sorted { $0.start > $1.start }
    }

    /// A day's rows: the pages read for it and the history read at once (fresher), newest first, once each.
    static func rows(_ day: ActivityDay, pages: [ActivityEntry], recent: [ActivityEntry]) -> [ActivityEntry] {
        var byId: [Int64: ActivityEntry] = [:]
        for e in pages where day.holds(e.at) { byId[e.id] = e }
        for e in recent where day.holds(e.at) { byId[e.id] = e }
        return byId.values.sorted { ($0.at, $0.id) > ($1.at, $1.id) }
    }
}

/// Sets apart in a title the words searched (the core's rule, so titles and snippets agree): after the title is
/// cleaned, a match is a term found case-insensitively at the start of a word (letters and digits), as long as the
/// term (the longest wins); matches apart by one or two characters that are neither letters, digits nor spaces
/// (`@ . / - _`) join into one. Never inside a word.
enum TitleHighlight {
    /// The soft violet behind a match; an underline goes with it, so colour is never the only cue.
    static let wash = Palette.violetWash

    private static func isWord(_ s: Unicode.Scalar) -> Bool { s.properties.isAlphabetic || s.properties.numericType != nil }

    private static func lower(_ s: Unicode.Scalar) -> Unicode.Scalar { s.properties.lowercaseMapping.unicodeScalars.first ?? s }

    /// The matches in `text` (already cleaned), as ranges of Unicode scalar offsets.
    static func hits(_ text: String, terms: [String]) -> [Range<Int>] {
        let terms = terms.map { Array($0.unicodeScalars.map(lower)) }.filter { !$0.isEmpty }
        guard !terms.isEmpty else { return [] }
        let scalars = Array(text.unicodeScalars)
        let chars = scalars.map(lower)
        var found: [Range<Int>] = []
        var i = 0
        while i < chars.count {
            if isWord(chars[i]) && (i == 0 || !isWord(chars[i - 1])) {
                let longest = terms
                    .filter { $0.count <= chars.count - i && zip($0, chars[i...]).allSatisfy { $0 == $1 } }
                    .map(\.count)
                    .max()
                if let n = longest {
                    found.append(i..<(i + n))
                    i += n
                    continue
                }
            }
            i += 1
        }
        var joined: [Range<Int>] = []
        for r in found {
            if let last = joined.last {
                let gap = scalars[last.upperBound..<r.lowerBound]
                if gap.count <= 2 && gap.allSatisfy({ !isWord($0) && !$0.properties.isWhitespace }) {
                    joined[joined.count - 1] = last.lowerBound..<r.upperBound
                    continue
                }
            }
            joined.append(r)
        }
        return joined
    }

    /// What is set apart in a title from outside: cleaned first (`untrusted`), so a hidden character can neither make
    /// a word start nor shift a match.
    static func matches(_ title: String, terms: [String]) -> [String] {
        let text = untrusted(title)
        let scalars = Array(text.unicodeScalars)
        return hits(text, terms: terms).map { String(String.UnicodeScalarView(scalars[$0])) }
    }

    /// Sets apart the matches in `text`, whose characters are already clean. The word joiners a title puts after a
    /// slash (so "octo/app" never breaks there) are not read: they neither split nor join a match.
    static func apply(_ text: inout AttributedString, terms: [String]) {
        guard !terms.isEmpty else { return }
        let scalars = Array(text.unicodeScalars)
        var kept: [Int] = []
        var plain = String.UnicodeScalarView()
        for (i, s) in scalars.enumerated() where s != "\u{2060}" {
            kept.append(i)
            plain.append(s)
        }
        for r in hits(String(plain), terms: terms) {
            // Indices from the string as it is now: setting attributes may change its runs.
            let view = text.unicodeScalars
            let start = view.index(view.startIndex, offsetBy: kept[r.lowerBound])
            let end = view.index(view.startIndex, offsetBy: kept[r.upperBound - 1] + 1)
            text[start..<end].backgroundColor = wash
            text[start..<end].underlineStyle = Text.LineStyle.single
            text[start..<end].foregroundColor = Palette.ink
        }
    }

    /// The parts of `text` set apart (tests, VoiceOver).
    static func marked(_ text: AttributedString) -> [String] {
        text.runs.filter { $0[AttributeScopes.SwiftUIAttributes.UnderlineStyleAttribute.self] != nil }.map { String(text[$0.range].characters) }
    }
}

/// Activity's search in words: the chips, the menus' rows, the snippet's field.
enum ActivitySearchText {
    static func outcome(_ o: ActivityOutcome) -> String {
        switch o {
        case .approved: "Approved"
        case .denied: "Denied"
        case .failed: "Needs attention"
        }
    }

    static func decider(_ d: ActivityDecider) -> String {
        switch d {
        case .you: "You"
        case .autopilot: "Automatic"
        case .bypass: "Bypass"
        case .lockdown: "Lockdown"
        }
    }

    /// The menu row ids: "filter:denied", "filter:autopilot" (the old filter chips' ids).
    static func id(_ o: ActivityOutcome) -> String { "filter:\(outcome(o).lowercased())" }

    static func id(_ d: ActivityDecider) -> String { d == .autopilot ? "filter:autopilot" : "filter:\(decider(d).lowercased())" }

    /// A kind in the words the app uses for it: "Search", "Send", "Permission".
    static func kind(_ action: String) -> String {
        switch ActionKind.of(action) {
        case .search: "Search"
        case .list: "List"
        case .read: "Read"
        case .write: "Change"
        case .send: "Send"
        case .grant: "Permission"
        case .accounts: "Accounts"
        case .pair: "Connect"
        case .headlines: "Headlines"
        case .upload: "Upload"
        case .join: "New phone"
        case .other: action == "request" || action.isEmpty ? "Request" : action.prefix(1).uppercased() + action.dropFirst()
        }
    }

    /// A chip: its name with nothing chosen, the one chosen, or the first and how many more ("Denied +1").
    static func chip(_ placeholder: String, _ chosen: [String]) -> String {
        switch chosen.count {
        case 0: placeholder
        case 1: chosen[0]
        default: "\(chosen[0]) +\(chosen.count - 1)"
        }
    }

    /// The snippet's field as its tiny mono label.
    static func field(_ f: ActivityField) -> String {
        switch f {
        case .title: "TITLE"
        case .agent: "AGENT"
        case .address: "ADDRESS"
        case .subject: "SUBJECT"
        case .body: "BODY"
        case .repo: "REPO"
        case .branch: "BRANCH"
        case .commit: "COMMIT"
        case .file: "FILE"
        case .diff: "DIFF"
        case .command: "COMMAND"
        case .url: "URL"
        case .request: "REQUEST"
        }
    }

    /// Where a match was found, as the glyph before the snippet (the field's label for VoiceOver).
    static func glyph(_ f: ActivityField) -> Glyph {
        switch f {
        case .title, .request: .fileText
        case .agent: .person
        case .address, .subject, .body: .mail
        case .repo, .branch: .gitBranch
        case .commit: .gitCommit
        case .file, .diff: .file
        case .command: .terminal
        case .url: .globe
        }
    }

    /// A snippet's span shown: cleaned again, its edge spaces kept so the spans still read as words.
    static func span(_ text: String) -> String {
        let inner = untrusted(text)
        guard !inner.isEmpty else { return text.isEmpty ? "" : " " }
        let lead = text.first.map { $0.isWhitespace } == true ? " " : ""
        let trail = text.last.map { $0.isWhitespace } == true ? " " : ""
        return lead + inner + trail
    }

    /// The snippet line: the field's label, then the spans, the matched ones set apart (ink, medium, a soft violet
    /// wash and an underline), the others in meta ink.
    static func snippet(_ s: ActivitySnippet) -> AttributedString {
        var out = AttributedString()
        for span in s.spans {
            var part = AttributedString(Self.span(span.text))
            part.font = RFont.mono(12)
            part.foregroundColor = span.hit ? Palette.ink : Palette.inkMeta
            if span.hit {
                part.backgroundColor = TitleHighlight.wash
                part.underlineStyle = Text.LineStyle.single
            }
            out += part
        }
        return out
    }

    /// The service's glyph in the Kind menu.
    static func glyph(service: String) -> Glyph {
        if service.hasPrefix(mcpPrefix) { return .server }
        switch service {
        case "gmail": return .mail
        case "github", "gitlab", "codeberg", "bitbucket": return .gitBranch
        case "telegram", "sms": return .send
        case "gcalendar", "device_calendar": return .calendar
        case "gcontacts", "device_contacts": return .person
        case "desktop", "terminal": return .laptop
        case "files", "file": return .file
        case "vault", "secrets": return .vault
        case "ssh": return .key
        default: return .link
        }
    }

    /// The kind's glyph in the Kind menu.
    static func glyph(action: String) -> Glyph {
        switch ActionKind.of(action) {
        case .search, .list: .search
        case .read, .headlines: .eye
        case .send: .send
        case .write: .pencil
        case .grant: .key
        case .accounts: .person
        case .pair: .link
        case .join: .phone
        case .upload: .file
        case .other: .shield
        }
    }
}
