import Foundation

// How a request is written and marked (design/signal/ASK-TEXT.md) and set in the app's type (Typography.swift): the
// kind label ("GitHub · Push"), the sentence in the display face (Sora) with exactly one semibold target (short and
// human: owner/repo, a person, a folder, a file, project · env), and machine details (a command, a branch, a host, a
// path, an address) only on a JetBrains Mono line under it, never in the sentence. Waiting asks are present tense and
// start with the verb: the asker is in the field's top bar. The icon is
// the integration's mark with the action as a badge in the kind's colour. Screens never build the sentence themselves.
//
// The sentence never claims more than what was asked: its verb, counts and target come only from structured fields
// (the push's refs and repository, the tool's name, what the core resolved), never from text an agent, a sender or a
// store wrote. Where reading is uncertain it is the generic sentence, with the full value on the mono line. Names go
// through untrustedName(); a cut name is display-only, and the full value is in a fact row below.

enum AskFormat {
    /// A request that waits, as its field writes it. `core`: the core's own label for it (what a question from the
    /// computer runs or touches, resolved by the core; never read again here). `computer`: the asking computer's name.
    static func waiting(_ view: ApprovalView, core: RequestLabel? = nil, computer: String? = nil) -> AskLine {
        let kind = label(view, core: core)
        let icon = self.icon(view, core: core)
        func line(_ lead: String, _ target: String?, _ after: String = "", mono: String? = nil, stand: String? = nil) -> AskLine {
            AskLine.make(kind: kind, lead: lead, target: target, after: after, mono: mono, stand: stand, icon: icon)
        }
        let service = serviceName(view.service.isEmpty ? "gmail" : view.service)

        if let p = view.purchase {
            // The store's name is what the AI typed; the domain is the page the purchase is on.
            let domain = untrustedName(p.domain)
            let ascii = asciiHost(domain)
            // A host beyond ASCII is shown as punycode; what it was, below.
            let shown = ascii == domain ? nil : "Store: \(rawForm(domain))"
            return line("Pay \(untrusted(p.total)) at ", ascii, mono: [shown, untrusted(p.merchant)].compactMap { $0 }.joined(separator: "\n"), stand: "a store")
        }
        if let git = view.git {
            let repo = untrustedName(git.repo, limit: .max)
            let refs = git.refs
            let mono = refs.map(refWords).joined(separator: ", ")
            if !refs.isEmpty && refs.allSatisfy({ $0.change == "delete" }) {
                let what = refs.count > 1 ? "Delete \(refs.count) refs from " : refs[0].kind == "tag" ? "Delete a tag from " : "Delete a branch from "
                return line(what, repo, mono: mono, stand: "a repository")
            }
            if !refs.isEmpty && refs.allSatisfy({ $0.kind == "tag" }) {
                return line(refs.count == 1 ? "Push a tag to " : "Push \(refs.count) tags to ", repo, mono: mono, stand: "a repository")
            }
            let commits = refs.filter { $0.change != "delete" }.reduce(0) { $0 + Int($1.commitCount) }
            let verb = refs.contains(where: \.force) ? "Force-push" : "Push"
            return line(commits == 0 ? "\(verb) to " : "\(verb) \(plural(commits, "commit")) to ", repo, mono: mono, stand: "a repository")
        }
        if let ask = view.ask {
            // A change of Autopilot asked from a computer: the core words the question itself from the phone's own state
            // ("Turn on Auto and leave Lockdown?"), so it is Reins' sentence; the computer is only who asks.
            if ApprovalWords.autopilotChange(view) { return line(ask.question, nil) }
            if core?.provider == "terminal", let command = core?.command {
                // The complete actionable command is the first fact, outside the fixed field.
                return line(command.heading, nil)
            }
            let place = computer.flatMap { $0.isEmpty ? nil : untrustedName($0) } ?? "your computer"
            // What it runs or touches is the core's reading (its label, the guard's match), never read again here.
            // Not read for sure, or not resolved: the generic sentence, and everything that was asked below it.
            let full = [ask.question, ask.detail ?? ""].map(untrusted).filter { !$0.isEmpty }.joined(separator: "\n")
            guard !ask.uncertain, let core else { return line("Answer a question from ", place, mono: full) }
            let subject = untrusted(core.subject + core.tail)
            switch core.provider {
            case "terminal":
                let matched = ask.rule.map { untrusted($0.matched) }.flatMap { $0.isEmpty ? nil : $0 }
                return line("Run a command on ", place, mono: matched ?? subject)
            case "file":
                return line(core.action == "read" ? "Read a file on " : "Change a file on ", place, mono: subject)
            default:
                return line("Answer a question from ", place, mono: untrusted(ask.question))
            }
        }
        if let mcp = view.mcp {
            // The tool's title is the server's own description of it: the tool's name is what runs.
            return line("Use a tool on ", untrustedName(mcp.serverName), mono: untrusted(mcp.tool), stand: "a server")
        }
        if let r = view.envRelease {
            let what = r.allVariables ? "every variable" : r.variables.count == 1 ? "a variable" : plural(r.variables.count, "variable")
            return line("\(r.print ? "Print" : "Release") \(what) of ", "\(untrustedName(r.project)) · \(untrustedName(r.env))", mono: untrusted(r.command),
                        stand: "a project")
        }
        if let s = view.secrets {
            return line("Release \(plural(s.items.count, "secret")) from ", "your vault", mono: untrusted(s.command))
        }
        if let s = view.secretsStore {
            let envs = s.envs.map { "\(untrusted($0.project)) · \(untrusted($0.env))" }.joined(separator: ", ")
            return line("Save \(plural(Int(s.total), "secret")) in ", "your vault", mono: envs)
        }
        if let t = view.tokenStore {
            return line("Save a ", serviceName(t.service), " token", mono: asciiHost(untrusted(t.host)))
        }
        if let s = view.ssh {
            return line("Sign in with ", untrustedName(s.keyName), mono: untrusted(sshTarget(host: s.host.map(asciiHost), hostKey: s.hostKey)))
        }
        switch view.kind {
        case .grant:
            let send = view.grant?.action == "send"
            let lines = (view.grant?.lines ?? []).map(untrusted).joined(separator: " · ")
            return line(send ? "Send email from " : "Read email in ", "your \(service)", " without asking", mono: lines)
        case .accounts:
            return line("See your ", service, " addresses")
        case .send:
            // A recipient's display name is what the AI wrote: the addresses themselves go below.
            let to = (view.email?.to ?? []) + (view.email?.cc ?? [])
            let lead = to.count > 1 ? "Send an email to \(to.count) people from " : "Send an email from "
            return line(lead, "your \(service)", mono: to.map { asciiAddress(untrusted($0)) }.joined(separator: ", "))
        case .search, .read:
            // A sender's name is what the sender wrote: the search, or the addresses, go below.
            let n = view.messages.count
            let query = view.query.map(untrusted).flatMap { $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 }
            var senders: [String] = []
            for m in view.messages where !senders.contains(m.from) { senders.append(m.from) }
            return line("Read \(plural(n, "email")) from ", "your \(service)", mono: query ?? senders.map(untrusted).joined(separator: ", "))
        case .fetch, .write:
            let title = capitalised(ApprovalWords.lowerFirst(untrusted(view.titleLine)))
            let target = view.resources.first { !$0.wider }.map { untrustedName($0.label) }
            if title.hasPrefix("Delete "), let target {
                // "Delete the repository octo/old-api": the verb is what matters.
                var noun = String(title.dropFirst("Delete ".count))
                for article in ["a ", "an ", "the ", serviceName(view.service) + " "] where noun.hasPrefix(article) {
                    noun = String(noun.dropFirst(article.count))
                }
                return line("Delete the \(noun) ", target)
            }
            guard let target else { return line(title, nil) }
            return line("\(title) in ", readsOn(target))
        }
    }

    /// The label line over the ask: what it touches, then what it does ("GitHub · Push", "Payments · Buy").
    static func label(_ view: ApprovalView, core: RequestLabel? = nil) -> String {
        let place: String
        if let mcp = view.mcp {
            place = untrustedName(mcp.serverName)
        } else if view.purchase != nil {
            place = "Payments"
        } else if ApprovalWords.autopilotChange(view) {
            place = "Autopilot"
        } else if view.ask != nil {
            place = "Computer"
        } else if view.secrets != nil || ApprovalWords.isSecrets(view) {
            place = "Vault"
        } else if view.ssh != nil {
            place = "SSH"
        } else {
            place = serviceName(view.service.isEmpty ? "gmail" : view.service)
        }
        return "\(place) · \(what(view, core: core))"
    }

    private static func what(_ view: ApprovalView, core: RequestLabel?) -> String {
        if view.mcp != nil { return "MCP tool" }
        if view.purchase != nil { return "Buy" }
        if ApprovalWords.autopilotChange(view) { return view.op == "autopilot_mode" ? "Mode" : "Correct" }
        if let ask = view.ask {
            guard !ask.uncertain, let core else { return "Asks" }
            switch core.provider {
            case "terminal": return "Run"
            case "file": return "Files"
            default: return "Asks"
            }
        }
        if view.secrets != nil || view.envRelease != nil { return "Secrets" }
        if view.secretsStore != nil || view.tokenStore != nil { return "Save" }
        if view.ssh != nil { return "Sign in" }
        if view.kind == .grant { return "Permission" }
        if view.kind == .accounts { return "Accounts" }
        if ApprovalWords.deletes(view) { return "Delete" }
        if view.git != nil { return "Push" }
        switch view.kind {
        case .send: return "Send"
        case .search: return "Search"
        case .read: return "Read"
        case .fetch: return "Fetch"
        default: break
        }
        switch ApprovalWords.badge(view) {
        case .search: return "Search"
        case .read: return "Read"
        case .list: return "List"
        case .send: return "Send"
        case .trash: return "Delete"
        case .terminal: return "Run"
        default: return "Change"
        }
    }

    /// The request's icon: the integration's mark (or a local glyph) and the action, in the request's field colour.
    static func icon(_ view: ApprovalView, core: RequestLabel? = nil) -> AskIcon {
        let field = ApprovalWords.tone(view).field
        let deletes = ApprovalWords.deletes(view)
        func spec(_ mark: AskIcon.Mark, _ action: Glyph) -> AskIcon { AskIcon(mark: mark, action: deletes ? .trash : action, kind: field) }
        if view.purchase != nil { return spec(.glyph(.card), .send) }
        if view.git != nil { return spec(.service(view.service.isEmpty ? "github" : view.service), .gitBranch) }
        if let ask = view.ask {
            guard !ask.uncertain, let core else { return spec(.glyph(.help), .help) }
            switch core.provider {
            case "terminal": return spec(.glyph(.terminal), core.action == "delete" || ask.destructive ? .trash : .play)
            case "file": return spec(.glyph(.file), core.action == "read" ? .eye : .pencil)
            default: return spec(.glyph(.help), .help)
            }
        }
        if view.mcp != nil { return spec(.service("mcp"), view.mcp?.readOnly == true ? .eye : .pencil) }
        if view.envRelease != nil || view.secrets != nil { return spec(.glyph(.lock), .key) }
        if view.secretsStore != nil { return spec(.service("vault"), .plus) }
        if view.tokenStore != nil { return spec(.glyph(.key), .plus) }
        if view.ssh != nil { return spec(.glyph(.key), .lockOpen) }
        let mark = AskIcon.Mark.service(ApprovalWords.service(view).isEmpty ? "gmail" : ApprovalWords.service(view))
        switch view.kind {
        case .grant: return spec(mark, .shieldCheck)
        case .accounts: return spec(mark, .person)
        case .send: return spec(mark, .send)
        case .search: return spec(mark, .search)
        case .read, .fetch: return spec(mark, view.action == "search" ? .search : .eye)
        case .write:
            switch ApprovalWords.badge(view) {
            case .send: return spec(mark, .send)
            case .search, .list: return spec(mark, .search)
            case .read: return spec(mark, .eye)
            default: return spec(mark, .pencil)
            }
        }
    }

    /// One ref of a push, as the mono line shows it: "feature/x · new branch", "v1.2.0 · new tag", "old · delete".
    static func refWords(_ r: GitRefView) -> String {
        var words = untrusted(r.shortName)
        if r.change == "delete" {
            words += " · delete"
        } else if r.change == "create" {
            words += r.kind == "tag" ? " · new tag" : " · new branch"
        } else if r.force {
            words += r.forceUnknown ? " · history not checked" : " · force"
        }
        return words
    }

    /// A resource's own label reads on in the sentence ("… in branch main of octo/app"); a name ("Family") stays.
    private static func readsOn(_ label: String) -> String {
        let generic = ["Branch ", "Any ", "Every ", "Repository ", "Folder ", "File ", "Chat ", "Issue ", "Project ", "Label "]
        return generic.contains { label.hasPrefix($0) } ? ApprovalWords.lowerFirst(label) : label
    }

    private static func capitalised(_ text: String) -> String {
        guard let first = text.first else { return text }
        return first.uppercased() + text.dropFirst()
    }

    private static func plural(_ n: Int, _ one: String, _ many: String? = nil) -> String {
        n == 1 ? "1 \(one)" : "\(n) \(many ?? one + "s")"
    }
}
