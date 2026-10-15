import SwiftUI
import UIKit

/// Settings > Account > Recovery code, for an account made without a master password: the code that opens its vault
/// when no phone that keeps the account's secret is left. Shown after Face ID, Touch ID or the passcode.
struct RecoveryCodeRow: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var code: String?
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        Button(action: open) {
            SettingsRowLabel(title: "Recovery code", chevron: !busy) {
                SettingsTile(symbol: "key.fill", tint: Palette.kindPush)
            } trailing: {
                if busy { ProgressView() }
            }
        }
        .buttonStyle(.plain)
        .disabled(busy)
        .accessibilityLabel("Recovery code")
        .accessibilityHint("Opens your vault if you lose this phone")
        .accessibilityIdentifier("recoveryCodeRow")
        .sheet(item: Binding(get: { code.map(ShownCode.init) }, set: { if $0 == nil { code = nil } })) { shown in
            RecoveryCodeSheet(code: shown.code) { code = nil }
        }
        .presentationFeedback(code != nil)
        .cardRow()
        if let error {
            FormBanner(text: error).cardRow()
        }
    }

    private struct ShownCode: Identifiable {
        var code: String
        var id: String { "recovery" }
    }

    private func open() {
        guard !busy else { return }
        busy = true
        error = nil
        feedback.play(.tap)
        Task {
            defer { busy = false }
            switch await OwnerPrompt.confirm(model, reason: "Show your recovery code") {
            case .confirmed:
                do {
                    code = try await model.core.accountRecoveryCode()
                } catch {
                    feedback.play(.error)
                    self.error = error.userMessage
                }
            case .cancelled:
                break
            case .unavailable:
                error = OwnerPrompt.unavailableMessage
            }
        }
    }
}

struct RecoveryCodeSheet: View {
    var code: String
    var required = false
    var onDone: () -> Void
    @Environment(\.feedback) private var feedback
    @State private var copied = false
    @State private var recorded = false

    private func copy() {
        UIPasteboard.general.setItems([[UIPasteboard.typeAutomatic: code]], options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(60)])
        feedback.play(.copied)
        copied = true
    }

    /// The thirteen groups, in rows of three (the last row has four).
    static func rows(_ code: String) -> [String] {
        let groups = code.split(separator: "-").map(String.init)
        var rows: [String] = []
        var i = 0
        while i < groups.count {
            let n = groups.count - i == 4 ? 4 : 3
            rows.append(groups[i..<min(i + n, groups.count)].joined(separator: " "))
            i += n
        }
        return rows
    }

    // The code, the unlock form, the passkeys: hidden in the app switcher and while the screen is recorded.
    var body: some View { screen.privacyCover() }

    @ViewBuilder private var screen: some View {
        ScrollView {
        VStack(alignment: .leading, spacing: 16) {
            // An account field: pink, because it changes how the account opens.
            SignalField(kind: .accounts) {
                VStack(alignment: .leading, spacing: 0) {
                    Text("Account · Recovery").textRole(.label).foregroundStyle(Palette.onField2)
                    AskText([AskText.Run(text: "Write down your "), AskText.Run(text: "recovery code", target: true)], role: .ask, lineLimit: nil)
                        .foregroundStyle(Palette.onField)
                        .padding(.top, 8)
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel("Write down your recovery code")
                        .accessibilityAddTraits(.isHeader)
                    Text("It opens your vault if you lose this phone. Nobody else has it, us included.")
                        .font(RFont.sans(14.5))
                        .foregroundStyle(Palette.onField2)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 10)
                }
                .padding(.horizontal, 22)
                .padding(.top, 26)
                .padding(.bottom, 24)
            }
            .padding(.horizontal, -24)
            .padding(.top, -24)
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 3), spacing: 10) {
                ForEach(Array(code.split(separator: "-").enumerated()), id: \.offset) { _, group in
                    Text(String(group)).font(RFont.mono(18, .medium)).foregroundStyle(Palette.ink)
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 20)
            .padding(.horizontal, 12)
            .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
            .environment(\.layoutDirection, .leftToRight)
            .textSelection(.enabled)
            .contentShape(Rectangle())
            .onTapGesture(perform: copy)
            .accessibilityAction(named: "Copy recovery code", copy)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("recoveryCode")
            Button {
                copy()
            } label: {
                Label(copied ? "Copied · clears in 60 s" : "Copy · clears in 60 s", systemImage: "doc.on.doc")
            }
            .buttonStyle(SignalButtonStyle(.soft, size: .small))
            .accessibilityIdentifier("copyRecoveryCode")
            if required {
                Toggle(isOn: $recorded) {
                    Text("I wrote it down").font(RFont.sans(15)).foregroundStyle(Palette.ink)
                }
                .toggleStyle(SignalSwitchStyle())
                .accessibilityIdentifier("recoveryRecorded")
                .onChange(of: recorded) { _, on in feedback.play(.toggle(on)) }
            }
            Button(required ? "Continue" : "Done", action: onDone)
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .frame(height: 54)
                .disabled(required && !recorded)
                .accessibilityIdentifier("recoveryDone")
        }
        .padding(24)
        }
        .scrollBounceBehavior(.basedOnSize)
        .interactiveDismissDisabled(required)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .pageBackground()
    }
}
