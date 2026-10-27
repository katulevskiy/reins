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
        case .fair: return "Longer is stronger."
        case .strong: return ""
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

/// The signed-out root (the Arrival board, 01): the shipped wallpaper full-bleed (Torii dither in light, Halo in dark)
/// and, on a frosted pane over its lower half, "Your AI asks, *You decide.*" and the way in: "Continue" (passwordless,
/// through the server's sign-in page: WorkOS AuthKit on the hosted server). "Other server" is for self-hosters; only a
/// server of one's own offers the master password forms. What follows a sign-in (the onboarding steps) is
/// `OnboardingScreen`; an account whose keys are on another phone goes to `UnlockScreen` first.
struct SignInScreen: View {
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var mode: AccountEntryView.Mode?
    @State private var sso = SsoSignIn()
    @State private var otherServer = false
    @State private var server = SignInState.defaultServer
    /// The server said it has no sign-in page (no SSO): "Continue" would only end on an error.
    @State private var noBrowserSignIn = false
    @FocusState private var serverFocused: Bool

    var body: some View {
        Group {
            if let mode {
                AccountEntryView(mode: mode, server: server) { self.mode = nil }
                    .transition(.opacity)
            } else {
                welcome.transition(.opacity)
            }
        }
        .animation(reduceMotion ? .easeInOut(duration: 0.15) : .smooth(duration: 0.25), value: mode)
        .animation(.smooth(duration: 0.25), value: otherServer)
        .animation(.smooth(duration: 0.25), value: sso.error)
        .onAppear {
            if model.demo && server == SignInState.defaultServer { server = DemoData.server }
            #if DEBUG
            // `-accountForm`: the form for a new account on a server of one's own (screenshots).
            if model.demo, ProcessInfo.processInfo.arguments.contains("-accountForm") {
                otherServer = true
                mode = .create
            }
            // `-autoContinue`: Continue at once (screenshots of what follows a sign-in, the Unlock screen).
            if model.demo, ProcessInfo.processInfo.arguments.contains("-autoContinue") {
                Task { await sso.run(model, feedback: feedback, server: serverUrl) }
            }
            #endif
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
        GeometryReader { geo in
            ZStack(alignment: .top) {
                Palette.paper.ignoresSafeArea()
                WelcomeArt()
                ScrollView {
                    VStack(spacing: 0) {
                        Spacer(minLength: 0)
                        pane
                            .frame(maxWidth: 440)
                            .padding(.horizontal, 12)
                            .padding(.bottom, 4)
                    }
                    .frame(maxWidth: .infinity, minHeight: geo.size.height)
                }
                .scrollDismissesKeyboard(.interactively)
                .scrollBounceBehavior(.basedOnSize)
            }
        }
    }

    /// The words and the way in, on the frosted pane.
    private var pane: some View {
        VStack(alignment: .leading, spacing: 22) {
            (Text("Your AI asks,\n").font(RFont.display(44)).foregroundStyle(Palette.ink)
                + Text("You decide.").font(RFont.display(44, target: true)).foregroundStyle(Palette.kindAccounts))
                .tracking(RFont.displayTracking(44))
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            VStack(spacing: 6) {
                if otherServer {
                    HStack(spacing: 10) {
                        GlyphIcon(.server, size: 18).foregroundStyle(Palette.inkMeta)
                        TextField("Server", text: $server)
                            .font(RFont.mono(15))
                            .textContentType(.URL)
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .submitLabel(.done)
                            .focused($serverFocused)
                            .disabled(sso.busy)
                            .accessibilityIdentifier("server")
                    }
                    .padding(.horizontal, 16)
                    .frame(minHeight: 50)
                    .background(Palette.paperSunken.opacity(0.8), in: Capsule())
                    .padding(.bottom, 4)
                    .transition(.opacity.combined(with: .move(edge: .top)))
                }
                if noBrowserSignIn {
                    Text("Email and master password")
                        .textRole(.meta)
                        .foregroundStyle(Palette.inkMeta)
                        .frame(maxWidth: .infinity)
                        .accessibilityIdentifier("passwordFormsNote")
                } else {
                    Button {
                        serverFocused = false
                        Task { await sso.run(model, feedback: feedback, server: serverUrl) }
                    } label: {
                        HStack(spacing: 10) {
                            if sso.busy { ProgressView().tint(Palette.onInverse) }
                            Text("Continue")
                        }
                    }
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .disabled(sso.busy || serverUrl.count <= "https://".count)
                    .accessibilityIdentifier("continue")
                }
                if let error = sso.error {
                    FormBanner(text: error).padding(.top, 4).transition(.opacity)
                }
                if customServer || noBrowserSignIn {
                    // A server of one's own may have no sign-in page: its accounts use a master password. Never on the
                    // hosted server, which is passwordless.
                    Button("Sign in with a master password") { mode = .signIn }
                        .buttonStyle(SignalButtonStyle(noBrowserSignIn ? .inverse : .soft))
                        .disabled(sso.busy)
                        .padding(.top, 4)
                        .accessibilityIdentifier("signInChoice")
                    PaneLink(title: "Create an account", glyph: .person) { mode = .create }
                        .disabled(sso.busy)
                        .accessibilityIdentifier("createAccount")
                } else {
                    PaneLink(title: "Other server", glyph: .server) {
                        otherServer = true
                        serverFocused = true
                    }
                    .disabled(sso.busy)
                    .accessibilityIdentifier("otherServer")
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 26)
        .padding(.bottom, 18)
        .background(FrostedPane(radius: 30))
    }
}

/// A quiet link under the main button (the kit's `lnk`): a glyph and words in ink-2, 46 pt tall.
private struct PaneLink: View {
    var title: String
    var glyph: Glyph
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            Label { Text(title) } icon: { GlyphIcon(glyph, size: 18) }
                .labelStyle(SignalLabelStyle())
                .font(RFont.sans(16, .semibold))
                .foregroundStyle(Palette.ink2)
                .frame(maxWidth: .infinity, minHeight: 46)
                .contentShape(Rectangle())
        }
        .buttonStyle(PressDimsStyle())
    }
}

/// The welcome's wallpaper: the shipped default for the theme, 620 pt tall from the top of the screen, already faded
/// into the page at its bottom.
private struct WelcomeArt: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        GeometryReader { geo in
            if let image = Wallpaper.defaultFor(scheme).image(scheme) {
                Image(uiImage: image)
                    .resizable()
                    .interpolation(.medium)
                    .scaledToFill()
                    .frame(width: geo.size.width, height: 620, alignment: .top)
                    .clipped()
                    .backdropArt(key: "welcome-\(scheme == .dark ? "dark" : "light")", image: image, dark: scheme == .dark)
            }
        }
        .frame(height: 620)
        .ignoresSafeArea(edges: .top)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
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
            if await model.finishSignedInDespite(error) { return }
            feedback.play(.error)
            self.error = error.userMessage
        }
    }
}

/// The R and the name (the privacy cover). `ReinsMark` has a light and a dark drawing (scripts/brand/render-logo.sh).
struct BrandMark: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Image("ReinsMark")
                .resizable()
                .scaledToFit()
                .frame(width: 56, height: 56)
                .accessibilityHidden(true)
                .padding(.bottom, 6)
            Text("Reins")
                .font(RFont.sans(34, .semibold))
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
        }
    }
}

/// How an account gets onto a server of one's own with a master password (the Arrival board, 02): creating one, or
/// signing in to one (email, master password, and a two-step code when the server asks). The hosted server is
/// passwordless: these forms are only reached from "Other server". Both hand the session to the model (`finishSignIn`).
struct AccountEntryView: View {
    enum Mode: Hashable { case create, signIn }
    var mode: Mode
    var onBack: () -> Void

    /// `server`: the one picked under "Other server" on the welcome (its field shows here too).
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

    private var mismatchNow: Bool {
        !passwordAgain.isEmpty && passwordAgain != password && password.count >= NewAccountRules.minPasswordLength
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                RoundHeaderButton(glyph: .chevronLeft, label: "Back", action: onBack)
                    .disabled(state.busy)
                    .accessibilityIdentifier("back")
                    .padding(.horizontal, 20)
                    .padding(.top, 12)
                HStack(spacing: 14) {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .fill(Palette.fieldAccounts)
                        .overlay { Grain().clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous)) }
                        .overlay { GlyphIcon(creating ? .person : .key, size: 22).foregroundStyle(.white) }
                        .frame(width: 48, height: 48)
                        .accessibilityHidden(true)
                    Text(creating ? "New account" : "Sign in")
                        .textRole(.title)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                }
                .padding(.horizontal, 20)
                .padding(.top, 14)
                RaisedFields {
                    if otherServer {
                        field("Server", glyph: .server, text: $server, field: .server, next: .email, content: .URL, keyboard: .URL, mono: true)
                    }
                    field("Email", glyph: .mail, text: $email, field: .email, next: .password, content: creating ? .emailAddress : .username, keyboard: .emailAddress)
                    GlyphInputRow(glyph: .lock) {
                        SecureField("Master password", text: $password)
                            // Creating: no password AutoFill. A master password is one to remember and write down, not a
                            // generated one, and with two secure fields iOS's new-password handling empties the first at
                            // every keystroke where it cannot offer one (the simulator). `.oneTimeCode` keeps it out.
                            .textContentType(creating ? .oneTimeCode : .password)
                            .submitLabel(creating || state.needsTotp ? .next : .go)
                            .focused($focus, equals: .password)
                            .onSubmit {
                                if creating { focus = .passwordAgain } else if state.needsTotp { focus = .totp } else { submit() }
                            }
                            .disabled(state.busy)
                            .accessibilityIdentifier("password")
                    } trailing: {
                        if creating && !password.isEmpty { StrengthMeter(password: password) }
                    }
                    if creating {
                        GlyphInputRow(glyph: .lock) {
                            SecureField("Master password again", text: $passwordAgain)
                                .textContentType(.oneTimeCode)
                                .submitLabel(.done)
                                .focused($focus, equals: .passwordAgain)
                                .onSubmit { focus = nil }
                                .disabled(state.busy)
                                .accessibilityIdentifier("passwordAgain")
                        } trailing: {
                            if !passwordAgain.isEmpty && passwordAgain == password && password.count >= NewAccountRules.minPasswordLength {
                                GlyphIcon(.check, size: 18).foregroundStyle(Palette.success).accessibilityLabel("The passwords match")
                            }
                        }
                    }
                    if state.needsTotp {
                        GlyphInputRow(glyph: .shieldCheck) {
                            TextField("Two-step code", text: Binding(get: { totp }, set: { totp = String($0.filter { !$0.isWhitespace }.prefix(10)) }))
                                .font(RFont.mono(15))
                                .textContentType(.oneTimeCode)
                                .keyboardType(.numberPad)
                                .submitLabel(.go)
                                .focused($focus, equals: .totp)
                                .onSubmit(submit)
                                .disabled(state.busy)
                                .accessibilityIdentifier("totp")
                        }
                        .transition(.opacity)
                    }
                }
                .padding(.top, 22)
                if creating {
                    VStack(alignment: .leading, spacing: 6) {
                        // Always there, so the fields above keep their place (and their focus) as the hint changes.
                        Text(mismatchNow ? "The two passwords do not match." : "At least \(NewAccountRules.minPasswordLength) characters.")
                            .textRole(.sub)
                            .foregroundStyle(mismatchNow ? Palette.danger : Palette.inkMeta)
                            .accessibilityIdentifier("passwordHint")
                        Label { Text("Nobody can reset your master password. Forget it and the account is lost.") } icon: { GlyphIcon(.warning, size: 14) }
                            .labelStyle(SignalLabelStyle())
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.warning)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("masterPasswordWarning")
                    }
                    .padding(.horizontal, 24)
                    .padding(.top, 10)
                    RaisedGroup {
                        HStack(spacing: 12) {
                            Text("[Terms](https://reins2fa.com/terms) · [Privacy](https://reins2fa.com/privacy)")
                                .font(RFont.sans(16, .medium))
                                .foregroundStyle(Palette.ink)
                                .tint(Palette.ink)
                                .underline()
                                .frame(maxWidth: .infinity, alignment: .leading)
                            Toggle("Accept the Terms and the Privacy Policy", isOn: $terms)
                                .labelsHidden()
                                .disabled(state.busy)
                                .accessibilityIdentifier("acceptTerms")
                                .onChange(of: terms) { _, on in feedback.play(.toggle(on)) }
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 56)
                    }
                    .padding(.top, 14)
                }
                if let error = state.error {
                    FormBanner(text: error).padding(.horizontal, 16).padding(.top, 12).transition(.opacity)
                }
                if !otherServer {
                    Button {
                        otherServer = true
                        focus = .server
                    } label: {
                        Label { Text("Other server") } icon: { GlyphIcon(.server, size: 18) }
                            .labelStyle(SignalLabelStyle())
                            .font(RFont.sans(16, .semibold))
                            .foregroundStyle(Palette.ink2)
                            .frame(maxWidth: .infinity, minHeight: 46)
                    }
                    .buttonStyle(PressDimsStyle())
                    .disabled(state.busy)
                    .padding(.top, 8)
                    .accessibilityIdentifier("otherServer")
                }
            }
            .frame(maxWidth: 480, alignment: .leading)
            .frame(maxWidth: .infinity)
            .padding(.bottom, 24)
        }
        .scrollDismissesKeyboard(.interactively)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            ArriveFooter {
                Button(action: submit) {
                    HStack(spacing: 10) {
                        if state.busy { ProgressView().tint(Palette.onInverse) }
                        Text(creating ? "Create account" : "Sign in")
                    }
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(!canSubmit || state.busy)
                .accessibilityIdentifier(creating ? "create" : "signIn")
            }
        }
        .pageBackground()
        .animation(.smooth(duration: 0.25), value: state)
        .animation(.smooth(duration: 0.25), value: otherServer)
        .onAppear {
            if model.demo && server == SignInState.defaultServer { server = DemoData.server }
        }
    }

    private func field(
        _ title: String, glyph: Glyph, text: Binding<String>, field: Field, next: Field, content: UITextContentType, keyboard: UIKeyboardType,
        mono: Bool = false
    ) -> some View {
        GlyphInputRow(glyph: glyph) {
            TextField(title, text: text)
                .font(mono ? RFont.mono(15) : RFont.sans(16))
                .textContentType(content)
                .keyboardType(keyboard)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.next)
                .focused($focus, equals: field)
                .onSubmit { focus = next }
                .disabled(state.busy)
                .accessibilityIdentifier(title.lowercased())
        }
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

/// How strong the master password looks, as three short segments in the field's corner (the board's meter), each
/// filling with a detent tick. A hint only: the server's one rule is the length.
private struct StrengthMeter: View {
    var password: String
    @Environment(\.feedback) private var feedback

    private var strength: NewAccountRules.Strength { NewAccountRules.strength(password) }
    private var filled: Int {
        switch strength {
        case .weak: 1
        case .fair: 2
        case .strong: 3
        }
    }
    private var tint: Color {
        switch strength {
        case .weak: Palette.danger
        case .fair: Palette.warning
        case .strong: Palette.success
        }
    }

    var body: some View {
        HStack(spacing: 3) {
            ForEach(0..<3, id: \.self) { i in
                Capsule().fill(i < filled ? tint : Palette.rule2).frame(width: 16, height: 4)
            }
        }
        .animation(.easeOut(duration: 0.2), value: filled)
        .onChange(of: filled) { _, step in feedback.play(.detent, step: step) }
        .accessibilityElement()
        .accessibilityLabel("Strength: \(strength.rawValue)")
        .accessibilityValue(NewAccountRules.strengthHint(password))
        .accessibilityIdentifier("passwordStrength")
    }
}
