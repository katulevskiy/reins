import SwiftUI

/// What signing in needs to know beyond the fields (the Android app's `SignInUi`).
struct SignInState: Equatable {
    var busy = false
    /// The server asked for a two-step code: the code field shows.
    var needsTotp = false
    var error: String?

    /// Sign in is possible once a server address, an email and a password are there.
    static func canSubmit(server: String, email: String, password: String) -> Bool {
        server.trimmingCharacters(in: .whitespaces).count > "https://".count
            && !email.trimmingCharacters(in: .whitespaces).isEmpty
            && !password.isEmpty
    }
}

/// Signing in to a Reins server: its address, the account's email and master password, and a two-step code when the
/// server asks for one. A centred card on wide screens, the full width on a phone.
struct SignInScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var state = SignInState()
    @State private var server = "https://"
    @State private var email = ""
    // The password lives only here and in the call; it is never kept anywhere else.
    @State private var password = ""
    @State private var totp = ""
    @FocusState private var focus: Field?

    private enum Field: Hashable { case server, email, password, totp }

    var body: some View {
        GeometryReader { geo in
            ScrollView {
                VStack(spacing: 0) {
                    Spacer(minLength: sizeClass == .regular ? 40 : 24)
                    card
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
        .onAppear {
            if model.demo && server == "https://" { server = DemoData.server }
        }
    }

    private var card: some View {
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
            Text("Sign in with your Reins account. This phone approves what your AI assistants ask for.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 10)

            field("Server", text: $server, field: .server, next: .email, content: .URL, keyboard: .URL)
            field("Email", text: $email, field: .email, next: .password, content: .username, keyboard: .emailAddress)
            SecureField("Master password", text: $password)
                .textContentType(.password)
                .submitLabel(state.needsTotp ? .next : .go)
                .focused($focus, equals: .password)
                .onSubmit { state.needsTotp ? (focus = .totp) : submit() }
                .disabled(state.busy)
                .fieldWell()
                .accessibilityIdentifier("password")
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
                    Text("Sign in")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary))
            .disabled(!SignInState.canSubmit(server: server, email: email, password: password) || state.busy)
            .opacity(SignInState.canSubmit(server: server, email: email, password: password) ? 1 : 0.45)
            .padding(.top, 4)
            .accessibilityIdentifier("signIn")
        }
        .animation(.smooth(duration: 0.25), value: state)
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
        guard !state.busy, SignInState.canSubmit(server: server, email: email, password: password) else { return }
        state.busy = true
        state.error = nil
        let (server, email, password) = (server.trimmingCharacters(in: .whitespaces), email.trimmingCharacters(in: .whitespaces), password)
        let code = totp.trimmingCharacters(in: .whitespaces)
        Task {
            do {
                let info = try await model.core.login(serverUrl: server, email: email, password: password, totp: code.isEmpty ? nil : code)
                self.password = ""
                self.totp = ""
                model.registrationError = nil
                await model.signedIn(info)
                // A fresh sign-in takes the approval role, even from a phone another one took it from.
                if !model.approvalDevice {
                    do {
                        try await model.registerDevice(force: true)
                    } catch {
                        model.registrationError = error.userMessage
                    }
                }
                state = SignInState()
            } catch CoreError.TwoFactorRequired {
                state = SignInState(needsTotp: true, error: CoreError.TwoFactorRequired.userMessage)
                focus = .totp
            } catch {
                state.busy = false
                state.error = error.userMessage
            }
        }
    }
}
