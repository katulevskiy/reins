import SwiftUI

/// A web passkey the member just added in the browser console, waiting for this phone (O3): its fingerprint, the same
/// code the console shows, built by the core from the key hash the confirmation signs. Confirm vouches for it with the
/// enrolled key; Not me refuses (revokes) it.
struct WebPasskeyCard: View {
    let key: OrgPasskeyView
    var busy = false
    var enabled = true
    let answer: (Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "person.badge.key")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(Palette.violet)
                    .accessibilityHidden(true)
                Text("New web passkey")
                    .font(RFont.sans(16.5, .semibold))
                    .foregroundStyle(Palette.ink)
            }
            Text(key.fingerprint)
                .font(RFont.mono(22, .medium))
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
                .accessibilityIdentifier("orgPasskeyFingerprint:\(key.credentialId)")
            Text("Matches your browser?")
                .font(RFont.sans(13.5))
                .foregroundStyle(Palette.inkMeta)
                .lineLimit(1)
            HStack(spacing: 10) {
                Button("Not me") { answer(false) }
                    .buttonStyle(SignalButtonStyle(.danger, size: .small))
                    .disabled(!enabled)
                    .accessibilityIdentifier("orgPasskeyRefuse:\(key.credentialId)")
                Button("Confirm") { answer(true) }
                    .buttonStyle(SignalButtonStyle(.inverse, size: .small))
                    .disabled(!enabled || busy)
                    .accessibilityIdentifier("orgPasskeyConfirm:\(key.credentialId)")
            }
            .padding(.top, 4)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(Palette.paperRaised))
        .accessibilityIdentifier("orgPasskey:\(key.credentialId)")
    }
}
