import SwiftUI

// The content under the field (the Android app's ApprovalFacts.kt; Next-Approve, Next-Kinds): raised groups of rows,
// a key word and its value (the kit's `kv`) or a glyph and its value (`fact`), long content folded into rows that
// open a page, by kind of request. What is approved (an address, a command, a path, a
// host, a secret's name, a tool's arguments) wraps and is never cut; only labels and titles keep a line limit.

/// The facts of the request, by kind.
struct Facts: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(AppModel.self) private var app
    @Environment(\.approvalRecord) private var record

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            switch view.kind {
            case .grant: GrantFacts(view: view)
            case .accounts: AccountsFacts(vm: vm, view: view)
            case .send: SendFacts(view: view)
            case .write:
                if let purchase = view.purchase {
                    PurchaseFacts(purchase: purchase)
                } else if let git = view.git {
                    GitFacts(view: view, git: git)
                } else if let mcp = view.mcp {
                    McpFacts(call: mcp)
                } else if let ask = view.ask, ApprovalWords.autopilotChange(view) {
                    AutopilotChangeFacts(ask: ask)
                } else if let ask = view.ask {
                    if record?.label?.command == nil {
                        AskFacts(ask: ask, command: app.pending.first { $0.id == view.requestId }?.label?.command)
                    }
                } else if let release = view.envRelease {
                    EnvReleaseFacts(vm: vm, release: release)
                } else if let store = view.secretsStore {
                    SecretsStoreFacts(store: store)
                } else if let token = view.tokenStore {
                    TokenStoreFacts(token: token)
                } else if let secrets = view.secrets {
                    SecretsFacts(secrets: secrets)
                } else if let ssh = view.ssh {
                    SshFacts(ssh: ssh)
                } else {
                    WriteFacts(view: view)
                }
                if let blob = view.blob { AttachedFileFacts(blob: blob) }
            case .search, .read, .fetch:
                MessagesFacts(vm: vm, view: view)
            }
            PublicDomainNote(vm: vm, view: view)
        }
    }
}

// MARK: Groups and rows

/// A group of rows (the kit's `grp`): raised paper, radius 20, 16 pt in from the sides, a hairline between rows from
/// their text. `border` is kept for callers that tint a group with a warning; the warning itself is said in its rows.
struct FactGroup<Content: View>: View {
    var border: Color?
    @ViewBuilder var content: Content

    var body: some View {
        RaisedGroup(inset: 16) { content }
    }
}

/// "To   ann@corp.example": a key word on the left (13.5 medium, meta) and its value after it (15.5 medium, or mono
/// 14), the kit's `kv`.
struct FactRow<Trailing: View>: View {
    var key: String
    var value: String
    var mono = false
    var valueId: String?
    var tint: Color?
    /// Nil: the value wraps and is never cut (what is approved). A limit only for what the AI says about it.
    var lines: Int? = nil
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(key)
                .font(RFont.sans(13.5, .medium))
                .foregroundStyle(Palette.inkMeta)
                .lineLimit(2)
                .frame(minWidth: 62, alignment: .leading)
                .fixedSize(horizontal: true, vertical: false)
            Text(value)
                .font(mono ? RFont.mono(14) : RFont.sans(15.5, .medium))
                .foregroundStyle(tint ?? Palette.ink)
                .lineLimit(lines)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .environment(\.layoutDirection, mono ? .leftToRight : .leftToRight)
                .accessibilityIdentifier(valueId ?? "")
                .textSelection(.enabled)
            trailing
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(minHeight: 50)
        .accessibilityElement(children: .combine)
    }
}

extension FactRow where Trailing == EmptyView {
    init(key: String, value: String, mono: Bool = false, valueId: String? = nil, tint: Color? = nil, lines: Int? = nil) {
        self.init(key: key, value: value, mono: mono, valueId: valueId, tint: tint, lines: lines) { EmptyView() }
    }
}

/// A fact whose glyph says what it is (the kit's `fact`): the account, a server.
struct FactLine: View {
    var glyph: Glyph
    var value: String
    var mono = false
    var valueId: String?

    var body: some View {
        HStack(spacing: 12) {
            GlyphIcon(glyph, size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20)
            Text(value)
                .font(mono ? RFont.mono(14) : RFont.sans(15.5, .medium))
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .environment(\.layoutDirection, .leftToRight)
                .accessibilityIdentifier(valueId ?? "")
                .textSelection(.enabled)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(minHeight: 50)
        .accessibilityElement(children: .combine)
    }
}

/// A row that opens a page: a title, a short detail on the right, a chevron.
struct DrillRow<Detail: View>: View {
    var title: String
    var id: String
    var page: ApprovalPage
    /// "+5 more": 15 pt in ink-2.
    var quiet = false
    @ViewBuilder var detail: Detail
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            nav.path.append(page)
            feedback.play(.open)
        } label: {
            HStack(spacing: 12) {
                Text(title).font(RFont.sans(quiet ? 15 : 16, .medium)).foregroundStyle(quiet ? Palette.ink2 : Palette.ink)
                Spacer(minLength: 0)
                detail
                GlyphIcon(.chevronRight, size: 14).foregroundStyle(Palette.inkFaint)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .frame(minHeight: quiet ? 46 : 50)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowButtonStyle())
        .accessibilityIdentifier(id)
    }
}

/// "+412 −96" in green and red, monospaced.
struct Counts: View {
    var additions: UInt64?
    var deletions: UInt64?
    var size: CGFloat = 14

    var body: some View {
        HStack(spacing: 6) {
            if let additions { Text("+\(additions)").foregroundStyle(Palette.success) }
            if let deletions { Text("−\(deletions)").foregroundStyle(Palette.danger) }
        }
        .font(RFont.mono(size, .medium))
        .lineLimit(1)
    }
}

/// A section's header over its group (the kit's `sec`): an optional glyph, a few words in 13 semibold ink-2, and an
/// action, counts or a (?) at the right.
struct FactHeader<Trailing: View>: View {
    var text: String
    var glyph: Glyph? = nil
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 6) {
            if let glyph { GlyphIcon(glyph, size: 15) }
            Text(text)
                .font(RFont.sans(13, .semibold))
                .tracking(13 * TextRole.section.trackingEm)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 8)
            trailing
        }
        .foregroundStyle(Palette.ink2)
        .padding(.horizontal, 24)
        .padding(.top, 2)
        .frame(minHeight: 22)
    }
}

extension FactHeader where Trailing == EmptyView {
    init(_ text: String, glyph: Glyph? = nil) { self.init(text: text, glyph: glyph) { EmptyView() } }
}

/// The kit's check box: 24 pt, radius 8, violet when ticked, a ring when not; dimmed when it can't change.
struct TickBox: View {
    var checked: Bool
    var enabled = true

    var body: some View {
        RoundedRectangle(cornerRadius: 8, style: .continuous)
            .fill(checked ? Palette.violet : Color.clear)
            .overlay {
                if !checked { RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.inkFaint, lineWidth: 2) }
            }
            .overlay { if checked { GlyphIcon(.check, size: 14).foregroundStyle(.white) } }
            .frame(width: 24, height: 24)
            .opacity(enabled ? 1 : 0.5)
            .animation(.spring(duration: 0.22), value: checked)
    }
}

// MARK: Emails and items found

/// The emails (or items) found (Next-Kinds 01, 06): ticking is the decision; a code is never ticked for you, one a
/// grant covers is ticked, dimmed and tagged Grant. The search itself is the field's mono line.
private struct MessagesFacts: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.feedback) private var feedback

    var body: some View {
        let draft = vm.draft
        if view.messages.isEmpty {
            FactGroup { FactRow(key: "Found", value: "Nothing") }
        } else {
            let allTicked = view.messages.allSatisfy { $0.coveredByGrant || draft.selected.contains($0.id) }
            if vm.readOnly {
                // An Activity entry: what was shared is ticked, and nothing can be ticked again.
                let shared = view.messages.filter { $0.coveredByGrant || draft.selected.contains($0.id) }.count
                FactHeader(AttentionText.mailHeader(view.messages, service: view.service, shared: shared), glyph: .search)
                    .accessibilityIdentifier("messagesHeader")
            } else {
                FactHeader(text: AttentionText.mailHeader(view.messages, service: view.service), glyph: .search) {
                    Button(allTicked ? "Clear" : "Select all") {
                        vm.draft.selected = allTicked ? [] : Set(view.messages.map(\.id))
                        vm.draft.allMail = nil
                        feedback.play(.tap)
                    }
                    .font(RFont.sans(13, .semibold))
                    .foregroundStyle(Palette.violet)
                    .frame(minHeight: 44)
                    .accessibilityIdentifier(allTicked ? "clearAll" : "selectAll")
                }
                .padding(.vertical, -11)
            }
            FactGroup {
                // Flagged first: rows are keyed by id, so the order on screen is all that changes.
                ForEach(AttentionText.ordered(view.messages) { AttentionText.mail($0, service: view.service)?.level }.map(\.element), id: \.id) { m in
                    MailRow(
                        message: m,
                        checked: m.coveredByGrant || draft.selected.contains(m.id) || (draft.allMail != nil && !m.sensitive),
                        enabled: !vm.readOnly && !m.coveredByGrant && (draft.allMail == nil || m.sensitive),
                        service: view.service
                    ) { on in
                        if on { vm.draft.selected.insert(m.id) } else { vm.draft.selected.remove(m.id) }
                    }
                }
            }
        }
        if view.kind == .fetch, let account = view.account, !account.isEmpty {
            FactGroup { FactRow(key: "Account", value: untrusted(account), mono: true) }
        }
    }
}

/// One email (or item): its tick, subject, who sent it, the date, a Grant tag when a grant covers it, and a flag when
/// it looks like a code.
private struct MailRow: View {
    var message: MessageView
    var checked: Bool
    var enabled: Bool
    var service: String
    var onChange: (Bool) -> Void
    @Environment(\.feedback) private var feedback
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let m = message
        let subject = untrusted(m.subject)
        let title = !subject.isEmpty ? subject : (!untrusted(m.snippet).isEmpty ? untrusted(m.snippet) : untrusted(m.from))
        let gmail = service.isEmpty || service == "gmail"
        // Mail says who; an item elsewhere (a login, a file) says what it is.
        let meta = !gmail && !subject.isEmpty && !m.snippet.isEmpty
            ? untrusted(m.snippet)
            : (subject.isEmpty ? "" : untrusted(ApprovalWords.displayName(m.from)))
        Button {
            guard enabled else { return }
            onChange(!checked)
            feedback.play(.toggle(!checked))
        } label: {
            HStack(alignment: .center, spacing: 12) {
                TickBox(checked: checked, enabled: enabled)
                VStack(alignment: .leading, spacing: 4) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(title).font(RFont.sans(16, .medium)).foregroundStyle(Palette.ink).lineLimit(typeSize.isAccessibilitySize ? nil : 2)
                        if !meta.isEmpty { Text(meta).font(RFont.sans(13)).foregroundStyle(Palette.inkMeta).lineLimit(1) }
                    }
                    .opacity(checked ? 1 : 0.5)
                    // One chip: a code or a password keeps its word ("Code") among the other reasons.
                    if let mark = AttentionText.mail(m, service: service) {
                        AttentionTag(mark: mark, id: m.sensitive ? "sensitive:\(m.id)" : "attention:\(m.id)")
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if m.coveredByGrant {
                    WarnTag(text: "Grant", tint: Palette.violet, glyph: .key).accessibilityLabel("Already allowed by a grant")
                }
                if m.date != 0 {
                    Text(Self.shortDay(m.date))
                        .textRole(.meta).foregroundStyle(Palette.inkMeta).lineLimit(1)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        // Not `disabled`: one a grant covers stays readable at full ink; only its box is dimmed.
        .allowsHitTesting(enabled)
        .accessibilityAddTraits(checked ? .isSelected : [])
        .accessibilityValue(checked ? "Ticked" : "Not ticked")
        .accessibilityHint(enabled ? "" : "Can't be changed")
        .accessibilityIdentifier("message:\(m.id)")
    }

    /// "Oct 23", with the year only when it isn't this one.
    static func shortDay(_ epochSeconds: Int64) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(epochSeconds))
        let thisYear = Calendar.current.isDate(d, equalTo: Date(timeIntervalSince1970: TimeInterval(Timers.nowSeconds())), toGranularity: .year)
        return thisYear ? d.formatted(.dateTime.month(.abbreviated).day()) : d.formatted(.dateTime.month(.abbreviated).day().year())
    }
}

// MARK: An email to send

/// An email to send (Next-Kinds 02): who it goes to, then the email itself (subject over its every word), then who
/// it is from.
private struct SendFacts: View {
    var view: ApprovalView

    var body: some View {
        if let email = view.email {
            FactGroup {
                FactRow(key: "To", value: email.to.map(untrusted).joined(separator: ", "), mono: true)
                if !email.cc.isEmpty { FactRow(key: "Cc", value: email.cc.map(untrusted).joined(separator: ", "), mono: true) }
            }
            .accessibilityContainer("emailPreview")
            FactGroup {
                VStack(alignment: .leading, spacing: 6) {
                    Text(untrusted(email.subject)).font(RFont.sans(15, .semibold)).foregroundStyle(Palette.ink)
                        .accessibilityIdentifier("emailSubject")
                    Text(untrusted(email.body))
                        .font(RFont.sans(15))
                        .lineHeight(.multiple(factor: 21 / 15))
                        .foregroundStyle(Palette.ink2)
                        .accessibilityIdentifier("emailBody")
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .textSelection(.enabled)
            }
            if let account = view.account { FactGroup { FactRow(key: "From", value: untrusted(account), mono: true) } }
        }
    }
}

/// An email to be sent, in full: who it goes to, the subject and every word of it (an activity entry).
struct EmailPreview: View {
    var email: EmailView
    var framed = true

    var body: some View {
        if framed {
            Card(padding: 16) { fields }.accessibilityContainer("emailPreview")
        } else {
            fields.padding(16).frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var fields: some View {
        VStack(alignment: .leading, spacing: 4) {
            Caption("To")
            ForEach(Array(email.to.enumerated()), id: \.offset) { _, a in address(a) }
            if !email.cc.isEmpty {
                Caption("Cc").padding(.top, 6)
                ForEach(Array(email.cc.enumerated()), id: \.offset) { _, a in address(a) }
            }
            Caption("Subject").padding(.top, 6)
            Text(untrusted(email.subject)).font(RFont.sans(16, .semibold)).foregroundStyle(Palette.ink)
            Caption("Message").padding(.top, 6)
            Text(untrusted(email.body)).font(RFont.sans(15)).foregroundStyle(Palette.ink).lineSpacing(3)
        }
        .textSelection(.enabled)
    }

    private func address(_ text: String) -> some View {
        Text(untrusted(text)).font(RFont.mono(14)).foregroundStyle(Palette.ink).environment(\.layoutDirection, .leftToRight)
    }
}

// MARK: A change elsewhere

/// What will be done in another integration, line by line; nothing happens until approved.
private struct WriteFacts: View {
    var view: ApprovalView

    var body: some View {
        if !view.preview.isEmpty {
            FactGroup {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(view.preview.enumerated()), id: \.offset) { i, line in
                        Text(untrusted(line))
                            .font(i == 0 ? RFont.sans(16, .semibold) : RFont.sans(15))
                            .foregroundStyle(i == 0 ? Palette.ink : Palette.ink2)
                            .lineSpacing(2)
                            .accessibilityIdentifier("previewLine:\(i)")
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 18)
                .padding(.vertical, 14)
                .textSelection(.enabled)
            }
            .accessibilityContainer("writePreview")
        }
        if let account = view.account, !account.isEmpty { FactGroup { FactRow(key: "Account", value: untrusted(account), mono: true) } }
    }
}

// MARK: A permission, accounts

/// A permission the AI asked for (Next-Kinds 04): what only, how far it reaches, what it says it is for; how long is
/// the dock's. Nothing is shared now (its (?) says so).
private struct GrantFacts: View {
    var view: ApprovalView

    var body: some View {
        if let grant = view.grant {
            let (tone, reach): (Color, String) = switch grant.breadth {
            case "everything": (Palette.danger, "Everything")
            case "broad": (Palette.warning, "Broad")
            default: (Palette.success, "Narrow")
            }
            FactGroup(border: Palette.violet.opacity(0.35)) {
                ForEach(Array(grant.lines.enumerated()), id: \.offset) { _, line in FactRow(key: "Only", value: untrusted(line), mono: true) }
                FactRow(key: "Reach", value: reach, tint: tone) {
                    HelpDot(id: "grant", text: "Nothing is shared now. Requests that match are answered without asking until it ends. Revoke it any time in Grants.", size: 28)
                }
                if let uses = grant.maxUses { FactRow(key: "Uses", value: "At most \(uses)") }
                if !grant.reason.trimmingCharacters(in: .whitespaces).isEmpty {
                    FactRow(key: "Says", value: "“\(untrusted(grant.reason))”", lines: 3)
                }
            }
            .accessibilityContainer("grantCard")
            if let account = view.account { FactGroup { FactRow(key: "Account", value: untrusted(account), mono: true) } }
        }
    }
}

/// The accounts an AI would see: tick the ones to share; those it already sees are ticked and locked.
private struct AccountsFacts: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.feedback) private var feedback

    var body: some View {
        let who = untrusted(view.connectionLabel)
        FactHeader(text: "Addresses") {
            HelpDot(id: "accounts", text: "\(who) sees only the addresses you tick, nothing in them, and how many it did not get.")
        }
        FactGroup {
            ForEach(Array(view.accounts.enumerated()), id: \.element) { i, address in
                let locked = view.sharedAccounts.contains(address)
                let ticked = locked || vm.draft.selected.contains(address)
                Button {
                    if ticked { vm.draft.selected.remove(address) } else { vm.draft.selected.insert(address) }
                    feedback.play(.toggle(!ticked))
                } label: {
                    HStack(spacing: 14) {
                        TickBox(checked: ticked, enabled: !locked && !vm.readOnly)
                        HStack(spacing: 12) {
                            BlobAvatar(seed: address, size: 32)
                            Text(untrusted(address))
                                .font(RFont.sans(15.5, .medium))
                                .foregroundStyle(Palette.ink)
                                .strikethrough(!ticked, color: Palette.ink2)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .opacity(ticked ? 1 : 0.42)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        if locked { Image(systemName: "lock.fill").font(.system(size: 13)).foregroundStyle(Palette.inkMeta).accessibilityLabel("Already shared") }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .contentShape(Rectangle())
                    .animation(.easeInOut(duration: 0.28), value: ticked)
                }
                .buttonStyle(.plain)
                .disabled(locked || vm.readOnly)
                .accessibilityAddTraits(ticked ? .isSelected : [])
                .accessibilityIdentifier("shared:\(i)")
            }
        }
        .accessibilityContainer("accountsCard")
    }
}

// MARK: A tool of an MCP server, the desktop app

/// A tool of an MCP server (Next-Kinds 05): the tool's own title over its arguments, each a key and its value, whether
/// it changes things, and the server it runs on with its address.
private struct McpFacts: View {
    var call: McpCallView

    var body: some View {
        let title = untrusted(call.title)
        FactHeader(text: title.isEmpty ? untrusted(call.tool) : title, glyph: .terminal) {
            WarnTag(text: call.readOnly ? "Read only" : "Changes things", tint: call.readOnly ? Palette.success : Palette.warning,
                    glyph: call.readOnly ? .eye : .pencil)
                .accessibilityIdentifier("mcpEffect")
        }
        .accessibilityIdentifier("mcpTool")
        FactGroup {
            if !call.description.trimmingCharacters(in: .whitespaces).isEmpty {
                Text(untrusted(call.description)).font(RFont.sans(13.5)).foregroundStyle(Palette.ink2).lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
            }
            if let args = McpArguments.pairs(call.argumentsJson) {
                ForEach(Array(args.enumerated()), id: \.offset) { _, pair in
                    FactRow(key: untrustedName(pair.key, limit: 24), value: pair.value, mono: pair.mono)
                }
            } else {
                let args = untrusted(call.argumentsJson)
                MonoBlock(text: args.isEmpty ? "{}" : args, id: "mcpArgumentsText")
            }
        }
        .accessibilityContainer("mcpArguments")
        FactGroup {
            FactLine(glyph: .server, value: "\(untrustedName(call.serverName)) · \(mcpHostLabel(call.serverUrl))", valueId: "mcpServer")
        }
        .accessibilityContainer("mcpCall")
    }
}

/// A tool's arguments as rows: a JSON object's keys (sorted) with their values; text as it is, numbers, true/false
/// and nested values in mono. Nil when the arguments are not an object, so they show exactly as sent.
enum McpArguments {
    struct Pair: Equatable {
        var key: String
        var value: String
        var mono: Bool
    }

    static func pairs(_ json: String) -> [Pair]? {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any], !object.isEmpty else { return nil }
        // In the order sent: where each key first appears in the text (ties, and keys not found, by name).
        let position = { (key: String) in json.range(of: "\"\(key)\"").map { json.distance(from: json.startIndex, to: $0.lowerBound) } ?? Int.max }
        return object.keys.sorted { (position($0), $0) < (position($1), $1) }.map { key in
            let value = object[key]
            if let text = value as? String { return Pair(key: key, value: untrusted(text), mono: false) }
            if let number = value as? NSNumber {
                let bool = CFGetTypeID(number) == CFBooleanGetTypeID()
                return Pair(key: key, value: bool ? (number.boolValue ? "true" : "false") : number.stringValue, mono: true)
            }
            if value is NSNull { return Pair(key: key, value: "null", mono: true) }
            let nested = (try? JSONSerialization.data(withJSONObject: value ?? NSNull(), options: [.sortedKeys, .fragmentsAllowed]))
                .flatMap { String(data: $0, encoding: .utf8) } ?? ""
            return Pair(key: key, value: untrusted(nested), mono: true)
        }
    }
}

private struct AskFacts: View {
    var ask: AskView
    var command: CommandPresentation?

    var body: some View {
        let detail = ask.detail.map(untrusted).flatMap { $0.isEmpty ? nil : $0 }
        let topic = ask.topic.map(untrusted).flatMap { $0.isEmpty ? nil : $0 }
        if let command {
            CommandFacts(presentation: command)
            let original = command.original.isEmpty ? detail : untrusted(command.original)
            if let original {
                FactGroup {
                    DrillRow(title: "Original command", id: "fullInvocation", page: .text(title: "Original command · may contain secrets", body: original, mono: true, id: "fullInvocationText")) {
                        Text("Includes setup and environment values").font(RFont.sans(12)).foregroundStyle(Palette.inkMeta)
                    }
                }
            }
        } else if detail != nil || topic != nil {
            FactGroup {
                if let detail { MonoBlock(text: detail, id: "askDetail") }
                if let topic { FactRow(key: "About", value: topic, mono: true, valueId: "askTopic") }
            }
            .accessibilityContainer("ask")
        }
    }
}

/// What a change of Autopilot asked from a computer means, in the core's words (who asked, what it changes, why only
/// the owner can), as text: it is the phone's own explanation, not the computer's.
private struct AutopilotChangeFacts: View {
    var ask: AskView

    var body: some View {
        if let detail = ask.detail.map(untrusted).flatMap({ $0.isEmpty ? nil : $0 }) {
            FactGroup {
                Text(detail)
                    .textRole(.body)
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    .accessibilityIdentifier("askDetail")
            }
            .accessibilityContainer("ask")
        }
    }
}

/// Core-derived facts: the clients never reinterpret a shell command or turn its summary into permission.
struct CommandFacts: View {
    var presentation: CommandPresentation
    @State private var showSetup = false

    private var notices: [String] {
        presentation.notices.map(untrusted).filter { !$0.isEmpty }.reduce(into: []) { result, notice in
            if !result.contains(notice) { result.append(notice) }
        }
    }

    var body: some View {
        FactGroup {
            if !presentation.actionable.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Command").font(RFont.sans(15)).foregroundStyle(Palette.inkMeta)
                    Text(untrusted(presentation.actionable)).font(RFont.mono(13.5)).foregroundStyle(Palette.ink)
                        .environment(\.layoutDirection, .leftToRight).textSelection(.enabled)
                        .accessibilityIdentifier("askCommand")
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 12)
            }
            ForEach(Array(notices.enumerated()), id: \.offset) { index, notice in
                Text(notice).font(RFont.sans(14)).foregroundStyle(Palette.ink2)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 12)
                    .accessibilityIdentifier("commandNotice:\(index)")
            }
            if !presentation.setup.isEmpty {
                Button {
                    showSetup.toggle()
                } label: {
                    HStack(spacing: 12) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text("Technical details").font(RFont.sans(15.5, .medium)).foregroundStyle(Palette.ink)
                            Text("Environment and tool configuration · \(presentation.setup.count) items")
                                .font(RFont.sans(12)).foregroundStyle(Palette.inkMeta)
                        }.frame(maxWidth: .infinity, alignment: .leading)
                        Image(systemName: showSetup ? "chevron.up" : "chevron.down")
                            .font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.inkMeta)
                    }.padding(.horizontal, 18).padding(.vertical, 13).contentShape(Rectangle())
                }.buttonStyle(RowButtonStyle()).accessibilityIdentifier("commandTechnicalDetails")
                    .accessibilityValue(showSetup ? "Expanded" : "Collapsed")
                if showSetup {
                    Text(untrusted(presentation.setup.joined(separator: "\n"))).font(RFont.mono(13)).foregroundStyle(Palette.ink)
                        .environment(\.layoutDirection, .leftToRight).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 12)
                        .accessibilityIdentifier("commandSetup")
                }
            }
        }.accessibilityContainer("commandPresentation").onChange(of: presentation) { _, _ in showSetup = false }
    }
}

private struct SecretsFacts: View {
    var secrets: SecretReleaseView

    var body: some View {
        FactGroup {
            FactRow(key: "For", value: untrusted(secrets.command), mono: true, valueId: "secretsCommand")
            if let purpose = secrets.purpose.map(untrusted), !purpose.isEmpty {
                FactRow(key: "Says", value: "“\(purpose)”", lines: 2)
            }
            ForEach(Array(secrets.items.enumerated()), id: \.offset) { i, item in
                HStack(spacing: 12) {
                    Image(systemName: "key.fill").font(.system(size: 14, weight: .semibold)).foregroundStyle(Palette.violet)
                    Text(untrusted(item)).font(RFont.sans(15.5, .medium)).foregroundStyle(Palette.ink)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 13)
                .accessibilityIdentifier("secret:\(i)")
            }
            FactRow(key: "Kept", value: String(leaseLabel(secrets.leaseSecs).dropFirst("for ".count)), valueId: "secretsLease")
        }
        .accessibilityContainer("secrets")
    }
}

private struct SshFacts: View {
    var ssh: SshSignView

    var body: some View {
        FactGroup {
            FactRow(key: "Server", value: untrusted(sshTarget(host: ssh.host, hostKey: ssh.hostKey)), mono: true, valueId: "sshWhat")
            FactRow(key: "Key", value: untrusted(ssh.keyName))
            MonoBlock(text: untrusted(ssh.keyFingerprint), id: "sshKey")
            if let hostKey = ssh.hostKey.map(untrusted), !hostKey.isEmpty {
                FactRow(key: "Host key", value: hostKey, mono: true, valueId: "sshHostKey")
            }
        }
        .accessibilityContainer("ssh")
    }
}

/// The file an AI uploaded for this change, as the server saw it.
private struct AttachedFileFacts: View {
    var blob: BlobView

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            FactHeader("Attached file")
            FileCard(blob: blob).padding(.horizontal, 16)
        }
        .accessibilityContainer("attachedFile")
    }
}

/// Text shown exactly as it is (a command, arguments), monospaced.
struct MonoBlock: View {
    var text: String
    var id: String
    var lines: Int?

    var body: some View {
        Text(text)
            .font(RFont.mono(13))
            .foregroundStyle(Palette.ink)
            .lineLimit(lines)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 18)
            .padding(.vertical, 13)
            .environment(\.layoutDirection, .leftToRight)
            .textSelection(.enabled)
            .accessibilityIdentifier(id)
    }
}

/// A long text folded out of the main page, in full.
struct TextPage: View {
    var text: String
    var mono: Bool
    var id: String

    var body: some View {
        FactGroup {
            Text(text)
                .font(mono ? RFont.mono(14) : RFont.sans(15))
                .foregroundStyle(Palette.ink)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(18)
                .environment(\.layoutDirection, .leftToRight)
                .textSelection(.enabled)
                .accessibilityIdentifier(id)
        }
        .padding(.top, 6)
    }
}

/// A remembered permission would let in everyone at a free mail provider.
private struct PublicDomainNote: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView

    var body: some View {
        // An Activity entry remembers nothing new, so there is nothing to warn about.
        if !vm.readOnly, let choice = vm.preview?.choice {
            let domains = publicDomainWarnings(choice)
            if !domains.isEmpty {
                HStack(spacing: 8) {
                    WarnTag(text: "Anyone at \(domains.joined(separator: ", "))", tint: Palette.warning)
                    HelpDot(id: "publicDomain", text: "These are free mail providers: allowing the whole domain covers millions of unrelated people. Prefer specific addresses.")
                }
                .padding(.horizontal, 16)
                .accessibilityIdentifier("publicDomainWarning")
            }
        }
    }
}
