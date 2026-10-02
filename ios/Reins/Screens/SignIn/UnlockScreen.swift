import SwiftUI
import UIKit

/// After "Continue", for an account whose keys this phone cannot open yet: another phone has them (it approves this
/// one, comparing a code), or the recovery code does. This phone takes the approval role only once it can open them.
/// The same two ways let a phone take the approval role from another one when the server asks for a proof
/// (`SessionState.otherApprovalDevice`).
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

    private var choose: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(takeover
                ? CoreError.OtherApprovalDevice.userMessage
                : "This account's vault is encrypted with keys that only your other phone and your recovery code can open. Ask that phone, or enter the code.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 10)
                .accessibilityIdentifier("unlockReason")
            Button {
                feedback.play(.tap)
                Task { await vm.ask(model) }
            } label: {
                HStack(spacing: 10) {
                    if vm.busy { ProgressView().tint(Palette.background) }
                    Text("Ask my other phone")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary))
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
}

/// The Unlock screen's state: asking the other phone (and polling its answer), or the recovery code.
@Observable
@MainActor
final class UnlockModel {
    enum Stage: Equatable {
        case choose
        /// Asked; the code both phones show.
        case waiting(String)
        case recovery
    }

    private(set) var stage = Stage.choose
    private(set) var busy = false
    var error: String?
    var code = ""
    /// How long between two looks at the other phone's answer.
    var pollInterval: Duration = .seconds(2)
    private var polling: Task<Void, Never>?

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
}
