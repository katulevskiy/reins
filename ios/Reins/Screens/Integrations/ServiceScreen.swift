import SwiftUI

/// The accounts of one integration and the way to add another, which depends on the kind of service.
struct ServiceScreen: View {
    var serviceId: String

    var body: some View {
        AccountsScreen(serviceId: serviceId)
    }
}

/// One integration's page (Gmail's too): its accounts with their status, then connecting another one.
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
                    content(service, accounts)
                } else {
                    EmptyState(symbol: "puzzlepiece.extension", title: "Integration", message: "This integration is not available.")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .pageBackground()
                        .navigationTitle("Integration")
                }
            } else {
                Color.clear.pageBackground()
            }
        }
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
        List {
            Section {
                if service.accounts.isEmpty {
                    NoAccountsRow(title: ServiceCopy.emptyTitle(service), message: ServiceCopy.intro(service))
                }
                ForEach(service.accounts, id: \.account) { account in
                    AccountRowView(
                        title: ServiceCopy.accountTitle(service, account.account),
                        status: m.statuses[account.account],
                        needsAgain: ServiceCopy.needsAgain(service),
                        canAllowAgain: ServiceCopy.canAllowAgain(service),
                        busy: m.busy,
                        avatar: {
                            if service.kind == "device" {
                                ServiceAvatar(service: service.service, size: 40)
                            } else {
                                BlobAvatar(seed: account.account, size: 40)
                            }
                        },
                        onAllowAgain: { allowAgain(service, account.account, m) },
                        onRemove: { removing = account.account }
                    )
                    .accessibilityIdentifier("account:\(account.account)")
                }
            } header: {
                GroupHeading("Accounts")
            } footer: {
                GroupFootnote(
                    [ServiceCopy.intro(service), ServiceCopy.accountsFooter(service), ServiceCopy.fineprint(service)]
                        .filter { !$0.isEmpty }
                        .joined(separator: "\n\n")
                )
            }
            .listRowBackground(Palette.elevated)

            Section {
                VStack(alignment: .leading, spacing: 12) {
                    if service.available {
                        AddAccountControls(service: service, accounts: m)
                    } else {
                        IntegrationBanner(text: untrusted(service.note ?? "Not available in this build."), kind: .warning)
                            .accessibilityIdentifier("unavailable")
                    }
                    if let error = m.error {
                        IntegrationBanner(text: untrusted(error), kind: .error)
                            .accessibilityIdentifier("accountError")
                        if m.refusedByIOS {
                            ActionButton(title: "Open Settings", symbol: "gear", kind: .secondary) {
                                if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                            }
                        }
                    }
                }
                .plainListRow(top: 4, bottom: 28)
            }
        }
        .integrationList()
        .navigationTitle(service.name)
        .navigationSubtitle("Integration")
        .navigationBarTitleDisplayMode(.large)
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

    var body: some View {
        switch service.kind {
        case "google":
            VStack(alignment: .leading, spacing: 10) {
                if !ServiceCopy.googleSignInNote.isEmpty {
                    Text(ServiceCopy.googleSignInNote)
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.tertiary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("googleSignInNote")
                }
                ActionButton(title: "Add account", symbol: "plus", busy: accounts.busy) {
                    Task { await accounts.addGoogle() }
                }
                .accessibilityIdentifier("addAccount")
            }
        case "device":
            if service.accounts.isEmpty {
                ActionButton(title: "Allow on this phone", symbol: "iphone", busy: accounts.busy) {
                    Task { await accounts.addDevice() }
                }
                .accessibilityIdentifier("allowDevice")
            }
        case "token":
            if let host = GitHosts.of(service.service) {
                GitHostConnect(host: host, accounts: accounts)
            } else {
                GitHubConnect(accounts: accounts)
            }
        case "vault":
            if service.accounts.isEmpty {
                SecretForm(
                    placeholder: "Master password",
                    button: "Unlock and connect",
                    busy: accounts.busy,
                    hint: "The vault of the account this phone is signed in with.",
                    contentType: .password
                ) { secret in Task { await accounts.addSecret(secret) } }
            }
        case "telegram":
            TelegramLogin(accounts: accounts)
        default:
            Text("This kind of integration cannot be added here.")
                .font(RFont.sans(14))
                .foregroundStyle(Palette.secondary)
        }
    }
}

// MARK: Tokens

/// GitHub in a few taps: open GitHub's token page (already filled in), press Generate and Copy there, come back and
/// paste. A pasted token that looks like one connects at once; typing one in by hand is still possible.
private struct GitHubConnect: View {
    var accounts: AccountsModel
    @Environment(\.openURL) private var openURL
    @State private var opened = false
    @State private var manual = false

    var body: some View {
        ActionButton(title: "Chosen repositories", symbol: "key.fill", enabled: !accounts.busy) {
            open(GitHubToken.fineGrainedURL())
        }
        .accessibilityIdentifier("openGithub")
        ActionButton(title: "Everything", kind: .secondary, enabled: !accounts.busy) {
            open(GitHubToken.classicURL())
        }
        .accessibilityIdentifier("openGithubClassic")
        Steps(
            "Either button opens GitHub with the token already set up. Tap Generate token, copy it, and come back to paste it.\n\n" +
                "Chosen repositories: a fine-grained token, only for the repositories you pick (no gists or notifications).\n\n" +
                "Everything: a classic token for every repository, plus gists and notifications."
        )
        if opened {
            CopiedTokenButton(busy: accounts.busy, looksLikeToken: GitHubToken.looksLikeToken, hostName: "GitHub") { token in
                Task { await accounts.addSecret(token) }
            }
        }
        if !manual {
            ActionButton(title: "I already have a token", kind: .ghost, enabled: !accounts.busy) { manual = true }
                .accessibilityIdentifier("pasteManually")
        } else {
            SecretForm(placeholder: "Access token", button: "Connect", busy: accounts.busy, hint: "") { token in
                Task { await accounts.addSecret(token) }
            }
        }
    }

    private func open(_ url: String) {
        opened = true
        if let u = URL(string: url) { openURL(u) }
    }
}

/// A git host besides GitHub: open its token page, create and copy the token there, come back and paste it.
private struct GitHostConnect: View {
    var host: GitHost
    var accounts: AccountsModel
    @Environment(\.openURL) private var openURL
    @State private var opened = false
    @State private var manual = false

    var body: some View {
        ActionButton(title: "Create a token on \(host.name)", symbol: "key.fill", enabled: !accounts.busy) {
            opened = true
            if let u = URL(string: host.tokenURL()) { openURL(u) }
        }
        .accessibilityIdentifier("openTokenPage")
        Steps(host.steps)
            .accessibilityIdentifier("tokenSteps")
        if opened && host.shape != nil {
            CopiedTokenButton(busy: accounts.busy, looksLikeToken: host.looksLikeToken, hostName: host.name) { token in
                Task { await accounts.addSecret(token) }
            }
        }
        if !(manual || host.pasteFirst) {
            ActionButton(title: "I already have a token", kind: .ghost, enabled: !accounts.busy) { manual = true }
                .accessibilityIdentifier("pasteManually")
        } else {
            SecretForm(placeholder: host.placeholder, button: "Connect", busy: accounts.busy, hint: host.hint) { token in
                Task { await accounts.addSecret(token) }
            }
        }
    }
}

/// "Paste the token I copied": the system's paste button (no "Allow Paste" prompt). Only something shaped like the
/// host's token is used, and it is taken off the clipboard once read.
private struct CopiedTokenButton: View {
    var busy: Bool
    var looksLikeToken: (String?) -> Bool
    var hostName: String
    var onToken: (String) -> Void
    @Environment(\.feedback) private var feedback
    @State private var wrong = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                Text("Copied the token on \(hostName)? Paste it here and it connects.")
                    .font(RFont.sans(14, .medium))
                    .foregroundStyle(Palette.text)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                PasteButton(payloadType: String.self) { strings in
                    Task { @MainActor in
                        let text = strings.first?.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard let text, looksLikeToken(text) else {
                            feedback.play(.error)
                            wrong = true
                            return
                        }
                        wrong = false
                        clearClipboard()
                        onToken(text)
                    }
                }
                .buttonBorderShape(.capsule)
                .tint(Palette.accent)
                .disabled(busy)
                .accessibilityIdentifier("useCopied")
            }
            .padding(.leading, 16)
            .padding(.trailing, 10)
            .padding(.vertical, 10)
            .background(Palette.accentSoft, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            if wrong {
                Text("That is not a \(hostName) token. Copy it again on \(hostName), or type it in below.")
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.danger)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 4)
            }
        }
    }
}

/// What to do on a token page.
private struct Steps: View {
    var text: String
    init(_ text: String) { self.text = text }

    // How-tos wait behind a (?).
    var body: some View { GroupFooter(text) }
}

// MARK: Telegram

/// Phone number → code → (password): the same steps as signing in to Telegram anywhere else.
private struct TelegramLogin: View {
    var accounts: AccountsModel
    @State private var phone = ""
    @State private var code = ""
    @State private var password = ""
    @FocusState private var focus: Field?

    private enum Field { case phone, code, password }

    var body: some View {
        let busy = accounts.busy
        switch accounts.login {
        case .phone:
            InputWell {
                TextField("Phone number, like +1 555 010 0100", text: $phone)
                    .font(RFont.mono(16))
                    .keyboardType(.phonePad)
                    .textContentType(.telephoneNumber)
                    .focused($focus, equals: .phone)
                    .disabled(busy)
                    .accessibilityIdentifier("phone")
            }
            ActionButton(title: "Send me a code", busy: busy, enabled: TelegramStep.phoneReady(phone)) {
                focus = nil
                Task { await accounts.loginBegin(phone) }
            }
            .accessibilityIdentifier("sendCode")
        case let .code(number):
            Steps("Telegram sent a code to \(number). It arrives in the Telegram app on your other devices, or as a text.")
            InputWell {
                TextField("Login code", text: Binding(get: { code }, set: { code = TelegramStep.cleanCode($0) }))
                    .font(RFont.mono(20, .medium))
                    .keyboardType(.numberPad)
                    .textContentType(.oneTimeCode)
                    .focused($focus, equals: .code)
                    .disabled(busy)
                    .accessibilityIdentifier("code")
            }
            .onAppear { focus = .code }
            ActionButton(title: "Sign in", busy: busy, enabled: TelegramStep.codeReady(code)) {
                let sent = code
                code = ""
                Task { await accounts.loginCode(sent) }
            }
            .accessibilityIdentifier("submitCode")
            ActionButton(title: "Use another number", kind: .ghost, enabled: !busy) {
                code = ""
                accounts.loginRestart()
            }
            .accessibilityIdentifier("restartLogin")
        case let .password(hint):
            Steps("This account has two-step verification. Enter its Telegram password" + (hint.map(untrusted).flatMap { $0.isEmpty ? nil : " (hint: \($0))" } ?? "") + ".")
            InputWell {
                SecureField("Telegram password", text: $password)
                    .font(RFont.sans(16))
                    .textContentType(.password)
                    .focused($focus, equals: .password)
                    .submitLabel(.go)
                    .onSubmit(submitPassword)
                    .disabled(busy)
                    .accessibilityIdentifier("tgPassword")
            }
            .onAppear { focus = .password }
            ActionButton(title: "Sign in", busy: busy, enabled: !password.isEmpty, action: submitPassword)
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
