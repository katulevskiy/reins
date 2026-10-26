import SwiftUI

/// Settings > Delete account (the Settings board, 06): your own irreversible act gets the same red field an AI's would.
/// What goes, the account's email typed to confirm, then a 1.2 s hold. The core deletes the account on the server (with
/// its WorkOS user) and this phone's copy; the app then forgets it as signing out does. A refusal (a typo, another
/// phone approves, no network) shows here and changes nothing.
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

    /// What goes, each with its glyph.
    private static let glyphs: [Glyph] = [.vault, .link, .shield, .mail, .phone]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                field
                SectionHeader("Goes for good", quiet: true)
                FlatGroup {
                    ForEach(Array(SettingsText.deletionItems.enumerated()), id: \.offset) { i, item in
                        TileRow(item, glyph: Self.glyphs[i % Self.glyphs.count], chevron: false).flatRow()
                    }
                }
                SectionHeader("Type your email", quiet: true)
                FlatGroup(inset: 32) {
                    InputRow(glyph: .mail) {
                        TextField("Your email", text: $typed)
                            .textContentType(.emailAddress)
                            .keyboardType(.emailAddress)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .submitLabel(.done)
                            .focused($focused)
                            .disabled(busy)
                            .accessibilityIdentifier("deleteAccountEmail")
                    } trailing: {
                        if confirmed {
                            GlyphIcon(.check, size: 18).foregroundStyle(Palette.success).accessibilityLabel("Matches")
                        }
                    }
                    .environment(\.layoutDirection, .leftToRight)
                }
                if let error {
                    QuietBanner(text: error)
                }
                // Held (1.2 s), once the email is typed.
                HoldButton(title: "Hold to delete", symbol: "trash", enabled: confirmed, busy: busy, onConfirm: delete)
                    .padding(.horizontal, 20)
                    .padding(.top, 18)
                    .accessibilityIdentifier("confirmDeleteAccount")
            }
            .padding(.bottom, 40)
        }
        .scrollBounceBehavior(.basedOnSize)
        .interactiveDismissDisabled(busy)
        .presentationDetents([.large])
        .presentationDragIndicator(.hidden)
        .pageBackground()
        #if DEBUG
        .onAppear {
            // `-fillEmail` (screenshots): the email already typed.
            if ProcessInfo.processInfo.arguments.contains("-fillEmail") { typed = email }
        }
        #endif
    }

    private var field: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Spacer()
                RoundHeaderButton(glyph: .close, label: "Close", onField: true, action: onClose)
                    .disabled(busy)
                    .accessibilityIdentifier("closeDeleteAccount")
            }
            AskText([AskText.Run(text: "Delete "), AskText.Run(text: email, target: true)], role: .title, lineLimit: nil)
                .foregroundStyle(Palette.onField)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("Delete \(email)")
                .accessibilityAddTraits(.isHeader)
            Text("Can’t be undone.")
                .font(RFont.sans(15))
                .foregroundStyle(Color.white.opacity(0.85))
                .padding(.top, 8)
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .padding(.bottom, 22)
        .frame(maxWidth: .infinity, alignment: .leading)
        .foregroundStyle(Palette.onField)
        .background {
            FieldSurface(kind: .delete)
                .clipShape(UnevenRoundedRectangle(topLeadingRadius: 32, bottomLeadingRadius: 30, bottomTrailingRadius: 30, topTrailingRadius: 32, style: .continuous))
                .ignoresSafeArea(edges: .top)
        }
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
