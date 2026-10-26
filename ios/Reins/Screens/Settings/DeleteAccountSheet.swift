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
                // Your own irreversible act gets the same red field an AI's would.
                SignalField(kind: .delete) {
                    VStack(alignment: .leading, spacing: 0) {
                        RoundHeaderButton(glyph: .close, label: "Close", onField: true, size: 34, action: onClose)
                            .disabled(busy)
                            .accessibilityIdentifier("closeDeleteAccount")
                        Text("Account · Delete").textRole(.section).foregroundStyle(Palette.onField2).padding(.top, 22)
                        AskText([AskText.Run(text: "Delete "), AskText.Run(text: email, target: true)], role: .ask, lineLimit: nil)
                            .foregroundStyle(Palette.onField)
                            .padding(.top, 8)
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel("Delete \(email)")
                            .accessibilityAddTraits(.isHeader)
                        Text("Gone for good. Can't be undone.")
                            .font(RFont.sans(14.5))
                            .foregroundStyle(Palette.onField2)
                            .padding(.top, 8)
                    }
                    .padding(.horizontal, 22)
                    .padding(.top, 18)
                    .padding(.bottom, 22)
                }
                .padding(.horizontal, -24)
                .padding(.top, -24)
                // Exactly what goes.
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(SettingsText.deletionItems, id: \.self) { item in
                        HStack(alignment: .top, spacing: 12) {
                            GlyphIcon(.close, size: 15).foregroundStyle(Palette.danger).padding(.top, 2)
                            Text(item).font(RFont.sans(15)).foregroundStyle(Palette.ink).fixedSize(horizontal: false, vertical: true)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.vertical, 12)
                        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
                    }
                }
                Text("Type your email to confirm")
                    .textRole(.meta)
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.top, 8)
                TextField("Your email", text: $typed)
                    .textContentType(.emailAddress)
                    .keyboardType(.emailAddress)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .focused($focused)
                    .disabled(busy)
                    .fieldWell()
                    .overlay(alignment: .trailing) {
                        if confirmed { GlyphIcon(.check, size: 16).foregroundStyle(Palette.success).padding(.trailing, 14) }
                    }
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityIdentifier("deleteAccountEmail")
                if let error {
                    FormBanner(text: error)
                }
                // Held (1.2 s), once the email is typed.
                HoldButton(title: "Hold to delete", symbol: "trash", enabled: confirmed, busy: busy, onConfirm: delete)
                    .accessibilityIdentifier("confirmDeleteAccount")
                // The sheet's Close is the sound (the style plays the default tap).
                Button("Cancel", action: onClose)
                    .font(RFont.sans(15, .medium))
                    .foregroundStyle(Palette.ink2)
                    .frame(maxWidth: .infinity, minHeight: 44)
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
