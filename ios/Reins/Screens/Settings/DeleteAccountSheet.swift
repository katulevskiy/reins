import SwiftUI

/// Settings > Session > Delete account: what goes, that it cannot be undone, and the account's email typed to confirm.
/// The core deletes the account on the server (with its WorkOS user) and this phone's copy; the app then forgets it as
/// signing out does. A refusal (a typo, another phone approves, no network) shows here and changes nothing.
struct DeleteAccountSheet: View {
    var email: String
    var onClose: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var typed = ""
    @State private var busy = false
    @State private var error: String?
    @FocusState private var focused: Bool

    private var confirmed: Bool { SettingsText.deletionConfirmed(typed, email: email) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Delete account")
                    .font(RFont.sans(26, .semibold))
                    .foregroundStyle(Palette.text)
                    .accessibilityAddTraits(.isHeader)
                Text("Gone for good. Can't be undone.")
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(SettingsText.deletionItems, id: \.self) { item in
                        Label {
                            Text(item).font(RFont.sans(15)).foregroundStyle(Palette.text)
                        } icon: {
                            Image(systemName: "xmark.circle.fill").foregroundStyle(Palette.danger)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(18)
                .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                Text("Type \(email) to confirm.")
                    .font(RFont.sans(14.5))
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                TextField("Your email", text: $typed)
                    .textContentType(.emailAddress)
                    .keyboardType(.emailAddress)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .focused($focused)
                    .disabled(busy)
                    .fieldWell()
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityIdentifier("deleteAccountEmail")
                if let error {
                    FormBanner(text: error)
                }
                Button(role: .destructive, action: delete) {
                    HStack(spacing: 10) {
                        if busy { ProgressView().tint(Palette.background) }
                        Text("Delete account")
                    }
                }
                .buttonStyle(CapsuleButtonStyle(kind: .danger))
                .disabled(busy || !confirmed)
                .accessibilityIdentifier("confirmDeleteAccount")
                // The sheet's Close is the sound (the style plays the default tap).
                Button("Cancel", action: onClose)
                .buttonStyle(CapsuleButtonStyle(kind: .secondary))
                .disabled(busy)
                .accessibilityIdentifier("cancelDeleteAccount")
            }
            .padding(24)
        }
        .scrollBounceBehavior(.basedOnSize)
        .interactiveDismissDisabled(busy)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .pageBackground()
    }

    private func delete() {
        guard !busy, confirmed else { return }
        busy = true
        error = nil
        focused = false
        Task {
            do {
                try await model.deleteAccount(confirmEmail: typed)
                onClose()
            } catch {
                feedback.play(.error)
                self.error = error.userMessage
            }
            busy = false
        }
    }
}
