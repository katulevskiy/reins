import Foundation

// How a request is written and marked everywhere but the approval itself (design/signal/ASK-TEXT.md): waiting cards,
// Activity rows, entry pages and notifications, from the core's own label (`RequestLabel`). The approval's field
// writes from its richer view (AskFormat) with the same rules and the same `AskLine`. Compiled into the app and the
// notification extension.
//
// What may reach the serif (the security reviews): the verb, counts and target come from the core's label, never from
// what an agent, a sender or a server wrote, and nothing is parsed here beyond the core's own fixed words. Commands,
// paths, hosts, addresses and anything an agent typed go to the mono line. A target that can pass for another leaves
// the sentence for its stand-in words, its raw form escaped below. What was done reads in the past; what was denied,
// failed or timed out says what was asked.

/// A request's icon: the integration's mark (a provider's logo or a local glyph) and the action, in the kind's field
/// colour.
struct AskIcon: Equatable {
    enum Mark: Equatable {
        /// A provider's logo: "github", "gmail", "mcp", "vault", …
        case service(String)
        case glyph(Glyph)
    }

    var mark: Mark
    var action: Glyph
    /// Nil for a request with no kind: the badge is grey.
    var kind: FieldKind?
}

/// One request, written.
struct AskLine: Equatable {
    /// The label line: "GitHub · Push".
    var kind: String
    /// Roman, before the target: "Push 8 commits to ".
    var lead: String
    /// The only italic: "octo/app". Nil when the sentence has none.
    var target: String?
    /// Roman, after the target: " and 2 more".
    var after = ""
    /// Machine details, in mono under the sentence; never cut.
    var mono: String?
    var icon: AskIcon

    /// The sentence as it reads.
    var text: String { lead + (target ?? "") + after }

    /// The sentence as the serif's runs, the target in italic. A slash never ends a line ("octo/ app").
    var runs: [AskText.Run] {
        [AskText.Run(text: lead), AskText.Run(text: target ?? "", target: true), AskText.Run(text: after)]
            .filter { !$0.text.isEmpty }
            .map { AskText.Run(text: $0.text.replacingOccurrences(of: "/", with: "/\u{2060}"), target: $0.target) }
    }

    /// A line, its target checked once for every surface. `stand`: what the sentence says instead of a target that can
    /// pass for another ("a repository"); nil for a name, which is flagged only when it mixes scripts. A flagged
    /// target's raw form, escaped, leads the mono line.
    static func make(
        kind: String, lead: String, target: String?, after: String = "", mono: String? = nil, stand: String? = nil, icon: AskIcon
    ) -> AskLine {
        var lead = lead
        var target = target.flatMap { $0.isEmpty ? nil : $0 }
        var flag: String?
        if let t = target, let stand {
            // Reins' own separator ("web · dev") is not part of what is checked.
            let own = t.replacingOccurrences(of: " · ", with: " ")
            if !own.unicodeScalars.allSatisfy(\.isASCII) || mixedScriptRaw(own) != nil {
                let noun = stand.split(separator: " ").last.map { $0.prefix(1).uppercased() + $0.dropFirst() } ?? "Name"
                flag = "\(noun): \(rawForm(t))"
                // No stand-in words: the sentence ends before the word that led to the target.
                lead = stand.isEmpty ? AskLabels.dropPreposition(lead.trimmingCharacters(in: .whitespaces)) : lead + stand
                target = nil
            }
        } else if let t = target, let raw = mixedScriptRaw(t) {
            flag = "\(raw) · mixed scripts"
        }
        let lines = [flag, mono?.trimmingCharacters(in: .whitespacesAndNewlines)].compactMap { $0 }.filter { !$0.isEmpty }
        return AskLine(kind: kind, lead: lead, target: target, after: after, mono: lines.isEmpty ? nil : lines.joined(separator: "\n"), icon: icon)
    }
}

/// When a request is told: still waiting, carried out, or not (denied, failed, timed out).
enum AskTense: Equatable {
    case waiting, done, refused

    /// Done only when it was carried out; anything else says what was asked, never "ran" or "sent".
    static func of(outcome: String) -> AskTense { ["released", "sent", "granted"].contains(outcome) ? .done : .refused }
}

enum AskLabels {
    /// A request from the core's label. `cwd`: where a command ran, as the core kept it; `matched`: the guard's own
    /// match; `who`: the asker, to start a past sentence where it is not shown beside it.
    static func line(
        _ label: RequestLabel, field: FieldKind?, tense: AskTense = .waiting, who: String? = nil, cwd: String? = nil, matched: String? = nil,
        git: GitPushView? = nil
    ) -> AskLine {
        let provider = label.provider
        let action = label.action
        let lead = oneLine(label.lead)
        let subject = oneLine(label.subject)
        let tail = oneLine(label.tail)
        let icon = self.icon(provider: provider, action: action, field: field)
        let kind = kindLabel(provider: provider, action: action)
        func make(_ lead: String, _ target: String? = nil, _ after: String = "", mono: String? = nil, stand: String? = nil) -> AskLine {
            told(AskLine.make(kind: kind, lead: lead, target: target, after: after, mono: mono, stand: stand, icon: icon), tense, who)
        }
        let more = tail.isEmpty ? "" : " " + tail.trimmingCharacters(in: .whitespaces)
        if provider == "terminal" {
            if let command = label.command {
                if tense == .done {
                    let heading = command.heading.hasPrefix("Run ") ? "Approved running " + command.heading.dropFirst(4) : "Approved a shell command"
                    return AskLine.make(kind: kind, lead: heading, target: nil, mono: command.actionable.isEmpty ? nil : untrusted(command.actionable), icon: icon)
                }
                return make(oneLine(command.heading), mono: command.actionable.isEmpty ? nil : untrusted(command.actionable))
            }
            let m = matched.map(untrusted)?.trimmingCharacters(in: .whitespacesAndNewlines)
            let folder = cwd.map { untrustedName(($0 as NSString).lastPathComponent) }.flatMap { $0.isEmpty || $0 == "/" ? nil : $0 }
            let mono = m.flatMap { $0.isEmpty ? nil : $0 } ?? (subject + more)
            return folder.map { make("Run a command in ", $0, mono: mono, stand: "a folder") } ?? make("Run a command", mono: mono)
        }
        if provider == "file" { return make("\(lead.isEmpty ? "Read" : lead) a file", mono: subject + more) }
        if provider == "ssh" && !subject.isEmpty { return make("Sign in to a server", mono: asciiHost(subject)) }
        if provider == "secrets" && !subject.isEmpty { return make("\(dropPreposition(lead)) for a command", mono: subject) }
        if provider == "desktop" && action == "ask" {
            // A question in its own words: the words go below, never into the sentence.
            let words = lead == "A question from your computer" ? nil : lead
            return make("Answer a question from ", "your computer", mono: words)
        }
        if provider.hasPrefix(mcpPrefix) {
            // "Use … on Linear": the server's name is the core's tail; the tool (or its title, the server's words) below.
            let server = tail.hasPrefix("on ") ? String(tail.dropFirst(3)) : serviceName(provider)
            return make("Use a tool on ", server, mono: subject, stand: "a server")
        }
        if action == "push" || (action == "delete" && lead.hasPrefix("Delete ") && lead.hasSuffix(" from")) {
            let (head, names) = gitLead(lead)
            // The push as kept with the entry: its count and its branches from the refs, never from words.
            if let git, names == nil, action == "push" {
                let commits = git.refs.reduce(0) { $0 + Int($1.commitCount) }
                let pushHead = commits > 0 ? "Push \(commits) \(commits == 1 ? "commit" : "commits") to" : head
                return make(pushHead + " ", subject, mono: refsLine(git), stand: "a repository")
            }
            return make(head + " ", subject, mono: names, stand: "a repository")
        }
        if provider == "gmail" && action == "send" {
            // The name in "Dana <dana@…>" is the AI's own words: the address goes below.
            let others = Int(tail.split(separator: " ").first { Int($0) != nil } ?? "") ?? 0
            let head = others > 0 ? "Send an email to \(others + 1) people" : "Send an email"
            return make(head, mono: subject.isEmpty ? nil : asciiAddress(subject) + more)
        }
        if lead.hasSuffix(":") { return make(String(lead.dropLast()), mono: subject) }
        if subject.isEmpty { return make(lead + more) }
        if label.code || looksMachine(subject) || provider == "gmail" {
            // A command, a path, an address, a sender's name or a search: below the sentence.
            return make(dropPreposition(lead) + more, mono: subject)
        }
        let names = ["telegram", "sms", "gcontacts", "device_contacts", "gcalendar", "device_calendar"].contains(provider)
        return make(lead + " ", subject, more, stand: names ? nil : "")
    }

    /// A waiting item: a request from its label, else from what its kind says; a pairing or a new phone in Reins'
    /// own words (the other side's name never reaches the sentence).
    static func pending(_ item: PendingItem, field: FieldKind?) -> AskLine {
        switch item.kind {
        case .pairing:
            // Whether it is a computer or an AI app is known once the pairing is opened; its own name only quoted.
            return AskLine(kind: "Reins · Connect", lead: "Connect to ", target: "Reins", mono: quoted(item.connectionLabel),
                           icon: AskIcon(mark: .glyph(.link), action: .plus, kind: field))
        case .join:
            return AskLine(kind: "Account · New phone", lead: "Add a phone to ", target: "your account", mono: quoted(item.connectionLabel),
                           icon: AskIcon(mark: .glyph(.phone), action: .plus, kind: field))
        case .blob:
            let name = item.label.map { oneLine($0.subject) } ?? ""
            return AskLine.make(kind: "Files · Share", lead: "Share a file", target: nil, mono: name, icon: AskIcon(mark: .glyph(.file), action: .send, kind: field))
        case .request:
            if let label = item.label { return line(label, field: field) }
            var line = legacy(service: item.service, action: item.action, count: Int(item.count), op: item.op, opTitle: item.opTitle, field: field, tense: .waiting)
            // What it is about (a search, an account), as the core summed it up, below.
            let subtitle = untrusted(item.subtitle).trimmingCharacters(in: .whitespacesAndNewlines)
            line.mono = subtitle.isEmpty ? nil : subtitle
            return line
        }
    }

    /// A waiting item in one line (lists, widgets, notifications): "Claude: Push 3 commits to octo/app"; a pairing or a
    /// new phone in Reins' own words, without the other side's name.
    static func listTitle(_ item: PendingItem) -> String {
        let text = pending(item, field: FieldKind.of(ActionKind.of(item.action))).text
        guard item.kind == .request || item.kind == .blob else { return text }
        let who = untrustedName(item.connectionLabel, limit: 40)
        return who.isEmpty ? text : "\(who): \(text)"
    }

    /// A decided entry, told as it ended.
    static func entry(_ e: ActivityEntry, field: FieldKind?, who: String? = nil) -> AskLine {
        let tense = AskTense.of(outcome: e.outcome)
        if let label = e.label {
            let request = e.request
            return line(label, field: field, tense: tense, who: who, cwd: request.flatMap { $0.cwd.isEmpty ? nil : $0.cwd }, git: request?.git)
        }
        return legacy(service: e.service, action: e.action, count: Int(e.count), op: e.op, opTitle: e.opTitle, field: field, tense: tense, who: who)
    }

    /// A request from before labels: what its operation is called ("Send email to 2"), nothing it carried.
    private static func legacy(
        service: String, action: String, count: Int, op: String, opTitle: String, field: FieldKind?, tense: AskTense, who: String? = nil
    ) -> AskLine {
        let svc = service.isEmpty ? "gmail" : service
        let lead = oneLine(operationTitle(action: action, count: count, service: service, title: opTitle, op: op))
        let line = AskLine(kind: kindLabel(provider: svc, action: action), lead: lead, target: nil, icon: icon(provider: svc, action: action, field: field))
        return told(line, tense, who)
    }

    // MARK: Parts

    /// A name the other side sent: quoted, one short clean line; nil for none.
    static func quoted(_ name: String) -> String? {
        let clean = untrustedName(name, limit: 48)
        return clean.isEmpty ? nil : "“\(clean)”"
    }

    /// Puts the tense on: present from the verb; past ("Pushed …", "Claude Code pushed …") once carried out; "Asked to
    /// …" otherwise.
    static func told(_ line: AskLine, _ tense: AskTense, _ who: String?) -> AskLine {
        var line = line
        line.lead = oneVerb(line.lead)
        let name = who.map { untrustedName($0) }.flatMap { $0.isEmpty || mixedScriptRaw($0) != nil ? nil : $0 }
        switch tense {
        case .waiting:
            line.lead = capitalised(line.lead)
        case .done:
            let past = self.past(line.lead)
            line.lead = name.map { "\($0) \(lowerFirst(past))" } ?? capitalised(past)
        case .refused:
            line.lead = (name.map { "\($0) asked to " } ?? "Asked to ") + lowerFirst(line.lead)
        }
        return line
    }

    /// The irregular and common verbs a request starts with, in the past.
    private static let pastTense: [String: String] = [
        "push": "Pushed", "force-push": "Force-pushed", "read": "Read", "send": "Sent", "search": "Searched", "delete": "Deleted",
        "create": "Created", "use": "Used", "open": "Opened", "list": "Listed", "share": "Shared", "write": "Wrote", "run": "Ran",
        "make": "Made", "add": "Added", "update": "Updated", "comment": "Commented", "merge": "Merged", "close": "Closed",
        "connect": "Connected", "see": "Saw", "get": "Got", "give": "Gave", "hand": "Handed", "move": "Moved", "change": "Changed",
        "edit": "Edited", "remove": "Removed", "reply": "Replied", "forward": "Forwarded", "archive": "Archived", "label": "Labelled",
        "fetch": "Fetched", "upload": "Uploaded", "download": "Downloaded", "sign": "Signed", "release": "Released", "save": "Saved",
        "answer": "Answered", "commit": "Committed", "ask": "Asked", "start": "Started", "end": "Ended", "clone": "Cloned", "pay": "Paid", "print": "Printed",
    ]

    /// One verb where the core's title has two for one act: a clone is a first fetch, so "Clone and fetch octo/app"
    /// is "Fetch octo/app" (and "Fetched", "Asked to fetch").
    static func oneVerb(_ lead: String) -> String {
        let prefix = "clone and fetch"
        guard lead.lowercased().hasPrefix(prefix) else { return lead }
        let rest = lead.dropFirst(prefix.count)
        return (lead.first?.isUppercase == true ? "Fetch" : "fetch") + rest
    }

    /// `lead` in the past: its first word, if it is a verb this knows, and a second verb joined to it by "and" too
    /// ("Create and push" is "Created and pushed", never "Created and push").
    static func past(_ lead: String) -> String {
        let words = lead.split(separator: " ", omittingEmptySubsequences: false).map(String.init)
        guard let first = words.first, let done = pastTense[first.lowercased()] else { return lead }
        var out = words
        out[0] = done
        if out.count > 2, out[1] == "and", let second = pastTense[out[2].lowercased()] {
            out[2] = second.lowercased()
        }
        return out.joined(separator: " ")
    }

    /// The label line from the core's provider and action: "GitHub · Push", "Computer · Run".
    static func kindLabel(provider: String, action: String) -> String {
        if provider == "ssh" { return "SSH · Sign in" }
        let place: String = switch provider {
        case "terminal", "file", "desktop": "Computer"
        case "secrets": "Vault"
        case "ssh": "SSH"
        case "files": "Files"
        case "": "Request"
        default: serviceName(provider)
        }
        return "\(place) · \(actionWord(action))"
    }

    private static func actionWord(_ action: String) -> String {
        switch action {
        case "run": "Run"
        case "read": "Read"
        case "send": "Send"
        case "delete": "Delete"
        case "secrets": "Secrets"
        case "push": "Push"
        case "search": "Search"
        case "grant": "Permission"
        case "ask": "Asks"
        case "accounts": "Accounts"
        case "": "Request"
        default: "Change"
        }
    }

    /// The icon from the core's provider and action, in `field` (the core's kind).
    static func icon(provider: String, action: String, field: FieldKind?) -> AskIcon {
        let mark: AskIcon.Mark = switch provider {
        case "terminal": .glyph(.terminal)
        case "file", "files": .glyph(.file)
        case "secrets": .glyph(.lock)
        case "ssh": .glyph(.key)
        case "desktop": .glyph(action == "ask" ? .help : .laptop)
        case "payments", "pay": .glyph(.card)
        case "": .glyph(.help)
        default: .service(provider.hasPrefix(mcpPrefix) ? "mcp" : provider)
        }
        let glyph: Glyph = switch action {
        case "run": .play
        case "read": .eye
        case "send": .send
        case "delete": .trash
        case "secrets": .key
        case "push": .gitBranch
        case "search": .search
        case "grant": .shieldCheck
        case "ask": .help
        case "accounts": .person
        default: .pencil
        }
        return AskIcon(mark: mark, action: glyph, kind: field)
    }

    /// A push's lead from the core with any branch or tag name moved out: "Delete branch old from" → ("Delete a branch
    /// from", "old"), "Push tag v1.2 to" → ("Push a tag to", "v1.2"). Only the core's own fixed words are matched.
    static func gitLead(_ lead: String) -> (String, String?) {
        for kind in ["branch", "tag"] where lead.hasPrefix("Delete \(kind) ") && lead.hasSuffix(" from") {
            let name = lead.dropFirst("Delete \(kind) ".count).dropLast(" from".count)
            return ("Delete a \(kind) from", String(name))
        }
        if lead.hasPrefix("Push tag ") && lead.hasSuffix(" to") {
            return ("Push a tag to", String(lead.dropFirst("Push tag ".count).dropLast(" to".count)))
        }
        return (lead, nil)
    }

    /// A push's branches in one mono line: "feature/laya · new branch", "main · force push", "v1.0 · new tag".
    static func refsLine(_ git: GitPushView) -> String? {
        let parts = git.refs.map { ref -> String in
            let name = oneLine(ref.shortName)
            switch ref.change {
            case "create": return "\(name) · new \(ref.kind == "tag" ? "tag" : "branch")"
            case "delete": return "\(name) · delete"
            default: return ref.force || ref.forceUnknown ? "\(name) · force push" : name
            }
        }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }

    /// "Search Gmail for" → "Search Gmail": the word that led to a target that went below.
    static func dropPreposition(_ lead: String) -> String {
        for word in [" for", " from", " to", " in", " on", " with", ":"] where lead.hasSuffix(word) {
            return String(lead.dropLast(word.count))
        }
        return lead
    }

    /// A subject that is a machine value: a URL, an address, a path.
    static func looksMachine(_ s: String) -> Bool {
        s.contains("://") || s.contains("@") || s.hasPrefix("/") || s.hasPrefix("~") || s.hasPrefix("./")
    }

    /// Cleaned, on one line, never cut.
    private static func oneLine(_ s: String) -> String { untrustedName(s, limit: .max) }

    private static func capitalised(_ s: String) -> String { s.prefix(1).uppercased() + s.dropFirst() }

    private static func lowerFirst(_ s: String) -> String {
        let chars = Array(s)
        guard chars.count > 1, chars[0].isUppercase, chars[1].isLowercase || chars[1] == " " || chars[1] == "-" else { return s }
        return chars[0].lowercased() + String(chars.dropFirst())
    }
}
