import SwiftUI

/// The accounts of one integration and the way to add another, which depends on the kind of service.
struct ServiceScreen: View {
    var serviceId: String

    var body: some View {
        AccountsScreen(serviceId: serviceId)
    }
}

/// One integration's page (the Connections board, 03, 04 and 06; Gmail's too): a quiet page. Its logo and name, its
/// accounts (each with its status, its permissions and removing it), then connecting another one.
struct AccountsScreen: View {
    var serviceId: String

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.openURL) private var openURL
    @State private var accounts: AccountsModel?
    @State private var removing: String?

    var body: some View {
        Group {
            if let accounts {
                if let service = accounts.view {
                    if let identity = accounts.connectedAccount {
                        AccountPermissionsScreen(service: serviceId, account: identity, finishing: true, onDone: { accounts.dismissConnection(); model.back() }, onAnother: service.kind == "device" || service.kind == "vault" ? nil : { accounts.dismissConnection() })
                    } else { content(service, accounts) }
                } else {
                    EmptyState(symbol: "puzzlepiece.extension", title: "Integration", message: "This integration is not available.")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .pageBackground()
                }
            } else {
                Color.clear.pageBackground()
            }
        }
        .quietPageBar(accounts?.view?.name ?? serviceName(serviceId))
        .task(id: serviceId) {
            let m: AccountsModel
            if let existing = accounts, existing.service == serviceId {
                m = existing
            } else {
                m = AccountsModel(service: serviceId, model: model)
                accounts = m
            }
            await m.refresh()
        }
    }

    @ViewBuilder
    private func content(_ service: ServiceView, _ m: AccountsModel) -> some View {
        NextPage {
            QuietTitle(title: service.name) {
                MarkPlate(size: 36) { ServiceAvatar(service: service.service, size: 36) }
            }
            SectionHeader("Accounts", quiet: true, trailing: service.accounts.isEmpty ? nil : "\(service.accounts.count)")
            FlatGroup(inset: 44) {
                if service.accounts.isEmpty {
                    NoAccountsRow(title: ServiceCopy.emptyTitle(service), message: ServiceCopy.intro(service))
                }
                ForEach(service.accounts, id: \.account) { account in
                    AccountRowView(
                        title: ServiceCopy.accountTitle(service, account.account),
                        mono: service.kind == "telegram",
                        status: m.statuses[account.account],
                        needsAgain: ServiceCopy.needsAgain(service),
                        canAllowAgain: ServiceCopy.canAllowAgain(service),
                        busy: m.busy,
                        avatar: {
                            if service.kind == "device" {
                                ServiceAvatar(service: service.service, size: 32)
                            } else {
                                BlobAvatar(seed: account.account, size: 32)
                            }
                        },
                        onAllowAgain: { allowAgain(service, account.account, m) },
                        onRemove: { removing = account.account }
                    )
                    .accessibilityIdentifier("account:\(account.account)")
                    Button {
                        model.show(.accountPermissions(service: serviceId, account: account.account, temporary: false))
                        feedback.defaultTap()
                    } label: {
                        TileRow("Permissions", glyph: .shield).padding(.horizontal, -14)
                    }
                    .buttonStyle(RowPressStyle())
                    .accessibilityLabel("Permissions for \(ServiceCopy.accountTitle(service, account.account))")
                    .accessibilityIdentifier("accountPermissions:\(account.account)")
                }
            }
            let help = [ServiceCopy.intro(service), ServiceCopy.accountsFooter(service), ServiceCopy.fineprint(service)]
                .filter { !$0.isEmpty }
                .joined(separator: "\n\n")
            // What the service is for and its small print (Telegram's warning about automation among them), behind a (?).
            if !help.isEmpty {
                GroupFooter(help).padding(.horizontal, 20).padding(.top, 2)
            }
            VStack(alignment: .leading, spacing: 0) {
                if service.available {
                    AddAccountControls(service: service, accounts: m)
                } else {
                    IntegrationBanner(text: untrusted(service.note ?? "Not available in this build."), kind: .warning)
                        .padding(.horizontal, 20)
                        .padding(.top, 18)
                        .accessibilityIdentifier("unavailable")
                }
                if let error = m.error {
                    IntegrationBanner(text: untrusted(error), kind: .error)
                        .padding(.horizontal, 20)
                        .padding(.top, 12)
                        .accessibilityIdentifier("accountError")
                    if m.refusedByIOS {
                        Button("Open Settings") {
                            if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                        }
                        .buttonStyle(SignalButtonStyle(.soft))
                        .padding(.horizontal, 20)
                        .padding(.top, 10)
                    }
                }
            }
        }
        .refreshable {
            feedback.play(.refresh)
            await m.refresh()
        }
        .animation(.smooth, value: service.accounts)
        .animation(.smooth, value: m.error)
        .confirmationDialog(
            removing.map { ServiceCopy.removeTitle(service, $0) } ?? "",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            titleVisibility: .visible,
            presenting: removing
        ) { account in
            Button("Remove", role: .destructive) {
                feedback.quietClose()
                removing = nil
                Task { await m.remove(account) }
            }
            Button("Cancel", role: .cancel) { removing = nil }
        } message: { _ in
            Text(ServiceCopy.removeMessage(gmail: m.isGmail))
        }
        .presentationFeedback(removing != nil)
    }

    private func allowAgain(_ service: ServiceView, _ account: String, _ m: AccountsModel) {
        switch service.kind {
        case "google": Task { await m.addGoogle(hint: account) }
        case "device": Task { await m.addDevice() }
        default: break
        }
    }
}

/// Connecting another account: Google's sign-in, this phone's permission, a token, the vault's master password, or
/// Telegram's phone number and code.
private struct AddAccountControls: View {
    var service: ServiceView
    var accounts: AccountsModel
    @Environment(AppModel.self) private var model

    var body: some View {
        switch service.kind {
        case "google":
            VStack(alignment: .leading, spacing: 4) {
                Button {
                    Task { await accounts.addGoogle() }
                } label: {
                    BusyLabel(title: "Add account", glyph: .plus, busy: accounts.busy)
                }
                .buttonStyle(SignalButtonStyle(.soft))
                .disabled(accounts.busy)
                .accessibilityIdentifier("addAccount")
                // Google's unverified-app screen, and the way past it, behind a (?).
                if !ServiceCopy.googleSignInNote.isEmpty {
                    GroupFooter(ServiceCopy.googleSignInNote)
                        .accessibilityIdentifier("googleSignInNote")
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
        case "device":
            if service.accounts.isEmpty {
                Button {
                    Task { await accounts.addDevice() }
                } label: {
                    BusyLabel(title: "Allow on this phone", glyph: .phone, busy: accounts.busy)
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(accounts.busy)
                .padding(.horizontal, 20)
                .padding(.top, 18)
                .accessibilityIdentifier("allowDevice")
            }
        case "token":
            if let host = GitHosts.of(service.service) {
                GitHostConnect(host: host, accounts: accounts)
            } else {
                GitHubConnect(accounts: accounts)
            }
        case "vault":
            VStack(alignment: .leading, spacing: 10) {
                if !service.accounts.isEmpty {
                    Button { model.push(.vault) } label: {
                        Label { Text("Open the vault") } icon: { GlyphIcon(.vault, size: 18) }
                    }
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .accessibilityIdentifier("openVault")
                    GroupFooter("See, add and change your API keys, logins and SSH keys. reins run, the API proxy and the SSH agent use them by name.")
                }
                if service.accounts.isEmpty {
                    SecretForm(
                        placeholder: "Master password",
                        button: "Unlock and connect",
                        busy: accounts.busy,
                        hint: "The vault of the account this phone is signed in with.",
                        contentType: .password
                    ) { secret in Task { await accounts.addSecret(secret) } }
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
        case "telegram":
            TelegramLogin(accounts: accounts)
        default:
            Text("This kind of integration cannot be added here.")
                .textRole(.body)
                .foregroundStyle(Palette.ink2)
                .padding(20)
        }
    }
}

/// A button's label that turns into a spinner while its work runs.
struct BusyLabel: View {
    var title: String
    var glyph: Glyph?
    var busy: Bool

    var body: some View {
        HStack(spacing: 8) {
            if busy {
                ProgressView().controlSize(.small)
            } else if let glyph {
                GlyphIcon(glyph, size: 18)
            }
            Text(title)
        }
    }
}

// MARK: Tokens

/// GitHub in a few taps (the Connections board, 03): "Chosen repos" or "Everything" opens GitHub's token page,
/// already filled in; Generate and Copy there, come back, and the token row's paste connects at once. Typing one in by
/// hand is still possible.
private struct GitHubConnect: View {
    var accounts: AccountsModel
    @Environment(\.openURL) private var openURL

    var body: some View {
        SectionHeader("Add an account", quiet: true)
        HStack(spacing: 8) {
            Button { open(GitHubToken.fineGrainedURL()) } label: {
                Label { Text("Chosen repos") } icon: { GlyphIcon(.folderSearch, size: 18) }
            }
            .buttonStyle(SignalButtonStyle(.soft, size: .small))
            .accessibilityIdentifier("openGithub")
            Button { open(GitHubToken.classicURL()) } label: {
                Label { Text("Everything") } icon: { GlyphIcon(.globe, size: 18) }
            }
            .buttonStyle(SignalButtonStyle(.soft, size: .small))
            .accessibilityIdentifier("openGithubClassic")
        }
        .disabled(accounts.busy)
        .padding(.horizontal, 20)
        .padding(.bottom, 4)
        TokenRow(placeholder: "Token", busy: accounts.busy, looksLikeToken: GitHubToken.looksLikeToken, hostName: "GitHub") { token in
            Task { await accounts.addSecret(token) }
        }
        .padding(.top, 8)
        GroupFooter(
            "Either button opens GitHub with the token already set up. Tap Generate token, copy it, and come back: paste connects it.\n\n" +
                "Chosen repos: a fine-grained token, only for the repositories you pick (no gists or notifications).\n\n" +
                "Everything: a classic token for every repository, plus gists and notifications. The token is kept encrypted on this phone."
        )
        .padding(.horizontal, 20)
    }

    private func open(_ url: String) {
        if let u = URL(string: url) { openURL(u) }
    }
}

/// A git host besides GitHub: open its token page, create and copy the token there, come back and paste it.
private struct GitHostConnect: View {
    var host: GitHost
    var accounts: AccountsModel
    @Environment(\.openURL) private var openURL

    var body: some View {
        SectionHeader("Add an account", quiet: true)
        Button {
            if let u = URL(string: host.tokenURL()) { openURL(u) }
        } label: {
            Label { Text("Create a token on \(host.name)") } icon: { GlyphIcon(.key, size: 18) }
        }
        .buttonStyle(SignalButtonStyle(.soft, size: .small))
        .disabled(accounts.busy)
        .padding(.horizontal, 20)
        .padding(.bottom, 4)
        .accessibilityIdentifier("openTokenPage")
        TokenRow(placeholder: host.placeholder, busy: accounts.busy, looksLikeToken: host.looksLikeToken, hostName: host.name) { token in
            Task { await accounts.addSecret(token) }
        }
        .padding(.top, 8)
        GroupFooter([host.steps, host.hint].filter { !$0.isEmpty }.joined(separator: "\n\n"))
            .padding(.horizontal, 20)
            .accessibilityIdentifier("tokenSteps")
    }
}

/// The token as a flat input row (the kit's `inp` with a paste button): pasting something shaped like the host's
/// token connects at once and takes it off the clipboard; anything typed connects with Connect. Sent once, then
/// cleared.
private struct TokenRow: View {
    var placeholder: String
    var busy: Bool
    var looksLikeToken: (String?) -> Bool
    var hostName: String
    var onToken: (String) -> Void
    @Environment(\.feedback) private var feedback
    @State private var secret = ""
    @State private var wrong = false
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            FlatFields {
                InputRow(glyph: .vault) {
                    SecureField(placeholder, text: $secret)
                        .font(RFont.mono(15))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .focused($focused)
                        .submitLabel(.go)
                        .onSubmit(submit)
                        .disabled(busy)
                        .accessibilityIdentifier("secret")
                } trailing: {
                    PasteButton(payloadType: String.self) { strings in
                        Task { @MainActor in paste(strings.first) }
                    }
                    .labelStyle(.iconOnly)
                    .buttonBorderShape(.circle)
                    .tint(Palette.ink2)
                    .controlSize(.small)
                    .disabled(busy)
                    .accessibilityLabel("Paste")
                    .accessibilityIdentifier("useCopied")
                }
            }
            if wrong {
                Text("That is not a \(hostName) token. Copy it again on \(hostName), or type it in.")
                    .textRole(.sub)
                    .foregroundStyle(Palette.danger)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 20)
            }
            if !secret.isEmpty || busy {
                Button(action: submit) { BusyLabel(title: "Connect", glyph: nil, busy: busy) }
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .disabled(busy || secret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .padding(.horizontal, 20)
                    .accessibilityIdentifier("connectSecret")
            }
        }
        .animation(.smooth(duration: 0.2), value: secret.isEmpty)
    }

    private func paste(_ text: String?) {
        let clean = text?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !clean.isEmpty else { return }
        clearClipboard()
        if looksLikeToken(clean) {
            wrong = false
            secret = ""
            onToken(clean)
        } else {
            feedback.play(.error)
            wrong = true
            secret = clean
        }
    }

    private func submit() {
        let sent = secret.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !busy, !sent.isEmpty else { return }
        secret = ""
        focused = false
        wrong = false
        onToken(sent)
    }
}

// MARK: Telegram

/// Phone number, code, then (if it has one) the password (the Connections board, 06): the same steps as signing in
/// to Telegram anywhere else.
private struct TelegramLogin: View {
    var accounts: AccountsModel
    @Environment(\.feedback) private var feedback
    @State private var phone = ""
    @State private var code = ""
    @State private var password = ""
    @FocusState private var focus: Field?

    private enum Field { case phone, code, password }

    var body: some View {
        let busy = accounts.busy
        switch accounts.login {
        case .phone:
            SectionHeader("Add an account", quiet: true)
            FlatFields {
                InputRow(glyph: .phone) {
                    TextField("Phone number", text: $phone)
                        .font(RFont.mono(15))
                        .keyboardType(.phonePad)
                        .textContentType(.telephoneNumber)
                        .focused($focus, equals: .phone)
                        .disabled(busy)
                        .accessibilityIdentifier("phone")
                }
            }
            Button {
                focus = nil
                Task { await accounts.loginBegin(phone) }
            } label: {
                BusyLabel(title: "Send me a code", glyph: nil, busy: busy)
            }
            .buttonStyle(SignalButtonStyle(.inverse))
            .disabled(busy || !TelegramStep.phoneReady(phone))
            .padding(.horizontal, 20)
            .padding(.top, 14)
            .accessibilityIdentifier("sendCode")
        case let .code(number):
            SectionHeader("Code sent to", quiet: true) {
                Text(verbatim: number).font(RFont.mono(13)).foregroundStyle(Palette.ink2).environment(\.layoutDirection, .leftToRight)
            }
            VStack(spacing: 0) {
                DigitBoxes(code: code, focused: focus == .code) {
                    TextField("Login code", text: Binding(get: { code }, set: { new in
                        let clean = TelegramStep.cleanCode(new)
                        // Each digit ticks as it lands.
                        if clean.count > code.count { feedback.play(.detent, step: clean.count) }
                        code = clean
                    }))
                    .keyboardType(.numberPad)
                    .textContentType(.oneTimeCode)
                    .focused($focus, equals: .code)
                    .disabled(busy)
                    .accessibilityIdentifier("code")
                }
                .onTapGesture { focus = .code }
                .onAppear { focus = .code }
                Button {
                    let sent = code
                    code = ""
                    Task { await accounts.loginCode(sent) }
                } label: {
                    BusyLabel(title: "Sign in", glyph: nil, busy: busy)
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(busy || !TelegramStep.codeReady(code))
                .padding(.top, 14)
                .accessibilityIdentifier("submitCode")
                Button {
                    code = ""
                    accounts.loginRestart()
                } label: {
                    Label { Text("Another number") } icon: { GlyphIcon(.pencil, size: 18) }
                        .labelStyle(SignalLabelStyle())
                        .font(RFont.sans(16, .semibold))
                        .foregroundStyle(Palette.ink2)
                        .frame(maxWidth: .infinity, minHeight: 46)
                }
                .buttonStyle(PressDimsStyle())
                .disabled(busy)
                .accessibilityIdentifier("restartLogin")
            }
            .padding(.horizontal, 20)
            .padding(.top, 4)
        case let .password(hint):
            SectionHeader("Two-step password", quiet: true)
            FlatFields {
                InputRow(glyph: .lock) {
                    SecureField("Telegram password", text: $password)
                        .textContentType(.password)
                        .focused($focus, equals: .password)
                        .submitLabel(.go)
                        .onSubmit(submitPassword)
                        .disabled(busy)
                        .accessibilityIdentifier("tgPassword")
                }
            }
            .onAppear { focus = .password }
            if let shown = hint.map({ untrustedName($0, limit: 40) }), !shown.isEmpty {
                Text("Hint: \(shown)")
                    .textRole(.sub)
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .padding(.horizontal, 20)
                    .padding(.top, 8)
            }
            Button(action: submitPassword) { BusyLabel(title: "Sign in", glyph: nil, busy: busy) }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(busy || password.isEmpty)
                .padding(.horizontal, 20)
                .padding(.top, 14)
                .accessibilityIdentifier("submitPassword")
        }
    }

    private func submitPassword() {
        guard !password.isEmpty, !accounts.busy else { return }
        let sent = password
        password = ""
        Task { await accounts.loginPassword(sent) }
    }
}

/// Telegram's code, one cell per digit (five, more if more are typed): the digits typed so far, the next cell ringed in
/// violet. The real field sits invisible underneath, so typing, pasting and the keyboard's code suggestion all work.
private struct DigitBoxes<Field: View>: View {
    var code: String
    var focused: Bool
    @ViewBuilder var field: Field

    var body: some View {
        let digits = Array(code)
        let count = max(5, digits.count)
        ZStack {
            field
                .foregroundStyle(.clear)
                .tint(.clear)
                .opacity(0.02)
            HStack(spacing: 8) {
                ForEach(0..<count, id: \.self) { i in
                    CodeCell(character: i < digits.count ? String(digits[i]) : "", next: i == digits.count && focused)
                        .animation(.easeOut(duration: 0.12), value: digits.count)
                }
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}
