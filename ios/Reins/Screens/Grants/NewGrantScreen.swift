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

/// Give an AI a permission before it asks (the Trust board, 03; quiet): five numbered steps that read like a sentence:
/// who, which account, read or send, from whom, for how long. Rare, so it lives behind the + on Grants. It is paper,
/// not a field: nothing is asking.
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
        let showAccounts = mailAccounts.count != 1
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text("New grant")
                    .textRole(.quietTitle)
                    .foregroundStyle(Palette.ink)
                    .padding(.horizontal, 20)
                    .padding(.top, 4)
                    .accessibilityAddTraits(.isHeader)

                FormStep(number: 1, title: "Who")
                if model.connections.isEmpty {
                    hint("No AI is connected yet.").padding(.horizontal, 20)
                }
                AgentPick(connections: model.connections, selected: draft.connectionId) { id in
                    form.edit { $0.connectionId = id }
                }

                // With one Gmail account there is nothing to choose (it is picked below).
                if showAccounts {
                    FormStep(number: 2, title: "Account")
                    if mailAccounts.isEmpty {
                        hint("No Gmail account yet. Add one under Connections.")
                            .padding(.horizontal, 20)
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
                    .padding(.horizontal, 16)
                }

                FormStep(number: showAccounts ? 3 : 2, title: "Can")
                Chooser([
                    Chooser.Segment(id: "kind:read", title: "Read", selected: !draft.send) { setSend(false) },
                    Chooser.Segment(id: "kind:send", title: "Send", selected: draft.send) { setSend(true) },
                ])
                .padding(.horizontal, 16)

                FormStep(number: showAccounts ? 4 : 3, title: draft.send ? "To" : "From") {
                    // All mail is time-boxed to a week and never sends: it takes the place of the addresses.
                    if !draft.send {
                        Toggle(isOn: Binding(get: { form.draft.anyMail }, set: { on in setAllMail(on) })) {
                            Text("All mail").font(RFont.sans(14, .medium)).foregroundStyle(Palette.ink2)
                        }
                        .toggleStyle(SignalSwitchStyle())
                        .fixedSize()
                        .scaleEffect(0.8, anchor: .trailing)
                        .accessibilityIdentifier("scope:all")
                    }
                }
                if draft.anyMail {
                    Text("All mail, up to 7 days. Never sending.")
                        .textRole(.body)
                        .foregroundStyle(Palette.inkMeta)
                        .padding(.horizontal, 20)
                } else {
                    RaisedGroup(inset: 44) {
                        HStack(spacing: 12) {
                            GlyphIcon(.mail, size: 18).foregroundStyle(Palette.ink2)
                            TextField(
                                draft.send ? "a@b.com, @b.com" : "@stripe.com, receipts@apple.com",
                                text: Binding(get: { form.draft.partiesText }, set: { t in form.edit { $0.partiesText = String(t.prefix(1000)) } }),
                                axis: .vertical
                            )
                            .lineLimit(1...4)
                            .font(RFont.mono(14))
                            .foregroundStyle(Palette.ink)
                            .keyboardType(.emailAddress)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .submitLabel(.next)
                            .focused($focus, equals: .parties)
                            .onSubmit { focus = .subject }
                            .accessibilityLabel(draft.send ? "Send to" : "From")
                            .accessibilityIdentifier("parties")
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 54)
                        HStack(spacing: 12) {
                            GlyphIcon(.search, size: 18).foregroundStyle(Palette.ink2)
                            TextField("Subject", text: Binding(get: { form.draft.subject }, set: { t in form.edit { $0.subject = String(t.prefix(200)) } }))
                                .font(RFont.sans(16))
                                .foregroundStyle(Palette.ink)
                                .submitLabel(.done)
                                .focused($focus, equals: .subject)
                                .onSubmit { focus = nil }
                                .accessibilityLabel("Subject contains")
                                .accessibilityIdentifier("newSubject")
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 54)
                    }
                }

                FormStep(number: showAccounts ? 5 : 4, title: "How long")
                lifetimes(draft)
                    .padding(.horizontal, 16)

                if let error = form.error {
                    FormBanner(text: error).padding(.horizontal, 16).padding(.top, 18)
                }
            }
            .padding(.bottom, 24)
            .frame(maxWidth: 640)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
        .pageBackground()
        .safeAreaInset(edge: .bottom, spacing: 0) {
            Button {
                focus = nil
                Task { await form.create(model) }
            } label: {
                ZStack {
                    Label { Text("Create") } icon: { GlyphIcon(.faceId, size: 18) }.opacity(form.busy ? 0 : 1)
                    if form.busy { ProgressView() }
                }
            }
            .buttonStyle(SignalButtonLook(kind: .inverse))
            .frame(height: 54)
            .disabled(form.busy)
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
            .background(Palette.paper.opacity(0.94).ignoresSafeArea(edges: .bottom))
            .accessibilityIdentifier("createGrant")
        }
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(true)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    focus = nil
                    model.back()
                    feedback.defaultTap()
                } label: {
                    GlyphIcon(.close, size: 18)
                }
                .accessibilityLabel("Cancel")
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

    private func hint(_ text: String) -> some View {
        Text(text).font(RFont.sans(15)).foregroundStyle(Palette.ink2)
    }
}

/// A numbered step of a form (the boards' `step`): its number on a small disc, its title, anything at the right.
struct FormStep<Trailing: View>: View {
    var number: Int
    var title: String
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 10) {
            Text("\(number)")
                .font(RFont.fixedSans(12, .semibold))
                .monospacedDigit()
                .foregroundStyle(Palette.ink2)
                .frame(width: 22, height: 22)
                .background(Palette.paperSunken, in: Circle())
                .accessibilityHidden(true)
            Text(title)
                .font(RFont.sans(14, .semibold))
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 8)
            trailing
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .padding(.bottom, 8)
        .frame(minHeight: 48, alignment: .bottom)
    }
}

extension FormStep where Trailing == EmptyView {
    init(number: Int, title: String) {
        self.init(number: number, title: title) { EmptyView() }
    }
}

/// The AIs to pick from as avatars with their names (the boards' `agent_pick`): at most four, the chosen one ringed in
/// violet with a check; the rest behind "+N More", a menu.
private struct AgentPick: View {
    var connections: [ConnectionView]
    var selected: String?
    var onPick: (String) -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        let shown = Self.shown(connections, selected: selected)
        let rest = connections.filter { c in !shown.contains { $0.id == c.id } }
        HStack(alignment: .top, spacing: 4) {
            ForEach(shown, id: \.id) { conn in
                let on = conn.id == selected
                Button {
                    feedback.play(.selection)
                    onPick(conn.id)
                } label: {
                    VStack(spacing: 6) {
                        ConnectionAvatar(connection: conn, size: 44)
                            .padding(3)
                            .overlay(Circle().strokeBorder(on ? Palette.violet : .clear, lineWidth: 2))
                            .overlay(alignment: .bottomTrailing) {
                                if on {
                                    GlyphIcon(.check, size: 11)
                                        .foregroundStyle(.white)
                                        .frame(width: 18, height: 18)
                                        .background(Palette.violet, in: Circle())
                                        .padding(2)
                                        .background(Palette.paper, in: Circle())
                                        .offset(x: 2, y: 2)
                                }
                            }
                        Text(verbatim: untrustedName(conn.label, limit: 24))
                            .font(RFont.sans(12.5, .semibold))
                            .foregroundStyle(on ? Palette.ink : Palette.ink2)
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 4)
                    .contentShape(Rectangle())
                }
                .buttonStyle(PressDimsStyle())
                .accessibilityLabel(untrustedName(conn.label))
                .accessibilityAddTraits(on ? .isSelected : [])
                .accessibilityIdentifier("conn:\(conn.id)")
            }
            if !rest.isEmpty {
                Menu {
                    ForEach(rest, id: \.id) { conn in
                        Button(untrustedName(conn.label)) { onPick(conn.id) }
                            .accessibilityIdentifier("conn:\(conn.id)")
                    }
                } label: {
                    VStack(spacing: 6) {
                        Text("+\(rest.count)")
                            .font(RFont.fixedSans(14, .semibold))
                            .foregroundStyle(Palette.ink2)
                            .frame(width: 44, height: 44)
                            .background(Palette.paperSunken, in: Circle())
                            .padding(3)
                        Text("More").font(RFont.sans(12.5, .semibold)).foregroundStyle(Palette.ink2)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 4)
                }
                .accessibilityLabel("\(rest.count) more AIs")
                .accessibilityIdentifier("moreAgents")
            }
        }
        .padding(.horizontal, 16)
    }

    /// The first four (three when more follow), always with the chosen one among them.
    static func shown(_ all: [ConnectionView], selected: String?) -> [ConnectionView] {
        let room = all.count > 5 ? 4 : 5
        var shown = Array(all.prefix(room))
        if let selected, !shown.contains(where: { $0.id == selected }), let pick = all.first(where: { $0.id == selected }) {
            shown[shown.count - 1] = pick
        }
        return shown
    }
}
