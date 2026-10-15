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

/// Give an AI a permission before it asks (iOS-Trust.png, 03). Rare, so it lives one tap away in Trust, not in the way.
/// A form that reads like a sentence: for this AI, to read mail, from these senders, for a day. It is paper, not a
/// field: nothing is asking.
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
                AskText([AskText.Run(text: "New "), AskText.Run(text: "grant", target: true)], role: .title)
                    .foregroundStyle(Palette.ink)
                    .padding(.horizontal, 6)
                    .padding(.top, 8)
                    .accessibilityAddTraits(.isHeader)
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
                    Chooser([
                        Chooser.Segment(id: "kind:read", title: "Read mail", selected: !draft.send) { setSend(false) },
                        Chooser.Segment(id: "kind:send", title: "Send mail", selected: draft.send) { setSend(true) },
                    ])
                }

                section(draft.send ? "Send to" : "From", trailing: {
                    // All mail is time-boxed to a week and never sends: it greys the address box.
                    if !draft.send {
                        Toggle(isOn: Binding(get: { form.draft.anyMail }, set: { on in setAllMail(on) })) {
                            Text("All mail").font(RFont.sans(13.5)).foregroundStyle(Palette.ink2)
                        }
                        .toggleStyle(SignalSwitchStyle())
                        .fixedSize()
                        .accessibilityIdentifier("scope:all")
                    }
                }) {
                    if draft.anyMail {
                        Text("All mail, up to 7 days. Never sending.")
                            .textRole(.body)
                            .foregroundStyle(Palette.inkMeta)
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
            .buttonStyle(SignalButtonLook(kind: .inverse))
            .frame(height: 54)
            .disabled(form.busy)
            .accessibilityIdentifier("createGrant")
        }
        .navigationTitle("")
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

    private func setSend(_ send: Bool) {
        guard send != form.draft.send else { return }
        feedback.play(.selection)
        form.edit {
            $0.send = send
            if send { $0.anyMail = false }
        }
    }

    /// All mail is time-boxed to a week, so turning it on drops Once and 30 days.
    private func setAllMail(_ on: Bool) {
        form.edit {
            $0.anyMail = on
            if on && !$0.lifetime.allowedForAllMail { $0.lifetime = .day }
        }
    }

    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        section(title, trailing: { EmptyView() }, content: content)
    }

    private func section<Trailing: View, Content: View>(
        _ title: String, @ViewBuilder trailing: () -> Trailing, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(title).textRole(.label).foregroundStyle(Palette.inkMeta)
                Spacer(minLength: 8)
                trailing()
            }
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
