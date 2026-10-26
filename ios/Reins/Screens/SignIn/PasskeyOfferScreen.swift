import SwiftUI

/// Before the recovery code, for an account without a passkey for its vault: one that opens the vault on a new or
/// reinstalled phone. Offered strongly, but "Use the recovery code only" goes on without one, so a password manager
/// without PRF never blocks anyone. The recovery code still follows either way (the Android app's
/// `PasskeyOfferScreen`).
struct PasskeyOfferScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var vm = VaultPasskeysModel()

    /// The Arrival board, 03: a full field of the account's pink, one illustrated chain (a key, this phone, unlocked),
    /// "Passkey" and one line; Add passkey or Skip at the bottom. Skip still leads to the recovery code.
    var body: some View {
        ZStack {
            FieldSurface(kind: .accounts).ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    HStack(spacing: 10) {
                        ArtNode(glyph: .key, size: 72).rotationEffect(.degrees(-14))
                        ArtDots()
                        ArtPhone(glyph: .faceId, width: 100, height: 170, shadow: true)
                        ArtDots()
                        ArtNode(glyph: .lockOpen, size: 72, ink: Color(red: 21 / 255, green: 128 / 255, blue: 61 / 255))
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.top, sizeClass == .regular ? 80 : 140)
                    Text("Passkey")
                        .font(RFont.display(53))
                        .tracking(RFont.displayTracking(53))
                        .foregroundStyle(.white)
                        .padding(.top, 64)
                        .accessibilityAddTraits(.isHeader)
                    Text("Your face unlocks the vault.")
                        .font(RFont.sans(17))
                        .foregroundStyle(.white.opacity(0.88))
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 12)
                    if let error = vm.error {
                        FormBanner(text: error)
                            .background(.white, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            .padding(.top, 16)
                            .transition(.opacity)
                            .accessibilityIdentifier("passkeyError")
                    }
                }
                .frame(maxWidth: 440, alignment: .leading)
                .padding(.horizontal, 24)
                .frame(maxWidth: .infinity)
            }
            .scrollBounceBehavior(.basedOnSize)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 8) {
                Button {
                    Task { await vm.add(model) }
                } label: {
                    HStack(spacing: 8) {
                        if vm.busy { ProgressView().tint(Palette.onFieldInk) } else { GlyphIcon(.faceId, size: 18) }
                        Text("Add passkey")
                    }
                }
                .buttonStyle(SignalButtonStyle(.onField))
                .disabled(vm.busy)
                .accessibilityIdentifier("addPasskey")
                Button("Skip") { model.declinePasskeyOffer() }
                    .buttonStyle(SignalButtonStyle(.onFieldWash))
                    .disabled(vm.busy)
                    .accessibilityHint("A recovery code comes next")
                    .accessibilityIdentifier("skipPasskey")
            }
            .frame(maxWidth: 440)
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
        }
        .animation(.smooth(duration: 0.25), value: vm.error)
        .accessibilityIdentifier("passkeyOffer")
    }
}
