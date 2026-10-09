import SwiftUI

/// The passkeys that open the account's vault on a new phone: the list in Settings, removing one, and "Add a passkey"
/// (there, and offered before the recovery code after signing in). The Android app's `VaultPasskeysViewModel`.
@Observable
@MainActor
final class VaultPasskeysModel {
    /// Nil until first read.
    private(set) var passkeys: [VaultPasskeyView]?
    private(set) var busy = false
    var error: String?

    /// Reads the list (a network call); a failure says why and keeps the last one.
    func load(_ model: AppModel) async {
        do {
            passkeys = try await model.core.vaultPasskeys()
        } catch {
            self.error = error.userMessage
        }
    }

    /// "Add a passkey": the system's passkey sheet makes one, the core keeps the vault's copy for it. Closing the sheet
    /// changes nothing; a password manager without PRF says so and adds nothing. Once added, the offer before the
    /// recovery code is done too. True when one was added.
    @discardableResult
    func add(_ model: AppModel) async -> Bool {
        guard !busy else { return false }
        busy = true
        error = nil
        defer { busy = false }
        do {
            guard let list = try await VaultPasskeys.add(core: model.core, prompt: model.passkeys, name: VaultPasskeys.deviceName()) else {
                return false
            }
            passkeys = list
            model.feedback.play(.grantCreated)
            model.vaultPasskeyAdded()
            return true
        } catch {
            model.feedback.play(.error)
            self.error = error.userMessage
            return false
        }
    }

    /// The passkey stops opening the vault; it stays in the password manager until deleted there.
    func remove(_ credentialId: Data, _ model: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        model.feedback.play(.revoked)
        do {
            passkeys = try await model.core.removeVaultPasskey(credentialId: credentialId)
        } catch {
            model.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    /// A credential id as the accessibility identifiers name it (base64url, as the Android app's test tags).
    static func tag(_ credentialId: Data) -> String {
        credentialId.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

/// Settings > Vault passkeys: the passkeys that open the vault on a new phone, adding one, removing one.
struct VaultPasskeysScreen: View {
    @Environment(AppModel.self) private var model
    @State private var vm = VaultPasskeysModel()
    @State private var removing: VaultPasskeyView?

    var body: some View {
        List {
            Section {
                switch vm.passkeys {
                case nil:
                    InfoRow("Loading…").cardRow()
                case let list? where list.isEmpty:
                    InfoRow("No passkey yet", subtitle: "Add one to unlock your vault on a new phone.", symbol: "key")
                        .accessibilityIdentifier("noPasskeys")
                        .cardRow()
                case let list?:
                    ForEach(list, id: \.credentialId) { passkey in
                        PasskeyRow(passkey: passkey, busy: vm.busy) { removing = passkey }.cardRow()
                    }
                }
            } header: {
                GroupHeader("Passkeys")
            } footer: {
                GroupFooter("Each one opens your vault on a new or reinstalled phone, without your other phone or the recovery code.")
            }
            Section {
                VStack(spacing: 12) {
                    Button {
                        Task { await vm.add(model) }
                    } label: {
                        HStack(spacing: 10) {
                            if vm.busy { ProgressView().tint(Palette.background) } else { Image(systemName: "plus") }
                            Text("Add a passkey")
                        }
                    }
                    .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 50))
                    .disabled(vm.busy)
                    .accessibilityIdentifier("addPasskey")
                    if let error = vm.error {
                        FormBanner(text: error).accessibilityIdentifier("passkeyError")
                    }
                }
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            } footer: {
                GroupFooter("Your password manager keeps the passkey. Reins keeps only a copy of the vault's key that the passkey alone opens.")
            }
        }
        .reinsGrouped()
        .navigationTitle("Vault passkeys")
        .animation(.smooth(duration: 0.25), value: vm.passkeys)
        .animation(.smooth(duration: 0.25), value: vm.error)
        .task { await vm.load(model) }
        .confirmationDialog(
            "Remove this passkey?",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            titleVisibility: .visible,
            presenting: removing
        ) { passkey in
            Button("Remove", role: .destructive) {
                removing = nil
                Task { await vm.remove(passkey.credentialId, model) }
            }
        } message: { _ in
            Text("It no longer opens your vault. It stays in your password manager until you delete it there.")
        }
        .presentationFeedback(removing != nil)
    }
}

/// One passkey: its name, when it was added, and Remove.
private struct PasskeyRow: View {
    var passkey: VaultPasskeyView
    var busy: Bool
    var onRemove: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        let tag = VaultPasskeysModel.tag(passkey.credentialId)
        HStack(spacing: 14) {
            // Its own element, apart from Remove (an `InfoRow` would combine the two).
            InfoRow(untrusted(passkey.name), subtitle: SettingsText.passkeyAdded(passkey.createdAt), symbol: "key.fill", tint: Palette.accent)
                .accessibilityIdentifier("passkey:\(tag)")
            Button {
                onRemove()
                // The dialog it opens has the sound.
                feedback.defaultTap()
            } label: {
                Image(systemName: "trash")
                    .font(.system(size: 17, weight: .medium))
                    .foregroundStyle(Palette.danger)
                    .frame(width: 40, height: 40)
                    .contentShape(Circle())
            }
            .buttonStyle(.borderless)
            .disabled(busy)
            .accessibilityLabel("Remove \(untrusted(passkey.name))")
            .accessibilityIdentifier("removePasskey:\(tag)")
        }
    }
}
