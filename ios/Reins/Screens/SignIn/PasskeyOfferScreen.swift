import SwiftUI

/// Before the recovery code, for an account without a passkey for its vault: one that opens the vault on a new or
/// reinstalled phone. Offered strongly, but "Use the recovery code only" goes on without one, so a password manager
/// without PRF never blocks anyone. The recovery code still follows either way (the Android app's
/// `PasskeyOfferScreen`).
struct PasskeyOfferScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var vm = VaultPasskeysModel()

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
            .scrollBounceBehavior(.basedOnSize)
        }
        .pageBackground()
        .animation(.smooth(duration: 0.25), value: vm.error)
        .accessibilityIdentifier("passkeyOffer")
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: 12) {
            // On paper, not a field: nothing waits on an AI here. The pink tile says it is an account matter.
            GlyphIcon(.key, size: 26)
                .foregroundStyle(Palette.onField)
                .frame(width: 56, height: 56)
                .fieldBackground(.accounts, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .accessibilityHidden(true)
                .padding(.bottom, 10)
            AskText([AskText.Run(text: "Protect your vault with a "), AskText.Run(text: "passkey", target: true)], role: .ask, lineLimit: nil)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
            Text("It unlocks your vault on a new phone, without your other phone or a recovery code. Your password manager keeps it.")
                .textRole(.body)
                .foregroundStyle(Palette.ink2)
                .fixedSize(horizontal: false, vertical: true)
            VStack(spacing: 0) {
                fact(.circleCheck, "Works with iCloud Keychain and password managers")
                fact(.lock, "Reins keeps only a sealed copy of the key")
            }
            .padding(.vertical, 8)
            if let error = vm.error {
                FormBanner(text: error).transition(.opacity).accessibilityIdentifier("passkeyError")
            }
            Button {
                Task { await vm.add(model) }
            } label: {
                HStack(spacing: 10) {
                    if vm.busy { ProgressView().tint(Palette.onInverse) }
                    Text("Add a passkey")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary))
            .disabled(vm.busy)
            .accessibilityIdentifier("addPasskey")
            Button("Use the recovery code only") { model.declinePasskeyOffer() }
                .buttonStyle(SignalButtonStyle(.outline))
                .disabled(vm.busy)
                .accessibilityIdentifier("skipPasskey")
            Text("A recovery code comes next.")
                .textRole(.meta)
                .foregroundStyle(Palette.inkMeta)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity)
        }
    }

    /// One thing worth knowing about the passkey, on a hairline row.
    private func fact(_ glyph: Glyph, _ text: String) -> some View {
        HStack(spacing: 12) {
            GlyphIcon(glyph, size: 18).foregroundStyle(Palette.ink2)
            Text(text).textRole(.row).foregroundStyle(Palette.ink).fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(.vertical, 12)
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
        .accessibilityElement(children: .combine)
    }
}
