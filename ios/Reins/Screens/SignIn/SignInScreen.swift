import SwiftUI

/// What signing in needs to know beyond the fields (the Android app's `SignInUi`).
struct SignInState: Equatable {
    var busy = false
    /// The server asked for a two-step code: the code field shows.
    var needsTotp = false
    var error: String?

    /// The hosted Reins server: the sign-in starts with it, and the server field stays hidden behind "Use another
    /// server" (`reins_proto::default_server!`).
    static let defaultServer = "https://app.reins2fa.com"

    /// Sign in is possible once a server address, an email and a password are there.
    static func canSubmit(server: String, email: String, password: String) -> Bool {
        server.trimmingCharacters(in: .whitespaces).count > "https://".count
            && !email.trimmingCharacters(in: .whitespaces).isEmpty
            && !password.isEmpty
    }
}

/// The server of the latest sign-in: the welcome offers it again after signing out (a server of one's own).
enum LastServer {
    private static let key = "signin.lastServer"

    static var value: String? {
        get { AppGroup.defaults.string(forKey: key) }
        set { AppGroup.defaults.set(newValue, forKey: key) }
    }

    /// The address to show on the welcome, when it is not the hosted server.
    static var toOffer: String? {
        guard let last = value?.trimmingCharacters(in: .whitespaces), !last.isEmpty,
              last.trimmingCharacters(in: CharacterSet(charactersIn: "/")).lowercased() != SignInState.defaultServer else { return nil }
        return last
    }
}

extension AppModel {
    /// `<server>/mcp`, the address AI clients connect to.
    var mcpAddress: String {
        guard case let .signedIn(info) = session else { return SignInState.defaultServer + "/mcp" }
        var server = info.serverUrl
        while server.hasSuffix("/") { server.removeLast() }
        return server + "/mcp"
    }
}

/// What creating an account checks before the server does (it checks the length again).
enum NewAccountRules {
    static let minPasswordLength = 12

    /// How hard a master password looks to guess (the Android app's `AccountRules.strength`): a hint only, the server's
    /// one rule is the length.
    enum Strength: String { case weak = "Weak", fair = "Fair", strong = "Strong" }

    /// Shorter than the minimum or very repetitive is weak; 20+ characters, or 16+ mixing three kinds (lower case, upper
    /// case, digits, other), is strong; anything else is fair.
    static func strength(_ password: String) -> Strength {
        guard password.count >= minPasswordLength, Set(password).count >= 5 else { return .weak }
        let kinds = [
            password.contains { $0.isLowercase }, password.contains { $0.isUppercase },
            password.contains { $0.isNumber }, password.contains { !$0.isLetter && !$0.isNumber },
        ].filter { $0 }.count
        if password.count >= 20 || (password.count >= 16 && kinds >= 3) { return .strong }
        return .fair
    }

    /// The line beside the strength.
    static func strengthHint(_ password: String) -> String {
        if password.count < minPasswordLength { return "Use at least \(minPasswordLength) characters." }
        switch strength(password) {
        case .weak: return "Too repetitive. Mix in other characters."
        case .fair: return "Longer, or mixing letters, digits and symbols, is stronger."
        case .strong: return "Hard to guess. Remember it, or write it down."
        }
    }

    /// The first thing that keeps the account from being created, in the order of the fields; nil when it can be.
    static func problem(server: String, email: String, password: String, again: String, terms: Bool) -> String? {
        if server.trimmingCharacters(in: .whitespaces).count <= "https://".count { return "Enter the server's address." }
        let e = email.trimmingCharacters(in: .whitespaces)
        if e.isEmpty || !e.contains("@") { return "Enter your email address." }
        if password.count < minPasswordLength { return "The master password needs at least \(minPasswordLength) characters." }
        if again != password { return "The two passwords do not match." }
        if !terms { return "Accept the Terms to continue." }
        return nil
    }
}

/// The signed-out root: a welcome whose way in is "Continue" (passwordless, through the server's sign-in page:
/// WorkOS AuthKit on the hosted server), with "Use another server" for self-hosters, which also offers the master
/// password forms. A centred card on wide screens, the full width on a phone. What follows a sign-in (the onboarding
/// steps) is `OnboardingScreen`; an account whose keys are on another phone goes to `UnlockScreen` first.
struct SignInScreen: View {
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var mode: AccountEntryView.Mode?
    @State private var sso = SsoSignIn()
    @State private var otherServer = false
    @State private var server = SignInState.defaultServer
    /// The server said it has no sign-in page (no SSO): "Continue" would only end on an error.
    @State private var noBrowserSignIn = false
    @FocusState private var serverFocused: Bool

    var body: some View {
        GeometryReader { geo in
            ScrollView {
                VStack(spacing: 0) {
                    Spacer(minLength: sizeClass == .regular ? 40 : 24)
                    Group {
                        if let mode {
                            AccountEntryView(mode: mode, server: server) { self.mode = nil }
                        } else {
                            welcome
                        }
                    }
                    .frame(maxWidth: 440)
                    .padding(.horizontal, sizeClass == .regular ? 32 : 24)
                    Spacer(minLength: 24)
                }
                .frame(maxWidth: .infinity, minHeight: geo.size.height)
            }
            .scrollDismissesKeyboard(.interactively)
            .scrollBounceBehavior(.basedOnSize)
        }
        .pageBackground()
        .animation(.smooth(duration: 0.25), value: mode)
        .animation(.smooth(duration: 0.25), value: otherServer)
        .animation(.smooth(duration: 0.25), value: sso.error)
        .onAppear {
            if model.demo && server == SignInState.defaultServer { server = DemoData.server }
            // Signed out of a server of one's own: offer it again rather than the hosted one.
            if !model.demo, server == SignInState.defaultServer, let last = LastServer.toOffer {
                server = last
                otherServer = true
            }
        }
        // Asked once the address stops changing: whether "Continue" works there at all.
        .task(id: serverUrl) {
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled, serverUrl.count > "https://".count else { return }
            let info = try? await model.core.serverInfo(serverUrl: serverUrl)
            guard !Task.isCancelled else { return }
            noBrowserSignIn = info?.browserSignIn == false
        }
    }

    private var serverUrl: String { server.trimmingCharacters(in: .whitespaces) }
    private var customServer: Bool {
        otherServer && serverUrl.trimmingCharacters(in: CharacterSet(charactersIn: "/")).lowercased() != SignInState.defaultServer
    }

    private var welcome: some View {
        VStack(alignment: .leading, spacing: 12) {
            BrandMark()
            Text("Your AI assistants ask, this phone decides: approve or deny what they want to read, send or change.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 14)
            if otherServer {
                TextField("Server", text: $server)
                    .textContentType(.URL)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .focused($serverFocused)
                    .disabled(sso.busy)
                    .fieldWell()
                    .accessibilityIdentifier("server")
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
            if noBrowserSignIn {
                Text("This server signs in with an email address and a master password.")
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("passwordFormsNote")
            } else {
                Button {
                    serverFocused = false
                    Task { await sso.run(model, feedback: feedback, server: serverUrl) }
                } label: {
                    HStack(spacing: 10) {
                        if sso.busy { ProgressView().tint(Palette.background) }
                        Text("Continue")
                    }
                }
                .buttonStyle(CapsuleButtonStyle(kind: .primary))
                .disabled(sso.busy || serverUrl.count <= "https://".count)
                .accessibilityIdentifier("continue")
                Text(customServer ? "Continue through your server's sign-in page." : "Sign in or create an account on the secure sign-in page.")
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error = sso.error {
                FormBanner(text: error).transition(.opacity)
            }
            if customServer || noBrowserSignIn {
                // A server of your own may have no sign-in page: its accounts use a master password.
                Button("Sign in with a master password") { mode = .signIn }
                    .buttonStyle(CapsuleButtonStyle(kind: noBrowserSignIn ? .primary : .secondary))
                    .disabled(sso.busy)
                    .padding(.top, 8)
                    .accessibilityIdentifier("signInChoice")
                Button {
                    mode = .create
                } label: {
                    Text("Create an account with a master password")
                        .font(RFont.sans(14, .medium))
                        .foregroundStyle(Palette.secondary)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                }
                .buttonStyle(.plain)
                .disabled(sso.busy)
                .accessibilityIdentifier("createAccount")
            } else {
                Button {
                    otherServer = true
                    serverFocused = true
                } label: {
                    Text("Use another server")
                        .font(RFont.sans(14, .medium))
                        .foregroundStyle(Palette.secondary)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                }
                .buttonStyle(.plain)
                .disabled(sso.busy)
                .padding(.top, 4)
                .accessibilityIdentifier("otherServer")
            }
        }
        .transition(.opacity)
    }
}

/// "Continue": the server's sign-in page in the system's web sheet, then the core makes or opens the account's keys
/// (`ssoFinish`). A closed sheet is no error. Where the account goes next is the model's (`AppModel.finishSso`).
@Observable
@MainActor
final class SsoSignIn {
    private(set) var busy = false
    var error: String?
    /// The web sheet; tests put a fake in its place.
    var browse: (URL, String) async throws -> URL = { url, scheme in try await WebAuth.run(url, callbackScheme: scheme) }

    func run(_ model: AppModel, feedback: Feedback, server: String) async {
        guard !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            let start = try await model.core.ssoBegin(serverUrl: server)
            guard let url = URL(string: start.url) else { throw WebAuth.Failure.failed("The server's sign-in address is not valid.") }
            let callback = model.demo
                ? URL(string: "\(start.callbackScheme)://sso-callback?code=demo&state=\(start.state)")!
                : try await browse(url, start.callbackScheme)
            let outcome = try await model.core.ssoFinish(
                serverUrl: server, callbackUrl: callback.absoluteString, state: start.state, verifier: start.verifier
            )
            feedback.play(.connected)
            await model.finishSso(outcome)
        } catch WebAuth.Failure.cancelled {
            // The person closed the page: back to the welcome as it was.
        } catch let WebAuth.Failure.failed(message) {
            feedback.play(.error)
            error = message
        } catch WebAuth.Failure.noWindow {
            error = "Open Reins and try again."
        } catch {
            feedback.play(.error)
            self.error = error.userMessage
        }
    }
}

/// The shield and the name, at the top of the signed-out pages.
struct BrandMark: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Image(systemName: "checkmark.shield.fill")
                .font(.system(size: 28, weight: .semibold))
                .foregroundStyle(Palette.accent)
                .frame(width: 56, height: 56)
                .background(Palette.accentSoft, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .accessibilityHidden(true)
                .padding(.bottom, 6)
            Text("Reins")
                .font(RFont.sans(34, .semibold))
                .foregroundStyle(Palette.text)
                .accessibilityAddTraits(.isHeader)
        }
    }
}

/// How an account gets onto this phone: creating one, or signing in to one (email, master password, and a two-step
/// code when the server asks for one). The server is the hosted one unless "Use another server" opens its field.
///
/// The welcome's "Continue" (`SsoSignIn`) is the way in on the hosted server; these forms are for a server of one's own
/// whose accounts use a master password. Both hand the session to the model (`finishSignIn`, `finishSso`).
struct AccountEntryView: View {
    enum Mode: Hashable { case create, signIn }
    var mode: Mode
    var onBack: () -> Void

    /// `server`: the one picked under "Use another server" on the welcome (its field shows here too).
    init(mode: Mode, server: String = SignInState.defaultServer, onBack: @escaping () -> Void) {
        self.mode = mode
        self.onBack = onBack
        _server = State(initialValue: server)
        _otherServer = State(initialValue: server != SignInState.defaultServer)
    }

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var state = SignInState()
    @State private var server: String
    @State private var otherServer: Bool
    @State private var email = ""
    // The password lives only here and in the call; it is never kept anywhere else.
    @State private var password = ""
    @State private var passwordAgain = ""
    @State private var terms = false
    @State private var totp = ""
    @FocusState private var focus: Field?

    private enum Field: Hashable { case server, email, password, passwordAgain, totp }

    private var creating: Bool { mode == .create }

    private var problem: String? {
        NewAccountRules.problem(server: server, email: email, password: password, again: passwordAgain, terms: terms)
    }

    private var canSubmit: Bool {
        creating ? problem == nil : SignInState.canSubmit(server: server, email: email, password: password)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GlassIconButton(symbol: "chevron.left", label: "Back", action: onBack)
                .disabled(state.busy)
                .accessibilityIdentifier("back")
                .padding(.bottom, 8)
            Text(creating ? "Create your account" : "Sign in")
                .font(RFont.sans(30, .semibold))
                .foregroundStyle(Palette.text)
                .accessibilityAddTraits(.isHeader)
            Text(creating
                ? "One account for this phone and your computers. This phone approves what your AI assistants ask for."
                : "Sign in with your Reins account. This phone approves what your AI assistants ask for.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 10)

            if otherServer {
                field("Server", text: $server, field: .server, next: .email, content: .URL, keyboard: .URL)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
            field("Email", text: $email, field: .email, next: .password, content: creating ? .emailAddress : .username, keyboard: .emailAddress)
            SecureField("Master password", text: $password)
                // Creating: no password AutoFill. A master password is one to remember and write down, not a generated
                // one, and with two secure fields iOS's new-password handling empties the first at every keystroke
                // where it cannot offer one (the simulator). `.oneTimeCode` is what keeps it out.
                .textContentType(creating ? .oneTimeCode : .password)
                .submitLabel(creating || state.needsTotp ? .next : .go)
                .focused($focus, equals: .password)
                .onSubmit {
                    if creating { focus = .passwordAgain } else if state.needsTotp { focus = .totp } else { submit() }
                }
                .disabled(state.busy)
                .fieldWell()
                .accessibilityIdentifier("password")
            if creating { createFields }
            if state.needsTotp {
                TextField("Two-step code", text: Binding(get: { totp }, set: { totp = String($0.filter { !$0.isWhitespace }.prefix(10)) }))
                    .textContentType(.oneTimeCode)
                    .keyboardType(.numberPad)
                    .submitLabel(.go)
                    .focused($focus, equals: .totp)
                    .onSubmit(submit)
                    .disabled(state.busy)
                    .fieldWell(mono: true)
                    .accessibilityIdentifier("totp")
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
            if let error = state.error {
                FormBanner(text: error).transition(.opacity)
            }
            Button(action: submit) {
                HStack(spacing: 10) {
                    if state.busy { ProgressView().tint(Palette.background) }
                    Text(creating ? "Create account" : "Sign in")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary))
            .disabled(!canSubmit || state.busy)
            .opacity(canSubmit ? 1 : 0.45)
            .padding(.top, 4)
            .accessibilityIdentifier(creating ? "create" : "signIn")
            if !otherServer {
                Button {
                    otherServer = true
                    focus = .server
                } label: {
                    Text("Use another server")
                        .font(RFont.sans(14, .medium))
                        .foregroundStyle(Palette.secondary)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                }
                .buttonStyle(.plain)
                .disabled(state.busy)
                .accessibilityIdentifier("otherServer")
            }
        }
        .animation(.smooth(duration: 0.25), value: state)
        .animation(.smooth(duration: 0.25), value: otherServer)
        .transition(.opacity)
        .onAppear {
            if model.demo && server == SignInState.defaultServer { server = DemoData.server }
        }
    }

    private func strengthTint(_ s: NewAccountRules.Strength) -> Color {
        switch s {
        case .weak: Palette.danger
        case .fair: Palette.warning
        case .strong: Palette.success
        }
    }

    /// The second password, what is still missing, the warning that nobody can recover the password, the Terms.
    @ViewBuilder private var createFields: some View {
        // How hard the first one is to guess, once there is something to judge.
        if !password.isEmpty {
            let strength = NewAccountRules.strength(password)
            Text("\(Text(strength.rawValue).fontWeight(.semibold).foregroundStyle(strengthTint(strength))) · \(NewAccountRules.strengthHint(password))")
                .font(RFont.sans(13.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("passwordStrength")
        }
        SecureField("Master password again", text: $passwordAgain)
            .textContentType(.oneTimeCode)
            .submitLabel(.done)
            .focused($focus, equals: .passwordAgain)
            .onSubmit { focus = nil }
            .disabled(state.busy)
            .fieldWell()
            .accessibilityIdentifier("passwordAgain")
        // Always there, so the fields above keep their place (and their focus) as the hint changes.
        let mismatch = !passwordAgain.isEmpty && passwordAgain != password && password.count >= NewAccountRules.minPasswordLength
        Text(mismatch ? "The two passwords do not match." : "At least \(NewAccountRules.minPasswordLength) characters.")
            .font(RFont.sans(13.5))
            .foregroundStyle(mismatch ? Palette.danger : Palette.tertiary)
            .accessibilityIdentifier("passwordHint")
        Banner("Nobody can recover or reset your master password, not even Reins: it is what encrypts your data. Write it down and keep it somewhere safe.", kind: .warning)
        HStack(alignment: .center, spacing: 12) {
            Text("I accept the [Terms](https://reins2fa.com/terms) and the [Privacy Policy](https://reins2fa.com/privacy).")
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.secondary)
                .tint(Palette.accent)
                .frame(maxWidth: .infinity, alignment: .leading)
            Toggle("Accept the Terms", isOn: $terms)
                .labelsHidden()
                .tint(Palette.accent)
                .disabled(state.busy)
                .accessibilityIdentifier("acceptTerms")
                .onChange(of: terms) { _, on in feedback.play(.toggle(on)) }
        }
        .padding(.vertical, 4)
    }

    private func field(
        _ title: String, text: Binding<String>, field: Field, next: Field, content: UITextContentType, keyboard: UIKeyboardType
    ) -> some View {
        TextField(title, text: text)
            .textContentType(content)
            .keyboardType(keyboard)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .submitLabel(.next)
            .focused($focus, equals: field)
            .onSubmit { focus = next }
            .disabled(state.busy)
            .fieldWell()
            .accessibilityIdentifier(title.lowercased())
    }

    private func submit() {
        guard !state.busy, canSubmit else { return }
        state.busy = true
        state.error = nil
        focus = nil
        let (server, email, password) = (server.trimmingCharacters(in: .whitespaces), email.trimmingCharacters(in: .whitespaces), password)
        let code = totp.trimmingCharacters(in: .whitespaces)
        let creating = creating
        Task {
            do {
                let info = creating
                    ? try await model.core.createAccount(serverUrl: server, email: email, password: password)
                    : try await model.core.login(serverUrl: server, email: email, password: password, totp: code.isEmpty ? nil : code)
                self.password = ""
                self.passwordAgain = ""
                self.totp = ""
                feedback.play(.connected)
                await model.finishSignIn(info)
                state = SignInState()
            } catch CoreError.TwoFactorRequired {
                // Not a failure: one more thing is needed from the user.
                feedback.play(.alert)
                state = SignInState(needsTotp: true, error: CoreError.TwoFactorRequired.userMessage)
                focus = .totp
            } catch {
                feedback.play(.error)
                state.busy = false
                state.error = error.userMessage
            }
        }
    }
}
