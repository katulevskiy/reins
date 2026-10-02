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
    @State private var form = NewGrantModel()
    @FocusState private var focus: Field?

    private enum Field { case parties, subject }

    var body: some View {
        let draft = form.draft
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text("Allow something before it is asked.")
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.secondary)
                    .padding(.top, 4)

                section("For which AI?") {
                    if model.connections.isEmpty {
                        hint("No AI is connected yet.")
                    }
                    ChipFlow {
                        ForEach(model.connections, id: \.id) { conn in
                            OptionChip(untrusted(conn.label), selected: draft.connectionId == conn.id) {
                                form.edit { $0.connectionId = conn.id }
                            }
                            .accessibilityIdentifier("conn:\(conn.id)")
                        }
                    }
                }

                section("On which account?") {
                    if model.accounts.isEmpty {
                        hint("No Gmail account is connected yet. Add one under Integrations.")
                            .accessibilityIdentifier("noAccounts")
                    }
                    ChipFlow {
                        ForEach(model.accounts, id: \.account) { a in
                            OptionChip(a.account, selected: draft.account == a.account) {
                                form.edit { $0.account = a.account }
                            }
                            .accessibilityIdentifier("acct:\(a.account)")
                        }
                    }
                }

                section("It may") {
                    HStack(spacing: 8) {
                        OptionChip("Read emails", selected: !draft.send) { form.edit { $0.send = false } }
                            .accessibilityIdentifier("kind:read")
                        OptionChip("Send emails", selected: draft.send) {
                            form.edit {
                                $0.send = true
                                $0.anyMail = false
                            }
                        }
                        .accessibilityIdentifier("kind:send")
                    }
                }

                if !draft.send {
                    section("Which emails?") {
                        HStack(spacing: 8) {
                            OptionChip("Specific senders", selected: !draft.anyMail) { form.edit { $0.anyMail = false } }
                                .accessibilityIdentifier("scope:specific")
                            OptionChip("All mail", selected: draft.anyMail) {
                                form.edit {
                                    $0.anyMail = true
                                    if !$0.lifetime.allowedForAllMail { $0.lifetime = .day }
                                }
                            }
                            .accessibilityIdentifier("scope:all")
                        }
                    }
                }

                if !draft.anyMail {
                    section(draft.send ? "Send to" : "From") {
                        TextField(
                            draft.send ? "a@b.com, @b.com" : "alerts@bank.com, @bank.com",
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
                        Text("Addresses, or a whole domain with @.")
                            .font(RFont.sans(12.5))
                            .foregroundStyle(Palette.tertiary)
                            .padding(.leading, 4)
                    }
                    section("Subject contains (optional)") {
                        TextField("statement", text: Binding(get: { form.draft.subject }, set: { t in form.edit { $0.subject = String(t.prefix(200)) } }))
                            .submitLabel(.done)
                            .focused($focus, equals: .subject)
                            .onSubmit { focus = nil }
                            .fieldWell()
                            .accessibilityLabel("Subject contains")
                            .accessibilityIdentifier("newSubject")
                    }
                } else {
                    Text("Every email, for as long as you choose (at most 7 days). Sending is never included.")
                        .font(RFont.sans(14))
                        .foregroundStyle(Palette.secondary)
                        .padding(.top, 14)
                }

                section("For how long?") {
                    ChipFlow {
                        ForEach(NewGrantLifetime.allCases.filter { !draft.anyMail || $0.allowedForAllMail }, id: \.self) { kind in
                            OptionChip(kind.label, selected: draft.lifetime == kind) { form.edit { $0.lifetime = kind } }
                                .accessibilityIdentifier("newLifetime:\(kind)")
                        }
                    }
                    if draft.lifetime == .oneTime {
                        Text("It covers one request and stays until then, whenever the AI gets around to asking.")
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.tertiary)
                            .padding(.top, 2)
                    }
                }

                if let error = form.error {
                    FormBanner(text: error).padding(.top, 18)
                }

                Button {
                    focus = nil
                    Task { await form.create(model) }
                } label: {
                    HStack(spacing: 8) {
                        if form.busy { ProgressView().tint(.white) }
                        Text("Create grant")
                    }
                }
                .buttonStyle(CapsuleButtonStyle(kind: .accent))
                .disabled(form.busy)
                .padding(.top, 24)
                .accessibilityIdentifier("createGrant")
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 32)
            .frame(maxWidth: 640)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
        .pageBackground()
        .navigationTitle("New grant")
        .navigationBarTitleDisplayMode(.inline)
        .task { await model.refreshConnections() }
        // With a single AI, or a single account, there is nothing to choose.
        .onChange(of: model.connections.map(\.id), initial: true) { _, ids in
            if form.draft.connectionId == nil, ids.count == 1 { form.edit { $0.connectionId = ids[0] } }
        }
        .onChange(of: model.accounts.map(\.account), initial: true) { _, accounts in
            if form.draft.account == nil, accounts.count == 1 { form.edit { $0.account = accounts[0] } }
        }
        .onChange(of: form.finished) { _, done in
            if done { model.back() }
        }
    }

    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FormLabel(title)
            content()
        }
        .padding(.top, 24)
    }

    private func hint(_ text: String) -> some View {
        Text(text).font(RFont.sans(15)).foregroundStyle(Palette.secondary)
    }
}
