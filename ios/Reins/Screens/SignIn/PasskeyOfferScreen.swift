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
            Image(systemName: "person.badge.key.fill")
                .font(.system(size: 26, weight: .semibold))
                .foregroundStyle(Palette.accent)
                .frame(width: 56, height: 56)
                .background(Palette.accentSoft, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .accessibilityHidden(true)
                .padding(.bottom, 6)
            Text("Protect your vault with a passkey")
                .font(RFont.sans(28, .semibold))
                .foregroundStyle(Palette.text)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            Text("Unlocks your vault on a new phone. Your password manager keeps it.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 10)
            if let error = vm.error {
                FormBanner(text: error).transition(.opacity).accessibilityIdentifier("passkeyError")
            }
            Button {
                Task { await vm.add(model) }
            } label: {
                HStack(spacing: 10) {
                    if vm.busy { ProgressView().tint(Palette.background) } else { Image(systemName: "key.fill") }
                    Text("Add a passkey")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary))
            .disabled(vm.busy)
            .accessibilityIdentifier("addPasskey")
            Button("Use the recovery code only") { model.declinePasskeyOffer() }
                .buttonStyle(CapsuleButtonStyle(kind: .secondary))
                .disabled(vm.busy)
                .accessibilityIdentifier("skipPasskey")
            Text("A recovery code comes next.")
                .font(RFont.sans(13))
                .foregroundStyle(Palette.tertiary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.leading, 4)
        }
    }
}
