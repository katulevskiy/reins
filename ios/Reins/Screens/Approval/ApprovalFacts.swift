import SwiftUI

// The facts under the field (the Android app's ApprovalFacts.kt): hairline rows of keys and values on paper, long
// content folded into rows that open a page, by kind of request. What is approved (an address, a command, a path, a
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

// MARK: Hairline rows

/// Rows on paper with a `rule` hairline under each (Signal lists: hairlines, never cards). The rows keep their own 18 pt
/// inside, so their text sits at the 24 pt gutter. `border` tints the hairlines (a group with a warning).
struct FactGroup<Content: View>: View {
    var border: Color?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Group(subviews: content) { rows in
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    row
                    Rectangle().fill(border ?? Palette.rule).frame(height: 1).padding(.horizontal, 18)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 6)
    }
}

/// "Branch   feature/passkeys": a key on the left, its value on the right.
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
        HStack(spacing: 14) {
            Text(key).font(RFont.sans(15.5)).foregroundStyle(Palette.ink2).lineLimit(1)
            Text(value)
                .font(mono ? RFont.mono(14, .medium) : RFont.sans(15.5, .medium))
                .foregroundStyle(tint ?? Palette.ink)
                .lineLimit(lines)
                .fixedSize(horizontal: false, vertical: true)
                .multilineTextAlignment(.trailing)
                .frame(maxWidth: .infinity, alignment: .trailing)
                .accessibilityIdentifier(valueId ?? "")
                .textSelection(.enabled)
            trailing
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 13)
        .frame(minHeight: 50)
        .accessibilityElement(children: .combine)
    }
}

extension FactRow where Trailing == EmptyView {
    init(key: String, value: String, mono: Bool = false, valueId: String? = nil, tint: Color? = nil, lines: Int? = nil) {
        self.init(key: key, value: value, mono: mono, valueId: valueId, tint: tint, lines: lines) { EmptyView() }
    }
}

/// A row that opens a page: a title, a short detail on the right, a chevron.
struct DrillRow<Detail: View>: View {
    var title: String
    var id: String
    var page: ApprovalPage
    @ViewBuilder var detail: Detail
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            nav.path.append(page)
            feedback.play(.open)
        } label: {
            HStack(spacing: 12) {
                Text(title).font(RFont.sans(15.5, .medium)).foregroundStyle(Palette.ink).lineLimit(1)
                detail.frame(maxWidth: .infinity, alignment: .trailing)
                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.inkMeta)
            }
            .padding(.leading, 18)
            .padding(.trailing, 14)
            .padding(.vertical, 13)
            .frame(minHeight: 50)
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

/// A small header above a group: a few words on the left, an action or a (?) on the right.
struct FactHeader<Trailing: View>: View {
    var text: String
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack {
            Text(text).font(RFont.sans(13, .medium)).foregroundStyle(Palette.inkMeta)
            Spacer(minLength: 8)
            trailing
        }
        .padding(.leading, 22)
        .padding(.trailing, 16)
        .padding(.top, 4)
    }
}

extension FactHeader where Trailing == EmptyView {
    init(_ text: String) { self.init(text: text) { EmptyView() } }
}

/// A square tick in the accent colour.
struct TickBox: View {
    var checked: Bool
    var enabled = true

    var body: some View {
        RoundedRectangle(cornerRadius: 7, style: .continuous)
            .fill(checked ? Palette.violet : Color.clear)
            .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous).strokeBorder(checked ? Palette.violet : Palette.dynamic(0x97979F, 0x3A3A40), lineWidth: 2))
            .overlay { if checked { Image(systemName: "checkmark").font(.system(size: 12, weight: .bold)).foregroundStyle(.white) } }
            .frame(width: 22, height: 22)
            .opacity(enabled ? 1 : 0.55)
            .animation(.spring(duration: 0.22), value: checked)
    }
}

// MARK: Emails and items found

/// The emails (or items) found: ticking is the decision; a code is never ticked for you, a covered one is locked.
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
                FactHeader(AttentionText.mailHeader(view.messages, service: view.service, shared: shared)).accessibilityIdentifier("messagesHeader")
            } else {
                FactHeader(text: AttentionText.mailHeader(view.messages, service: view.service)) {
                    Button(allTicked ? "Clear" : "Select all") {
                        vm.draft.selected = allTicked ? [] : Set(view.messages.map(\.id))
                        vm.draft.allMail = nil
                        feedback.play(.tap)
                    }
                    .font(RFont.sans(14.5, .semibold))
                    .foregroundStyle(Palette.violet)
                    .accessibilityIdentifier(allTicked ? "clearAll" : "selectAll")
                }
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
        if let query = view.query.map(untrusted), !query.isEmpty {
            FactGroup {
                DrillRow(title: "Search", id: "queryRow", page: .text(title: "Search", body: query, mono: true, id: "queryText")) {
                    Text(query).font(RFont.mono(13)).foregroundStyle(Palette.inkMeta).lineLimit(1)
                }
            }
        }
        if view.kind == .fetch, let account = view.account, !account.isEmpty {
            FactGroup { FactRow(key: "Account", value: untrusted(account), mono: true) }
        }
    }
}

/// One email: its tick, subject, sender and date, and a flag when it looks like a code.
private struct MailRow: View {
    var message: MessageView
    var checked: Bool
    var enabled: Bool
    var service: String
    var onChange: (Bool) -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        let m = message
        let subject = untrusted(m.subject)
        let title = !subject.isEmpty ? subject : (!untrusted(m.snippet).isEmpty ? untrusted(m.snippet) : untrusted(m.from))
        let gmail = service.isEmpty || service == "gmail"
        // Mail says who and when; an item elsewhere (a login, a file) says what it is.
        let meta = !gmail && !subject.isEmpty && !m.snippet.isEmpty
            ? untrusted(m.snippet)
            : [subject.isEmpty ? nil : untrusted(ApprovalWords.displayName(m.from)), m.date != 0 ? TimeText.day(m.date) : nil].compactMap { $0 }.joined(separator: " · ")
        Button {
            onChange(!checked)
            feedback.play(.toggle(!checked))
        } label: {
            HStack(spacing: 14) {
                TickBox(checked: checked, enabled: enabled)
                VStack(alignment: .leading, spacing: 5) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title).font(RFont.sans(15, .semibold)).foregroundStyle(Palette.ink).strikethrough(!checked, color: Palette.inkMeta).lineLimit(1)
                        if !meta.isEmpty { Text(meta).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta).lineLimit(1) }
                    }
                    .opacity(checked ? 1 : 0.45)
                    // One chip: a code or a password keeps its word ("Code") among the other reasons.
                    if let mark = AttentionText.mail(m, service: service) {
                        AttentionTag(mark: mark, id: m.sensitive ? "sensitive:\(m.id)" : "attention:\(m.id)")
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if m.coveredByGrant {
                    Image(systemName: "lock.fill").font(.system(size: 13)).foregroundStyle(Palette.inkMeta).accessibilityLabel("Already allowed")
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 13)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityAddTraits(checked ? .isSelected : [])
        .accessibilityValue(checked ? "Ticked" : "Not ticked")
        .accessibilityIdentifier("message:\(m.id)")
    }
}

// MARK: An email to send

private struct SendFacts: View {
    var view: ApprovalView

    var body: some View {
        if let email = view.email {
            FactGroup {
                FactRow(key: "To", value: email.to.map(untrusted).joined(separator: ", "), mono: true)
                if !email.cc.isEmpty { FactRow(key: "Cc", value: email.cc.map(untrusted).joined(separator: ", "), mono: true) }
                FactRow(key: "Subject", value: untrusted(email.subject))
                Text(untrusted(email.body))
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.ink)
                    .lineSpacing(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 14)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("emailBody")
            }
            .accessibilityContainer("emailPreview")
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

private struct GrantFacts: View {
    var view: ApprovalView

    var body: some View {
        if let grant = view.grant {
            let (tone, reach): (Color, String) = switch grant.breadth {
            case "everything": (Palette.danger, "Everything")
            case "broad": (Palette.warning, "Broad")
            default: (Palette.success, "Narrow")
            }
            FactHeader(text: "Permission") {
                HelpDot(id: "grant", text: "Nothing is shared now. Requests that match are answered without asking until it ends. Revoke it any time in Grants.")
            }
            FactGroup(border: Palette.violet.opacity(0.35)) {
                ForEach(Array(grant.lines.enumerated()), id: \.offset) { _, line in FactRow(key: "Only", value: untrusted(line)) }
                FactRow(key: "For", value: ApprovalWords.period(grant.durationSecs) + (grant.maxUses.map { " · at most \($0) uses" } ?? ""))
                FactRow(key: "Reach", value: reach, tint: tone)
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

private struct McpFacts: View {
    var call: McpCallView

    var body: some View {
        FactGroup {
            HStack(spacing: 12) {
                ServiceAvatar(service: "mcp", size: 34)
                VStack(alignment: .leading, spacing: 1) {
                    Text(untrusted(call.serverName)).font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                    Text(mcpHostLabel(call.serverUrl)).font(RFont.mono(12.5)).foregroundStyle(Palette.inkMeta).lineLimit(1)
                        .environment(\.layoutDirection, .leftToRight)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                TintTag(text: call.readOnly ? "Read only" : "Changes things", tint: call.readOnly ? Palette.success : Palette.kindPush)
                    .accessibilityIdentifier("mcpEffect")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            VStack(alignment: .leading, spacing: 3) {
                let title = untrusted(call.title)
                Text(title.isEmpty ? untrusted(call.tool) : title).font(RFont.sans(16, .semibold)).foregroundStyle(Palette.ink)
                    .accessibilityIdentifier("mcpTool")
                if call.title != call.tool { Text(untrusted(call.tool)).font(RFont.mono(12.5)).foregroundStyle(Palette.inkMeta) }
                if !call.description.trimmingCharacters(in: .whitespaces).isEmpty {
                    Text(untrusted(call.description)).font(RFont.sans(13.5)).foregroundStyle(Palette.ink2).lineLimit(3)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 18)
            .padding(.vertical, 13)
            VStack(alignment: .leading, spacing: 6) {
                Text("Arguments").font(RFont.sans(13, .medium)).foregroundStyle(Palette.inkMeta)
                let args = untrusted(call.argumentsJson)
                Text(args.isEmpty ? "{}" : args).font(RFont.mono(13)).foregroundStyle(Palette.ink)
                    .environment(\.layoutDirection, .leftToRight)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("mcpArguments")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 18)
            .padding(.vertical, 13)
        }
        .accessibilityContainer("mcpCall")
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
                    DrillRow(title: "Full invocation", id: "fullInvocation", page: .text(title: "Full invocation · may contain secrets", body: original, mono: true, id: "fullInvocationText")) {
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

/// Core-derived facts: the clients never reinterpret a shell command or turn its summary into permission.
struct CommandFacts: View {
    var presentation: CommandPresentation

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
            ForEach(Array(presentation.notices.enumerated()), id: \.offset) { index, notice in
                Text(untrusted(notice)).font(RFont.sans(14)).foregroundStyle(Palette.ink2)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 12)
                    .accessibilityIdentifier("commandNotice:\(index)")
            }
            if !presentation.setup.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Setup").font(RFont.sans(15)).foregroundStyle(Palette.inkMeta)
                    Text(untrusted(presentation.setup.joined(separator: "\n"))).font(RFont.mono(13)).foregroundStyle(Palette.ink)
                        .environment(\.layoutDirection, .leftToRight).textSelection(.enabled)
                }.frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 18).padding(.vertical, 12)
                    .accessibilityIdentifier("commandSetup")
            }
        }.accessibilityContainer("commandPresentation")
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
