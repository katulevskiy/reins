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

/// Settings > Passkeys (the Settings board, 05), quiet: each passkey that opens the vault on a new phone as a device
/// card (its name, when it was added, Remove), then Add a passkey.
struct VaultPasskeysScreen: View {
    @Environment(AppModel.self) private var model
    @State private var vm = VaultPasskeysModel()
    @State private var removing: VaultPasskeyView?

    // The passkeys: hidden in the app switcher and while the screen is recorded.
    var body: some View { screen.privacyCover() }

    @ViewBuilder private var screen: some View {
        QuietPage("Passkeys") {
            switch vm.passkeys {
            case nil:
                ProgressView().frame(maxWidth: .infinity).padding(.top, 40)
            case let list? where list.isEmpty:
                FlatGroup {
                    TileRow("No passkey yet", glyph: .faceId, chevron: false).flatRow()
                }
                .padding(.top, 16)
                .accessibilityIdentifier("noPasskeys")
            case let list?:
                VStack(spacing: 12) {
                    ForEach(list, id: \.credentialId) { passkey in
                        PasskeyCard(passkey: passkey, busy: vm.busy) { removing = passkey }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.top, 16)
            }
            Button {
                Task { await vm.add(model) }
            } label: {
                HStack(spacing: 8) {
                    if vm.busy { ProgressView() } else { GlyphIcon(.plus, size: 18) }
                    Text("Add a passkey")
                }
            }
            .buttonStyle(SignalButtonStyle(.soft))
            .disabled(vm.busy)
            .padding(.horizontal, 20)
            .padding(.top, 16)
            .accessibilityIdentifier("addPasskey")
            if let error = vm.error {
                QuietBanner(text: error).accessibilityIdentifier("passkeyError")
            }
            GroupFooter("Each one opens your vault on a new or reinstalled phone, without your other phone or the recovery code. "
                + "Your password manager keeps the passkey; Reins keeps only a copy of the vault's key that the passkey alone opens.")
                .padding(.horizontal, 20)
                .padding(.top, 4)
        }
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
            Text("It stops opening your vault.")
        }
        .presentationFeedback(removing != nil)
    }
}

/// One passkey as the quiet page's focal card: an ink tile with the phone, its name in the display face, "Passkey",
/// Remove, and when it was added.
private struct PasskeyCard: View {
    var passkey: VaultPasskeyView
    var busy: Bool
    var onRemove: () -> Void

    var body: some View {
        let tag = VaultPasskeysModel.tag(passkey.credentialId)
        let name = untrustedName(passkey.name, limit: 40)
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 14) {
                // Its own element, apart from Remove.
                HStack(spacing: 14) {
                    ZStack {
                        Palette.inverse
                        Grain()
                        GlyphIcon(.phone, size: 24).foregroundStyle(Palette.onInverse)
                    }
                    .frame(width: 52, height: 52)
                    .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(name)
                            .font(RFont.display(25.5))
                            .foregroundStyle(Palette.ink)
                            .lineLimit(1)
                            .minimumScaleFactor(0.7)
                        Text("Passkey").font(RFont.sans(13.5)).foregroundStyle(Palette.inkMeta)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("passkey:\(tag)")
                RowIconButton(glyph: .trash, label: "Remove \(name)", action: onRemove)
                    .disabled(busy)
                    .padding(.trailing, -8)
                    .accessibilityIdentifier("removePasskey:\(tag)")
            }
            .padding(.bottom, 14)
            HStack(spacing: 12) {
                GlyphIcon(.calendar, size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20).accessibilityHidden(true)
                Text(SettingsText.passkeyAdded(passkey.createdAt)).font(RFont.sans(15)).foregroundStyle(Palette.ink).lineLimit(1)
            }
            .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
            .overlay(alignment: .top) { Rectangle().fill(Palette.rule).frame(height: 1) }
        }
        .padding(.top, 18)
        .padding(.horizontal, 16)
        .padding(.bottom, 6)
        .overlay(RoundedRectangle(cornerRadius: 24, style: .continuous).strokeBorder(Palette.rule2, lineWidth: 1))
    }
}
