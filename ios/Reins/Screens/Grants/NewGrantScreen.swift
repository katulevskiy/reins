import SwiftUI

/// The "New grant" form's state (the Android app's `NewGrantViewModel`). One per visit: the route's token changes for
/// every visit, so the form always starts empty.
@Observable
@MainActor
final class NewGrantModel {
    var draft = NewGrantDraft()
    private(set) var busy = false
    private(set) var error: String?
    private(set) var finished = false

    func edit(_ change: (inout NewGrantDraft) -> Void) {
        change(&draft)
        error = nil
    }

    /// Creating a permission needs the same authentication as approving one.
    func create(_ app: AppModel) async {
        guard !busy else { return }
        guard case let .ok(connectionId, account, kind, standing) = buildNewGrant(draft) else {
            if case let .invalid(message) = buildNewGrant(draft) {
                app.feedback.play(.error)
                error = message
            }
            return
        }
        busy = true
        error = nil
        defer { busy = false }
        if let refusal = await app.confirmOwner("Create grant") {
            error = refusal.message
            return
        }
        do {
            app.feedback.play(.grantCreated)
            try await app.core.createGrant(connectionId: connectionId, account: account, kind: kind, standing: standing)
            await app.refreshPending()
            finished = true
        } catch {
            app.feedback.play(.error)
            self.error = error.userMessage
        }
    }
}

/// Give an AI a permission before it asks. Rare, so it lives one tap away in Grants, not in the way.
struct NewGrantScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var form = NewGrantModel()
    @FocusState private var focus: Field?

    private enum Field { case parties, subject }

    /// A grant made here is for mail: the vault and the other services' accounts are not offered.
    private var mailAccounts: [AccountView] { model.accounts.filter { $0.service == "gmail" } }

    var body: some View {
        let draft = form.draft
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                section("For") {
                    if model.connections.isEmpty {
                        hint("No AI is connected yet.")
                    }
                    ChipFlow {
                        ForEach(model.connections, id: \.id) { conn in
                            AgentChip(connection: conn, selected: draft.connectionId == conn.id) {
                                form.edit { $0.connectionId = conn.id }
                            }
                            .accessibilityIdentifier("conn:\(conn.id)")
                        }
                    }
                }

                // With one Gmail account there is nothing to choose (it is picked below).
                if mailAccounts.count != 1 {
                    section("Account") {
                        if mailAccounts.isEmpty {
                            hint("No Gmail account yet. Add one under Integrations.")
                                .accessibilityIdentifier("noAccounts")
                        }
                        ChipFlow {
                            ForEach(mailAccounts, id: \.account) { a in
                                OptionChip(a.account, selected: draft.account == a.account) {
                                    form.edit { $0.account = a.account }
                                }
                                .accessibilityIdentifier("acct:\(a.account)")
                            }
                        }
                    }
                }

                section("To") {
                    KindSegments(send: draft.send) { send in
                        form.edit {
                            $0.send = send
                            if send { $0.anyMail = false }
                        }
                    }
                }

                if !draft.send {
                    section("Which emails") {
                        scopeChips(draft)
                    }
                }

                section(draft.send ? "Send to" : "From") {
                    if draft.anyMail {
                        Text("All mail, up to 7 days. Never sending.")
                            .font(RFont.sans(14))
                            .foregroundStyle(Palette.secondary)
                            .padding(.horizontal, 4)
                    } else {
                        TextField(
                            draft.send ? "a@b.com, @b.com" : "@stripe.com, receipts@apple.com",
                            text: Binding(get: { form.draft.partiesText }, set: { t in form.edit { $0.partiesText = String(t.prefix(1000)) } }),
                            axis: .vertical
                        )
                        .lineLimit(1...4)
                        .keyboardType(.emailAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .submitLabel(.next)
                        .focused($focus, equals: .parties)
                        .onSubmit { focus = .subject }
                        .fieldWell(mono: true)
                        .accessibilityLabel(draft.send ? "Send to" : "From")
                        .accessibilityIdentifier("parties")
                        TextField("Subject contains (optional)", text: Binding(get: { form.draft.subject }, set: { t in form.edit { $0.subject = String(t.prefix(200)) } }))
                            .submitLabel(.done)
                            .focused($focus, equals: .subject)
                            .onSubmit { focus = nil }
                            .fieldWell()
                            .accessibilityLabel("Subject contains")
                            .accessibilityIdentifier("newSubject")
                    }
                }

                section("For how long") {
                    lifetimes(draft)
                }

                if let error = form.error {
                    FormBanner(text: error).padding(.top, 18)
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 24)
            .frame(maxWidth: 640)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
        .pageBackground()
        .pageDock {
            Button {
                focus = nil
                Task { await form.create(model) }
            } label: {
                ZStack {
                    Label("Create grant", systemImage: BiometricSymbol.name).opacity(form.busy ? 0 : 1)
                    if form.busy { ProgressView() }
                }
            }
            .buttonStyle(DockButtonStyle(kind: .light))
            .disabled(form.busy)
            .accessibilityIdentifier("createGrant")
        }
        .navigationTitle("New grant")
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(true)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button("Cancel") {
                    focus = nil
                    model.back()
                    feedback.defaultTap()
                }
                .accessibilityIdentifier("cancelNewGrant")
            }
        }
        .task { await model.refreshConnections() }
        // With a single AI, or a single account, there is nothing to choose.
        .onChange(of: model.connections.map(\.id), initial: true) { _, ids in
            if form.draft.connectionId == nil, ids.count == 1 { form.edit { $0.connectionId = ids[0] } }
        }
        .onChange(of: mailAccounts.map(\.account), initial: true) { _, accounts in
            if form.draft.account == nil, accounts.count == 1 { form.edit { $0.account = accounts[0] } }
        }
        .onChange(of: form.finished) { _, done in
            if done { model.back() }
        }
    }

    /// Once | 1 hour | 1 day | Custom ▾ (7 days, 30 days). All mail takes neither Once nor 30 days.
    private func lifetimes(_ draft: NewGrantDraft) -> some View {
        typealias Option = DurationControl<NewGrantLifetime>.Option
        let allowed: (NewGrantLifetime) -> Bool = { !draft.anyMail || $0.allowedForAllMail }
        let inline = [NewGrantLifetime.oneTime, .hour, .day].map { Option(value: $0, label: $0.shortLabel, id: "\($0)", enabled: allowed($0)) }
        let more = [NewGrantLifetime.week, .month].map { Option(value: $0, label: $0.shortLabel, id: "\($0)", enabled: allowed($0)) }
        return DurationControl(inline: inline, more: more, selection: draft.lifetime, idPrefix: "newLifetime", onOther: nil) { kind in
            form.edit { $0.lifetime = kind }
        }
    }

    /// Specific senders | All mail. All mail is time-boxed to a week, so picking it drops Once and 30 days.
    private func scopeChips(_ draft: NewGrantDraft) -> some View {
        HStack(spacing: 8) {
            OptionChip("Specific senders", selected: !draft.anyMail, symbol: "at") {
                form.edit { $0.anyMail = false }
            }
            .accessibilityIdentifier("scope:specific")
            OptionChip("All mail", selected: draft.anyMail, symbol: "tray") {
                form.edit {
                    $0.anyMail = true
                    if !$0.lifetime.allowedForAllMail { $0.lifetime = .day }
                }
            }
            .accessibilityIdentifier("scope:all")
        }
    }

    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FormLabel(title)
                .padding(.horizontal, 6)
            content()
        }
        .padding(.top, 22)
    }

    private func hint(_ text: String) -> some View {
        Text(text).font(RFont.sans(15)).foregroundStyle(Palette.secondary)
    }
}

/// An AI to pick, with its avatar ("[🙂 Claude Code ✓]"): the shared `SelectChip`.
private struct AgentChip: View {
    var connection: ConnectionView
    var selected: Bool
    var action: () -> Void

    var body: some View {
        SelectChip(title: untrusted(connection.label), selected: selected, leading: {
            ConnectionAvatar(label: untrusted(connection.label), pick: connection.icon, size: 22)
        }, action: action)
    }
}

/// "Read mail | Send mail", as a segmented control with icons.
private struct KindSegments: View {
    var send: Bool
    var onChange: (Bool) -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 0) {
            segment("Read mail", symbol: "eye", on: !send, id: "kind:read") { onChange(false) }
            segment("Send mail", symbol: "paperplane", on: send, id: "kind:send") { onChange(true) }
        }
        .padding(3)
        .background(Palette.dynamic(0xE7E7EB, 0x060606), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private func segment(_ title: String, symbol: String, on: Bool, id: String, action: @escaping () -> Void) -> some View {
        Button {
            guard !on else { return }
            action()
            feedback.play(.selection)
        } label: {
            Label(title, systemImage: symbol)
                .font(RFont.sans(14, .semibold))
                .foregroundStyle(on ? Palette.text : Palette.secondary)
                .frame(maxWidth: .infinity, minHeight: 38)
                .background(on ? Palette.dynamic(0xFFFFFF, 0x232327) : .clear, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
        .accessibilityIdentifier(id)
    }
}
