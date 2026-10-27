import SwiftUI
import UIKit

/// After "Continue", for an account whose keys this phone cannot open yet: another phone has them (it approves this
/// one, comparing a code), or the recovery code does. This phone takes the approval role only once it can open them.
/// With neither, a third way out resets the vault: everything in it is deleted and the account starts over with new
/// keys and a new recovery code, confirmed by signing in again (`UnlockModel.reset`).
/// The first two ways (not the reset) let a phone take the approval role from another one when the server asks for a
/// proof (`SessionState.otherApprovalDevice`).
struct UnlockScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm = UnlockModel()
    @FocusState private var codeFocused: Bool

    // The code, the unlock form, the passkeys: hidden in the app switcher and while the screen is recorded.
    var body: some View { screen.privacyCover() }

    /// The Arrival board, 05: the account's pink field (the other phone, unlocked, beside this one, locked; "Another
    /// phone"; the account in mono), the passkey first when the account has one, then the other phone or the recovery
    /// code, and apart from them resetting the vault (a hold, after signing in again) and signing out.
    @ViewBuilder private var screen: some View {
        GeometryReader { geo in
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                field(top: geo.safeAreaInsets.top)
                switch vm.stage {
                case .choose: choose
                case let .waiting(code): waiting(code)
                case .recovery: recovery
                case .reset: resetting
                }
                if let error = vm.error {
                    FormBanner(text: error).padding(.horizontal, 16).padding(.top, 12).transition(.opacity).accessibilityIdentifier("unlockError")
                }
            }
            .padding(.bottom, 24)
        }
        .scrollDismissesKeyboard(.interactively)
        .scrollBounceBehavior(.basedOnSize)
        // The field runs into the status bar: the page starts at the top of the screen, the field's words below it.
        .scrollEdgeEffectHidden(true, for: .top)
        .ignoresSafeArea(edges: .top)
        }
        .pageBackground()
        .animation(.smooth(duration: 0.25), value: vm.stage)
        .animation(.smooth(duration: 0.25), value: vm.error)
        .animation(.smooth(duration: 0.25), value: vm.passkeyUnlock)
        .task { await vm.loadPasskeys(model) }
        .onDisappear { vm.stopWaiting() }
    }

    /// Signed in, but the server would not make this phone the approval device without a proof: the same two ways give
    /// it one.
    private var takeover: Bool {
        if case .otherApprovalDevice = model.session { true } else { false }
    }

    private func field(top: CGFloat) -> some View {
        SignalField(kind: .accounts) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    ArtPhone(glyph: .lockOpen, width: 58, height: 96, fill: .white.opacity(0.2), ink: .white)
                    ArtDots(count: 4)
                    ArtPhone(glyph: .lock, width: 58, height: 96)
                }
                Text(takeover ? "Another phone approves" : "Another phone")
                    .font(RFont.display(37))
                    .tracking(RFont.displayTracking(37))
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 22)
                    .accessibilityAddTraits(.isHeader)
                if let info = model.session.unlocking {
                    Text(verbatim: info.email)
                        .font(RFont.mono(14))
                        .foregroundStyle(.white.opacity(0.88))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .environment(\.layoutDirection, .leftToRight)
                        .padding(.top, 8)
                }
                if takeover {
                    // Why this phone is asked at all: the server keeps another phone for approvals.
                    Text(CoreError.OtherApprovalDevice.userMessage)
                        .font(RFont.sans(14.5))
                        .foregroundStyle(.white.opacity(0.88))
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 10)
                        .accessibilityIdentifier("unlockReason")
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, top + 12)
            .padding(.bottom, 26)
        }
    }

    private var choose: some View {
        VStack(alignment: .leading, spacing: 0) {
            if vm.passkeyUnlock {
                // A passkey added for the vault comes first: no other phone or code needed.
                Button {
                    Task { await vm.unlockWithPasskey(model) }
                } label: {
                    HStack(spacing: 8) {
                        if vm.passkeyBusy { ProgressView().tint(Palette.onInverse) } else { GlyphIcon(.faceId, size: 18) }
                        Text("Passkey")
                    }
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(vm.busy)
                .padding(.horizontal, 16)
                .padding(.top, 16)
                .accessibilityIdentifier("unlockWithPasskey")
            }
            RaisedGroup {
                Button {
                    feedback.play(.tap)
                    Task { await vm.ask(model) }
                } label: {
                    TileRow("Other phone", glyph: .phone) {
                        if vm.busy && !vm.passkeyBusy { ProgressView() }
                    }
                }
                .buttonStyle(RowPressStyle())
                .disabled(vm.busy)
                .accessibilityIdentifier("askOtherPhone")
                Button {
                    feedback.play(.tap)
                    vm.showRecovery()
                    codeFocused = true
                } label: {
                    TileRow("Recovery code", glyph: .fileLock)
                }
                .buttonStyle(RowPressStyle())
                .disabled(vm.busy)
                .accessibilityIdentifier("enterRecoveryCode")
            }
            .padding(.top, 12)
            RaisedGroup {
                if !takeover {
                    // Neither the other phone nor the recovery code: start over (explained, then held).
                    Button {
                        feedback.play(.tap)
                        vm.showReset()
                    } label: {
                        TileRow("Reset vault", glyph: .trash, danger: true, chevron: false)
                    }
                    .buttonStyle(RowPressStyle())
                    .disabled(vm.busy)
                    .accessibilityIdentifier("resetVault")
                }
                Button {
                    feedback.play(.tap)
                    vm.stopWaiting()
                    Task { await model.signOut() }
                } label: {
                    TileRow("Sign out", glyph: .quit, chevron: false)
                }
                .buttonStyle(RowPressStyle())
                .disabled(vm.busy)
                .accessibilityIdentifier("unlockSignOut")
            }
            .padding(.top, 22)
        }
        .transition(.opacity)
    }

    private func waiting(_ code: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Open Reins on your other phone. Approve this one only if it shows the same code.")
                .textRole(.body)
                .foregroundStyle(Palette.ink2)
                .fixedSize(horizontal: false, vertical: true)
            // Worked out from this phone's key: the other phone shows the same digits.
            Text(verbatim: code)
                .font(RFont.mono(48, .medium))
                .tracking(3)
                .monospacedDigit()
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
                .minimumScaleFactor(0.6)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 18)
                .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                .environment(\.layoutDirection, .leftToRight)
                .accessibilityLabel("Code \(code)")
                .accessibilityIdentifier("joinCode")
            HStack(spacing: 10) {
                ProgressView()
                Text("Waiting for your other phone").textRole(.meta).foregroundStyle(Palette.inkMeta)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 4)
            Button("Cancel") {
                feedback.play(.tap)
                Task { await vm.cancel(model) }
            }
            .buttonStyle(SignalButtonStyle(.outline))
            .accessibilityIdentifier("cancelJoin")
        }
        .padding(.horizontal, 16)
        .padding(.top, 18)
        .transition(.opacity)
    }

    private var recovery: some View {
        @Bindable var vm = vm
        return VStack(alignment: .leading, spacing: 0) {
            SectionHeader("Recovery code", quiet: true)
            RaisedFields {
                GlyphInputRow(glyph: .fileLock) {
                    TextField("Recovery code or master password", text: $vm.code, axis: .vertical)
                        .font(RFont.mono(16))
                        .textInputAutocapitalization(.characters)
                        .autocorrectionDisabled()
                        .textContentType(.oneTimeCode)
                        .lineLimit(1...4)
                        .submitLabel(.go)
                        .focused($codeFocused)
                        .onSubmit { Task { await vm.unlock(model) } }
                        .disabled(vm.busy)
                        .accessibilityIdentifier("recoveryCode")
                }
            }
            GroupFooter("The recovery code is in thirteen groups of four letters and digits. An account made with a master password opens with that password instead.")
                .padding(.horizontal, 20)
            Button {
                codeFocused = false
                Task { await vm.unlock(model) }
            } label: {
                HStack(spacing: 10) {
                    if vm.busy { ProgressView().tint(Palette.onInverse) }
                    Text("Open the vault")
                }
            }
            .buttonStyle(SignalButtonStyle(.inverse))
            .disabled(vm.busy || vm.code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .padding(.horizontal, 16)
            .padding(.top, 8)
            .accessibilityIdentifier("unlock")
            backLink("unlockBack")
        }
        .transition(.opacity)
    }

    /// What a reset deletes and what it keeps; signing in again confirms it, held.
    private var resetting: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Deletes the vault: items, integrations, grants and activity. Other phones sign out; AIs connect again. Can't be undone.")
                .textRole(.body)
                .foregroundStyle(Palette.danger)
                .fixedSize(horizontal: false, vertical: true)
            Text("Your email and sign-in stay. You get a new recovery code. To confirm, sign in again with this account.")
                .textRole(.body)
                .foregroundStyle(Palette.ink2)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 6)
            // A red hold, never a tap: everything in the vault goes.
            HoldButton(title: "Hold to sign in again and reset", busy: vm.busy) {
                Task { await vm.reset(model) }
            }
            .accessibilityIdentifier("confirmReset")
            backLink("resetBack").padding(.horizontal, -16)
        }
        .padding(.horizontal, 16)
        .padding(.top, 18)
        .transition(.opacity)
    }

    private func backLink(_ id: String) -> some View {
        Button {
            feedback.play(.tap)
            vm.back()
        } label: {
            Text("Back")
                .font(RFont.sans(16, .semibold))
                .foregroundStyle(Palette.ink2)
                .frame(maxWidth: .infinity, minHeight: 46)
        }
        .buttonStyle(PressDimsStyle())
        .disabled(vm.busy)
        .padding(.horizontal, 16)
        .accessibilityIdentifier(id)
    }
}

/// The Unlock screen's state: asking the other phone (and polling its answer), the recovery code, or resetting the
/// vault.
@Observable
@MainActor
final class UnlockModel {
    enum Stage: Equatable {
        case choose
        /// Asked; the code both phones show.
        case waiting(String)
        case recovery
        /// What a reset deletes, and the sign-in that confirms it.
        case reset
    }

    private(set) var stage = Stage.choose
    private(set) var busy = false
    /// What `busy` waits for is the passkey (its button shows it).
    private(set) var passkeyBusy = false
    /// The account has a passkey for its vault: "Unlock with passkey" comes first. Unknown (offline) counts as none.
    private(set) var passkeyUnlock = false
    var error: String?
    var code = ""
    /// How long between two looks at the other phone's answer.
    var pollInterval: Duration = .seconds(2)
    private var polling: Task<Void, Never>?
    /// The web sheet for the sign-in that confirms a reset; tests put a fake in its place. It does not reuse the
    /// browser's last sign-in, so the person really signs in again.
    var browse: (URL, String) async throws -> URL = { url, scheme in
        try await WebAuth.run(url, callbackScheme: scheme, ephemeral: true)
    }

    /// The name the other phone shows ("Add iPhone?").
    static var deviceName: String { UIDevice.current.name }

    func ask(_ model: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        do {
            let start = try await model.core.joinBegin(deviceName: Self.deviceName)
            busy = false
            stage = .waiting(start.code)
            startPolling(model)
        } catch {
            busy = false
            model.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    private func startPolling(_ model: AppModel) {
        polling?.cancel()
        polling = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                try? await Task.sleep(for: self.pollInterval)
                if Task.isCancelled { return }
                if await self.pollOnce(model) { return }
            }
        }
    }

    /// One look at the answer; true when the request is over.
    @discardableResult
    func pollOnce(_ model: AppModel) async -> Bool {
        do {
            switch try await model.core.joinPoll() {
            case .waiting:
                return false
            case .joined:
                model.feedback.play(.connected)
                stage = .choose
                await model.finishUnlock()
            case .denied:
                model.feedback.play(.denied)
                stage = .choose
                error = "Your other phone did not approve this one."
            case .expired:
                stage = .choose
                error = "The request expired. Ask again."
            }
        } catch CoreError.NotFound {
            stage = .choose
        } catch {
            // Offline for a moment: keep asking.
            return false
        }
        return true
    }

    func cancel(_ model: AppModel) async {
        stopWaiting()
        try? await model.core.joinCancel()
        stage = .choose
    }

    func stopWaiting() {
        polling?.cancel()
        polling = nil
    }

    func showRecovery() {
        error = nil
        stage = .recovery
    }

    func back() {
        error = nil
        code = ""
        stage = .choose
    }

    func unlock(_ model: AppModel) async {
        let entered = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !busy, !entered.isEmpty else { return }
        busy = true
        error = nil
        do {
            try await model.core.unlockAccount(codeOrPassword: entered)
            code = ""
            busy = false
            model.feedback.play(.connected)
            await model.finishUnlock()
        } catch {
            busy = false
            model.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    func loadPasskeys(_ model: AppModel) async {
        passkeyUnlock = !((try? await model.core.vaultPasskeys()) ?? []).isEmpty
    }

    /// "Unlock with passkey": one of the account's passkeys opens the copy of the vault's key kept for it, and the
    /// sign-in finishes as with the recovery code. Closing the sheet changes nothing.
    func unlockWithPasskey(_ model: AppModel) async {
        guard !busy else { return }
        busy = true
        passkeyBusy = true
        error = nil
        do {
            let opened = try await VaultPasskeys.unlock(core: model.core, prompt: model.passkeys)
            busy = false
            passkeyBusy = false
            guard opened else { return }
            model.feedback.play(.connected)
            await model.finishUnlock()
        } catch {
            busy = false
            passkeyBusy = false
            model.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    func showReset() {
        error = nil
        stage = .reset
    }

    /// "Sign in again and reset": a fresh sign-in to the same account (in a browser session that does not reuse the last
    /// one) confirms it; the core deletes the vault and makes the account's keys anew (`resetAccount`). What follows is a
    /// new account's: `finishSso` signs in, and the new recovery code must be recorded. A closed page is no error; only
    /// a locked account (not a takeover) can be reset.
    func reset(_ model: AppModel) async {
        guard !busy, case let .keysLocked(info) = model.session else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            let start = try await model.core.ssoBegin(serverUrl: info.serverUrl)
            guard let url = URL(string: start.url) else { throw WebAuth.Failure.failed("The server's sign-in address is not valid.") }
            let callback = model.demo
                ? URL(string: "\(start.callbackScheme)://sso-callback?code=demo&state=\(start.state)")!
                : try await browse(url, start.callbackScheme)
            let outcome = try await model.core.resetAccount(
                serverUrl: info.serverUrl, callbackUrl: callback.absoluteString, state: start.state, verifier: start.verifier
            )
            model.feedback.play(.connected)
            stage = .choose
            await model.finishSso(outcome)
        } catch WebAuth.Failure.cancelled {
            // The person closed the page: the reset stays offered, nothing changed.
        } catch let WebAuth.Failure.failed(message) {
            model.feedback.play(.error)
            error = message
        } catch WebAuth.Failure.noWindow {
            error = "Open Reins and try again."
        } catch {
            model.feedback.play(.error)
            self.error = error.userMessage
        }
    }
}
