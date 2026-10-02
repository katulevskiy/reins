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
            InfoRow(title: "Recovery code", subtitle: "Opens your vault if you lose this phone", symbol: "key.viewfinder") {
                if busy { ProgressView() } else { Chevron() }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(busy)
        .accessibilityIdentifier("recoveryCodeRow")
        .sheet(item: Binding(get: { code.map(ShownCode.init) }, set: { if $0 == nil { code = nil } })) { shown in
            RecoveryCodeSheet(code: shown.code) { code = nil }
        }
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
            switch await OwnerCheck.confirm(model, reason: "Show your recovery code") {
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
                error = OwnerCheck.unavailableMessage
            }
        }
    }
}

struct RecoveryCodeSheet: View {
    var code: String
    var onDone: () -> Void
    @Environment(\.feedback) private var feedback
    @State private var copied = false

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

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Recovery code")
                .font(RFont.sans(26, .semibold))
                .foregroundStyle(Palette.text)
                .accessibilityAddTraits(.isHeader)
            Text("This code opens your account's vault if you lose this phone. Anyone with it and your sign-in can read your vault. Write it down and keep it somewhere safe; Reins cannot show it to you again if this phone is gone.")
                .font(RFont.sans(15))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Self.rows(code), id: \.self) { row in
                    Text(row).font(RFont.mono(19, .medium)).foregroundStyle(Palette.text)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(18)
            .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .environment(\.layoutDirection, .leftToRight)
            .textSelection(.enabled)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("recoveryCode")
            Text("To add another phone, sign in on it: this phone asks you to approve it, and no code is needed.")
                .font(RFont.sans(14))
                .foregroundStyle(Palette.tertiary)
                .fixedSize(horizontal: false, vertical: true)
            Button(copied ? "Copied" : "Copy") {
                // Kept out of other devices' clipboards and dropped after a minute.
                UIPasteboard.general.setItems([[UIPasteboard.typeAutomatic: code]], options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(60)])
                feedback.play(.copied)
                copied = true
            }
            .buttonStyle(CapsuleButtonStyle(kind: .secondary))
            .accessibilityIdentifier("copyRecoveryCode")
            Button("Done", action: onDone)
                .buttonStyle(CapsuleButtonStyle(kind: .primary))
                .accessibilityIdentifier("recoveryDone")
        }
        .padding(24)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .pageBackground()
    }
}
