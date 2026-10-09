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
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.feedback) private var feedback
    @State private var vm = UnlockModel()
    @FocusState private var codeFocused: Bool

    var body: some View {
        GeometryReader { geo in
            ScrollView {
                VStack(spacing: 0) {
                    Spacer(minLength: sizeClass == .regular ? 40 : 24)
                    content
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

    @ViewBuilder private var content: some View {
        VStack(alignment: .leading, spacing: 12) {
            Image(systemName: "iphone.gen3")
                .font(.system(size: 26, weight: .semibold))
                .foregroundStyle(Palette.accent)
                .frame(width: 56, height: 56)
                .background(Palette.accentSoft, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .accessibilityHidden(true)
                .padding(.bottom, 6)
            Text(takeover ? "Another phone approves for this account" : "Your account is on another phone")
                .font(RFont.sans(28, .semibold))
                .foregroundStyle(Palette.text)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            if let info = model.session.unlocking {
                Text(info.email)
                    .font(RFont.mono(14))
                    .foregroundStyle(Palette.secondary)
                    .environment(\.layoutDirection, .leftToRight)
            }
            switch vm.stage {
            case .choose: choose
            case let .waiting(code): waiting(code)
            case .recovery: recovery
            case .reset: resetting
            }
            if let error = vm.error {
                FormBanner(text: error).transition(.opacity).accessibilityIdentifier("unlockError")
            }
            Button("Sign out") {
                feedback.play(.tap)
                vm.stopWaiting()
                Task { await model.signOut() }
            }
            .font(RFont.sans(14.5, .medium))
            .foregroundStyle(Palette.secondary)
            .frame(maxWidth: .infinity)
            .padding(.top, 10)
            .disabled(vm.busy)
            .accessibilityIdentifier("unlockSignOut")
        }
    }

    private var reason: String {
        if takeover { return CoreError.OtherApprovalDevice.userMessage }
        return vm.passkeyUnlock
            ? "This account's vault is encrypted with keys that only your other phone, your passkeys and your recovery code can open. Unlock with your passkey, ask that phone, or enter the code."
            : "This account's vault is encrypted with keys that only your other phone and your recovery code can open. Ask that phone, or enter the code."
    }

    private var choose: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(reason)
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 10)
                .accessibilityIdentifier("unlockReason")
            if vm.passkeyUnlock {
                // A passkey added for the vault comes first: no other phone or code needed.
                Button {
                    Task { await vm.unlockWithPasskey(model) }
                } label: {
                    HStack(spacing: 10) {
                        if vm.passkeyBusy { ProgressView().tint(Palette.background) } else { Image(systemName: "person.badge.key.fill") }
                        Text("Unlock with passkey")
                    }
                }
                .buttonStyle(CapsuleButtonStyle(kind: .primary))
                .disabled(vm.busy)
                .accessibilityIdentifier("unlockWithPasskey")
            }
            Button {
                feedback.play(.tap)
                Task { await vm.ask(model) }
            } label: {
                HStack(spacing: 10) {
                    if vm.busy && !vm.passkeyBusy { ProgressView().tint(vm.passkeyUnlock ? Palette.text : Palette.background) }
                    Text("Ask my other phone")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: vm.passkeyUnlock ? .secondary : .primary))
            .disabled(vm.busy)
            .accessibilityIdentifier("askOtherPhone")
            Button("Enter recovery code") {
                feedback.play(.tap)
                vm.showRecovery()
                codeFocused = true
            }
            .buttonStyle(CapsuleButtonStyle(kind: .secondary))
            .disabled(vm.busy)
            .accessibilityIdentifier("enterRecoveryCode")
            if !takeover {
                // Neither the other phone nor the recovery code: start over.
                Button("Lost both? Reset the vault") {
                    feedback.play(.tap)
                    vm.showReset()
                }
                .font(RFont.sans(14.5, .medium))
                .foregroundStyle(Palette.secondary)
                .frame(maxWidth: .infinity)
                .padding(.top, 4)
                .disabled(vm.busy)
                .accessibilityIdentifier("resetVault")
            }
        }
        .transition(.opacity)
    }

    private func waiting(_ code: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Open Reins on your other phone and approve this one. Approve only if it shows this code:")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text(code)
                .font(RFont.mono(40, .semibold))
                .foregroundStyle(Palette.text)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 18)
                .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .environment(\.layoutDirection, .leftToRight)
                .accessibilityLabel("Code \(code)")
                .accessibilityIdentifier("joinCode")
            HStack(spacing: 10) {
                ProgressView()
                Text("Waiting for your other phone...")
                    .font(RFont.sans(14.5))
                    .foregroundStyle(Palette.secondary)
            }
            .padding(.vertical, 4)
            Button("Cancel") {
                feedback.play(.tap)
                Task { await vm.cancel(model) }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .secondary))
            .accessibilityIdentifier("cancelJoin")
        }
        .transition(.opacity)
    }

    private var recovery: some View {
        @Bindable var vm = vm
        return VStack(alignment: .leading, spacing: 12) {
            Text("The recovery code is in thirteen groups of four letters and digits. An account made with a master password opens with that password instead.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
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
                .fieldWell()
                .accessibilityIdentifier("recoveryCode")
            Button {
                codeFocused = false
                Task { await vm.unlock(model) }
            } label: {
                HStack(spacing: 10) {
                    if vm.busy { ProgressView().tint(Palette.background) }
                    Text("Open the vault")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary))
            .disabled(vm.busy || vm.code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .accessibilityIdentifier("unlock")
            Button("Back") {
                feedback.play(.tap)
                vm.back()
            }
            .font(RFont.sans(14.5, .medium))
            .foregroundStyle(Palette.secondary)
            .frame(maxWidth: .infinity)
            .disabled(vm.busy)
            .accessibilityIdentifier("unlockBack")
        }
        .transition(.opacity)
    }

    /// What a reset deletes and what it keeps; signing in again confirms it.
    private var resetting: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Resetting deletes everything in this account's vault: saved items, integrations and their grants, and the activity and settings kept for it. Any other phone signed in to it is signed out, and connected AIs must be connected again. This cannot be undone.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text("The account itself stays: its email and its sign-in. You get a new recovery code to write down.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text("To confirm, sign in again with this account.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 10)
            Button {
                feedback.play(.tap)
                Task { await vm.reset(model) }
            } label: {
                HStack(spacing: 10) {
                    if vm.busy { ProgressView().tint(.white) }
                    Text("Sign in again and reset")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .danger))
            .disabled(vm.busy)
            .accessibilityIdentifier("confirmReset")
            Button("Back") {
                feedback.play(.tap)
                vm.back()
            }
            .font(RFont.sans(14.5, .medium))
            .foregroundStyle(Palette.secondary)
            .frame(maxWidth: .infinity)
            .disabled(vm.busy)
            .accessibilityIdentifier("resetBack")
        }
        .transition(.opacity)
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
