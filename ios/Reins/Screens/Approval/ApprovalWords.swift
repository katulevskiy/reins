import Foundation

// The approval screen in words and colours (the Android app's ApprovalWords.kt, rule for rule), kept apart from the
// views so every rule is tested in ApprovalWordsTests.

/// A run of the hero's sentence; the key nouns are bold.
struct SentenceSpan: Equatable {
    var text: String
    var bold = false
}

/// The hero's colour: what kind of thing is asked.
enum HeroTone { case write, search, send, grant, danger }

/// The small badge on the service's tile: the action.
enum HeroBadge {
    case pen, search, read, list, send, trash, shield, people, key, terminal, question, cart

    var symbol: String {
        switch self {
        case .pen: "pencil"
        case .search: "magnifyingglass"
        case .read: "eye.fill"
        case .list: "list.bullet"
        case .send: "paperplane.fill"
        case .trash: "trash.fill"
        case .shield: "checkmark.shield.fill"
        case .people: "person.2.fill"
        case .key: "key.fill"
        case .terminal: "terminal.fill"
        case .question: "questionmark"
        case .cart: "cart.fill"
        }
    }
}

enum ApprovalWords {
    /// A change that cannot be undone or reaches far: asked every time, shown in red.
    static func destructive(_ view: ApprovalView) -> Bool {
        // A desktop question says itself: one asked every time for a key or a command it cannot read is not red.
        if let ask = view.ask { return ask.destructive }
        return view.noStanding && view.kind == .write
    }

    /// The request removes something (a repository, an issue, a branch).
    static func deletes(_ view: ApprovalView) -> Bool {
        if view.op.contains("delete") || view.opTitle.hasPrefix("Delete") || (view.mcp?.tool.hasPrefix("delete") ?? false) { return true }
        if let refs = view.git?.refs, !refs.isEmpty, refs.allSatisfy({ $0.change == "delete" }) { return true }
        return false
    }

    static func tone(_ view: ApprovalView) -> HeroTone {
        if destructive(view) { return .danger }
        switch view.kind {
        case .grant, .accounts: return .grant
        case .send: return .send
        case .search, .read, .fetch: return .search
        case .write:
            if view.action == "send" { return .send }
            if ["read", "search", "list"].contains(view.action) { return .search }
            return .write
        }
    }

    static func badge(_ view: ApprovalView) -> HeroBadge {
        if destructive(view) && deletes(view) { return .trash }
        if view.ask != nil { return .question }
        if view.secrets != nil { return .key }
        if view.ssh != nil { return .terminal }
        switch view.kind {
        case .grant: return .shield
        case .accounts: return .people
        case .send: return .send
        case .search: return .search
        case .read: return .read
        case .fetch, .write:
            switch view.action {
            case "list": return .list
            case "search": return .search
            case "read": return .read
            case "send": return .send
            default: return .pen
            }
        }
    }

    /// The service whose logo the hero shows ("mcp" for any MCP server).
    static func service(_ view: ApprovalView) -> String {
        view.service.hasPrefix("mcp:") || view.mcp != nil ? "mcp" : view.service
    }

    /// "Read Telegram messages" → "read Telegram messages"; a name that starts with capitals ("GitHub") stays.
    static func lowerFirst(_ text: String) -> String {
        let chars = Array(text)
        guard chars.count > 1, chars[0].isUppercase, chars[1].isLowercase else { return text }
        return chars[0].lowercased() + String(chars.dropFirst())
    }

    /// "Bank <alerts@bank.com>" → "Bank"; a bare address stays as it is.
    static func displayName(_ from: String) -> String {
        let name = (from.components(separatedBy: "<").first ?? from).trimmingCharacters(in: .whitespaces)
            .trimmingCharacters(in: CharacterSet(charactersIn: "\""))
        if !name.isEmpty { return name }
        return from.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "<>"))
    }

    private static func plural(_ n: Int, _ one: String, _ many: String? = nil) -> String {
        n == 1 ? "1 \(one)" : "\(n) \(many ?? one + "s")"
    }

    /// The hero's one sentence: "Wants to **push 8 commits**\nto **octo/app**". Everything quoted is untrusted.
    static func sentence(_ view: ApprovalView) -> [SentenceSpan] {
        let wants = SentenceSpan(text: "Wants to ")
        func b(_ t: String) -> SentenceSpan { SentenceSpan(text: t, bold: true) }
        func p(_ t: String) -> SentenceSpan { SentenceSpan(text: t) }
        if let git = view.git {
            let repo = b(untrusted(git.repo))
            let refs = git.refs
            if !refs.isEmpty && refs.allSatisfy({ $0.change == "delete" }) {
                let what = refs.count > 1 ? "delete \(refs.count) refs"
                    : refs[0].kind == "tag" ? "delete tag \(untrusted(refs[0].shortName))" : "delete branch \(untrusted(refs[0].shortName))"
                return [wants, b(what), p("\nfrom "), repo]
            }
            if !refs.isEmpty && refs.allSatisfy({ $0.kind == "tag" }) {
                return [wants, b(refs.count == 1 ? "push tag \(untrusted(refs[0].shortName))" : "push \(refs.count) tags"), p("\nto "), repo]
            }
            let commits = refs.filter { $0.change != "delete" }.reduce(0) { $0 + Int($1.commitCount) }
            let what = commits == 0 ? "push \(untrusted(refs.first?.shortName ?? ""))".trimmingCharacters(in: .whitespaces) : "push \(plural(commits, "commit"))"
            return [wants, b(what), p("\nto "), repo]
        }
        if let ask = view.ask { return [b(untrusted(ask.question))] }
        if let s = view.secrets { return [p("Wants "), b(plural(s.items.count, "secret")), p("\nfor "), b(untrusted(s.command))] }
        if let s = view.ssh { return [wants, b("sign in"), p(" to\n"), b(untrusted(sshTarget(host: s.host, hostKey: s.hostKey)))] }
        if let mcp = view.mcp {
            let title = untrusted(mcp.title)
            return [wants, b(lowerFirst(title.isEmpty ? untrusted(mcp.tool) : title)), p("\non "), b(untrusted(mcp.serverName))]
        }
        switch view.kind {
        case .grant:
            return [wants, b(view.grant?.action == "send" ? "send emails" : "read emails"), p("\n"), b("without asking")]
        case .accounts:
            return [p("Wants to see your\n"), b("\(serviceName(view.service.isEmpty ? "gmail" : view.service)) addresses")]
        case .send:
            let to = (view.email?.to ?? []) + (view.email?.cc ?? [])
            var out = [wants, b("send an email")]
            if let first = to.first {
                out += [p("\nto "), b(untrusted(displayName(first)))]
                if to.count > 1 { out.append(p(" and \(to.count - 1) more")) }
            }
            return out
        case .search, .read:
            let n = view.messages.count
            var senders: [String] = []
            for m in view.messages where !senders.contains(displayName(m.from)) { senders.append(displayName(m.from)) }
            let head = [wants, b("read \(plural(n, "email"))")]
            if n > 0 && senders.count == 1 { return head + [p("\nfrom "), b(untrusted(senders[0]))] }
            if let q = view.query, !q.trimmingCharacters(in: .whitespaces).isEmpty { return head + [p("\nfound for "), b(untrusted(q))] }
            return head
        case .fetch, .write:
            let title = lowerFirst(view.titleLine)
            let target = view.resources.first { !$0.wider }.map { untrusted($0.label) }
            if title.hasPrefix("delete "), let target {
                // "Wants to **delete** the repository\n**octo/old-api**": the verb is what matters.
                var noun = String(title.dropFirst("delete ".count))
                for article in ["a ", "an ", "the ", serviceName(view.service) + " "] where noun.hasPrefix(article) {
                    noun = String(noun.dropFirst(article.count))
                }
                return [wants, b("delete"), p(" the \(noun)\n"), b(target)]
            }
            var out = [wants, b(title)]
            if let target { out += [p("\n"), b(target)] }
            return out
        }
    }

    /// The items a search or read releases as the draft stands (ticked, already covered, or all mail but codes).
    static func sharedCount(_ view: ApprovalView, _ draft: ApprovalDraft) -> Int {
        view.messages.filter { $0.coveredByGrant || draft.selected.contains($0.id) || (draft.allMail != nil && !$0.sensitive) }.count
    }

    /// What Approve says it does: "Approve", "Share 2", "Allow", "Yes", "Send", "Delete".
    static func approveLabel(_ view: ApprovalView, _ draft: ApprovalDraft) -> String {
        if view.ask != nil { return "Yes" }
        if view.kind == .grant || view.kind == .accounts { return "Allow" }
        if destructive(view) && deletes(view) { return "Delete" }
        if view.kind == .send { return "Send" }
        if [ApprovalKind.search, .read, .fetch].contains(view.kind) && !view.messages.isEmpty { return "Share \(sharedCount(view, draft))" }
        return "Approve"
    }

    static func denyLabel(_ view: ApprovalView) -> String { view.ask != nil ? "No" : "Deny" }

    /// The remembered approval is "anything like this" (the user's explicit choice), wider than the screen shows.
    static func likeThisOn(_ view: ApprovalView, _ draft: ApprovalDraft) -> Bool {
        draft.likeThis && view.quick?.allow != nil && draft.lifetime != .once && draft.allMail == nil && !view.noStanding
    }

    /// Nothing here may be remembered: the dock says "Asked every time" instead of offering how long.
    static func rememberLocked(_ view: ApprovalView) -> Bool { view.noStanding }

    /// 3600 → "1 hour", 28800 → "8 hours", 86400 → "1 day", 604800 → "1 week", 2592000 → "30 days".
    static func period(_ secs: UInt64) -> String {
        if secs % 604_800 == 0, (1...4).contains(secs / 604_800) { return plural(Int(secs / 604_800), "week") }
        if secs % 86_400 == 0 { return plural(Int(secs / 86_400), "day") }
        if secs % 3_600 == 0 { return plural(Int(secs / 3_600), "hour") }
        return plural(max(Int(secs / 60), 1), "minute")
    }

    /// The dock's second segment: the core's "allow for a while" period ("1 hour", "8 hours"), else an hour.
    static func shortLabel(_ view: ApprovalView) -> String {
        var d = ApprovalDraft()
        d.lifetime = .hour
        return period(lifetimeSeconds(view, d) ?? 3_600)
    }

    /// What the last segment says: "Custom" until something is picked in it, then what ("1 week", "3 days", "Forever").
    static func customLabel(_ draft: ApprovalDraft) -> String {
        if draft.allMail != nil { return "All mail" }
        return sheetChoiceLabel(draft) ?? "Custom"
    }

    /// The lifetime picked in the "Remember for" sheet in a few words; nil for the dock's own (once, 1 hour, 1 day).
    static func sheetChoiceLabel(_ draft: ApprovalDraft) -> String? {
        switch draft.lifetime {
        case .week: "1 week"
        case .month: "30 days"
        case .custom: draft.customUnit.label(draft.customAmount)
        case .uses: plural(draft.uses, "use")
        case .untilRevoked: "Forever"
        case .once, .hour, .day: nil
        }
    }

    /// The lifetimes the last segment's sheet offers (until revoked is set apart, see `offersUntilRevoked`).
    static func sheetLifetimes(_ view: ApprovalView) -> [LifetimeKind] {
        view.kind == .accounts ? [.week, .month] : [.week, .month, .custom, .uses]
    }

    static func offersUntilRevoked(_ view: ApprovalView) -> Bool { view.kind != .accounts }

    /// The sheet's confirm button for `draft`'s lifetime: "Remember for 1 week", "Remember until I revoke".
    static func rememberButton(_ view: ApprovalView, _ draft: ApprovalDraft) -> String {
        switch draft.lifetime {
        case .untilRevoked: "Remember until I revoke"
        case .once: "Done"
        case .hour: "Remember for \(shortLabel(view))"
        case .day: "Remember for 1 day"
        default: "Remember for \(sheetChoiceLabel(draft) ?? "")"
        }
    }

    /// The dock's segments, in order (the last opens the sheet).
    enum Segment { case once, short, day, custom }

    static func segment(_ draft: ApprovalDraft) -> Segment {
        if draft.allMail != nil { return .custom }
        switch draft.lifetime {
        case .once: return .once
        case .hour: return .short
        case .day: return .day
        default: return .custom
        }
    }

    /// A tag on the hero for what cannot be undone, and what its (?) says.
    struct DangerTag: Equatable {
        var label: String
        var id: String
        var help: String
        /// Red, held to approve; else an amber tag on an ordinary question.
        var destructive = true
        var symbol = "exclamationmark.triangle.fill"
    }

    /// The guard rule a desktop question matched, and what matched it: ".env files · .env".
    static func ruleTag(_ rule: AskRuleView, everyTime: Bool = false) -> DangerTag {
        let matched = untrusted(rule.matched)
        let label = matched.isEmpty ? rule.label : "\(rule.label) · \(matched)"
        var help = "Matched the rule \(untrusted(rule.pattern)) on your computer."
        if rule.reported { help += " Your computer says so; this phone doesn't know the rule." }
        help += rule.destructive
            ? " It can't be undone, so it's asked every time."
            : everyTime ? " It's asked every time." : " Autopilot can learn it."
        return DangerTag(label: label, id: "guardRule", help: help, destructive: rule.destructive, symbol: "shield.lefthalf.filled")
    }

    static func dangerTag(_ view: ApprovalView) -> DangerTag? {
        if let rule = view.ask?.rule { return ruleTag(rule, everyTime: view.noStanding) }
        if let git = view.git {
            if let i = git.refs.firstIndex(where: { $0.force && !$0.forceUnknown }) {
                return DangerTag(label: "Rewrites history", id: "forceWarning:\(i)", help: "This push replaces commits that are already there. What they held is lost on the server.")
            }
            if let i = git.refs.firstIndex(where: \.forceUnknown) {
                return DangerTag(label: "History not checked", id: "forceUnknown:\(i)", help: "Reins could not check that this push only adds commits, so it is treated as one that rewrites history.")
            }
            if git.refs.contains(where: { r in r.commits.contains(where: \.hiddenChars) || r.files.contains(where: \.hiddenChars) }) {
                return DangerTag(label: "Hidden characters", id: "hiddenChars", help: hiddenHelp)
            }
        }
        if let mcp = view.mcp, mcp.destructive {
            return DangerTag(label: "Destructive", id: "mcpDestructive", help: "The server marks this tool as destructive. It is asked for every time and can never be allowed in advance.")
        }
        if destructive(view) {
            return DangerTag(
                label: "Can't be undone",
                id: "onceWarning",
                help: deletes(view) ? "Removes it for good. Autopilot never allows this; you're always asked." : "This reaches far or can't be undone. It's asked every time and never remembered."
            )
        }
        return nil
    }

    static let hiddenHelp = "Invisible characters can make code read unlike what it does. They show as \u{FFFD} here."

    /// 40 → "40s", 125 → "3m", 7200 → "2h".
    static func seconds(_ secs: Int64) -> String {
        if secs < 100 { return "\(secs)s" }
        if secs < 3_600 { return "\((secs + 59) / 60)m" }
        return "\((secs + 3_599) / 3_600)h"
    }

    /// `src/auth/login.ts` → (`src/auth/`, `login.ts`).
    static func splitPath(_ path: String) -> (String, String) {
        guard let cut = path.lastIndex(of: "/") else { return ("", path) }
        return (String(path[...cut]), String(path[path.index(after: cut)...]))
    }

    /// Approve, Deny, or Unsure when Autopilot would ask.
    static func verdict(_ v: Verdict) -> String {
        switch v {
        case .approve: "Approve"
        case .deny: "Deny"
        case .ask: "Unsure"
        }
    }

    static func commits(_ n: Int) -> String { plural(n, "commit") }
    static func files(_ n: Int) -> String { plural(n, "file") }
}
