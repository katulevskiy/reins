import SwiftUI

/// What signing in needs to know beyond the fields (the Android app's `SignInUi`).
struct SignInState: Equatable {
    var busy = false
    /// The server asked for a two-step code: the code field shows.
    var needsTotp = false
    var error: String?

    /// The hosted Reins server: the sign-in starts with it, and the server field stays hidden behind "Use another
    /// server" (`rewarden_proto::default_server!`).
    static let defaultServer = "https://app.reins2fa.com"

    /// Sign in is possible once a server address, an email and a password are there.
    static func canSubmit(server: String, email: String, password: String) -> Bool {
        server.trimmingCharacters(in: .whitespaces).count > "https://".count
            && !email.trimmingCharacters(in: .whitespaces).isEmpty
            && !password.isEmpty
    }
}

/// What creating an account checks before the server does (it checks the length again).
enum NewAccountRules {
    static let minPasswordLength = 12

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

/// The signed-out root: a welcome with the two ways in, then the account entry for the one picked. A centred card on
/// wide screens, the full width on a phone. What follows a sign-in (the onboarding steps) is `OnboardingScreen`.
struct SignInScreen: View {
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var mode: AccountEntryView.Mode?

    var body: some View {
        GeometryReader { geo in
            ScrollView {
                VStack(spacing: 0) {
                    Spacer(minLength: sizeClass == .regular ? 40 : 24)
                    Group {
                        if let mode {
                            AccountEntryView(mode: mode) { self.mode = nil }
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
    }

    private var welcome: some View {
        VStack(alignment: .leading, spacing: 12) {
            BrandMark()
            Text("Your AI assistants ask, this phone decides: approve or deny what they want to read, send or change.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 14)
            Button("Create account") { mode = .create }
                .buttonStyle(CapsuleButtonStyle(kind: .primary))
                .accessibilityIdentifier("createAccount")
            Button("Sign in") { mode = .signIn }
                .buttonStyle(CapsuleButtonStyle(kind: .secondary))
                .accessibilityIdentifier("signInChoice")
        }
        .transition(.opacity)
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
/// This is the one seam for the ways in: other sign-in methods (WorkOS AuthKit's "Continue") replace or join these
/// fields here, and hand the session to `AppModel.finishSignIn` like they do.
struct AccountEntryView: View {
    enum Mode: Hashable { case create, signIn }
    var mode: Mode
    var onBack: () -> Void

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var state = SignInState()
    @State private var server = SignInState.defaultServer
    @State private var otherServer = false
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

    /// The second password, what is still missing, the warning that nobody can recover the password, the Terms.
    @ViewBuilder private var createFields: some View {
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
