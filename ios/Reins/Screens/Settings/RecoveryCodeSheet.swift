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

    /// The Arrival board, 04: the account's pink field with the code's file and "Recovery code", one line; the groups
    /// numbered in reading order on raised paper (left to right, then down) so it is clear how to copy them out; Copy
    /// (cleared from the clipboard in 60 s); "I wrote it down" before Continue.
    @ViewBuilder private var screen: some View {
        let groups = code.split(separator: "-").map(String.init)
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                SignalField(kind: .accounts) {
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(spacing: 12) {
                            ArtNode(glyph: .fileLock, size: 44, fill: .white.opacity(0.2), ink: .white, shadow: false)
                            Text("Recovery code")
                                .font(RFont.display(30))
                                .tracking(RFont.displayTracking(30))
                                .lineLimit(1)
                                .minimumScaleFactor(0.7)
                                .accessibilityAddTraits(.isHeader)
                        }
                        Text("The only way back in if you lose this phone.")
                            .font(RFont.sans(15.5))
                            .foregroundStyle(.white.opacity(0.88))
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.horizontal, 20)
                    .padding(.top, 26)
                    .padding(.bottom, 24)
                }
                RaisedGroup {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(minimum: 0), spacing: 8), count: 3), spacing: 13) {
                        ForEach(Array(groups.enumerated()), id: \.offset) { i, group in
                            HStack(spacing: 6) {
                                Text("\(i + 1)")
                                    .font(RFont.mono(11))
                                    .foregroundStyle(Palette.inkFaint)
                                    .frame(width: 16, alignment: .trailing)
                                Text(verbatim: group)
                                    .font(RFont.mono(19, .medium))
                                    .foregroundStyle(Palette.ink)
                                    .lineLimit(1)
                                    .minimumScaleFactor(0.7)
                            }
                            .frame(minHeight: 24)
                        }
                    }
                    .padding(.vertical, 20)
                    .padding(.horizontal, 12)
                    .environment(\.layoutDirection, .leftToRight)
                    .textSelection(.enabled)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Recovery code: " + groups.joined(separator: ", "))
                    .accessibilityAction(named: "Copy recovery code", copy)
                    .accessibilityIdentifier("recoveryCode")
                }
                .padding(.top, 18)
                Button(action: copy) {
                    Label { Text(copied ? "Copied · clears in 60 s" : "Copy") } icon: { GlyphIcon(copied ? .check : .copy, size: 18) }
                }
                .buttonStyle(SignalButtonStyle(.soft, size: .small))
                .padding(.horizontal, 16)
                .padding(.top, 12)
                .accessibilityHint("Cleared from the clipboard in 60 seconds")
                .accessibilityIdentifier("copyRecoveryCode")
                if required {
                    RaisedGroup {
                        HStack(spacing: 12) {
                            GlyphTile(.pencil)
                            Toggle(isOn: $recorded) {
                                Text("I wrote it down").textRole(.row).foregroundStyle(Palette.ink)
                            }
                            .toggleStyle(SignalSwitchStyle())
                            .accessibilityIdentifier("recoveryRecorded")
                            .onChange(of: recorded) { _, on in feedback.play(.toggle(on)) }
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 56)
                    }
                    .padding(.top, 14)
                }
            }
            .padding(.bottom, 24)
        }
        .scrollBounceBehavior(.basedOnSize)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            ArriveFooter {
                Button(required ? "Continue" : "Done", action: onDone)
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .disabled(required && !recorded)
                    .accessibilityIdentifier("recoveryDone")
            }
        }
        .interactiveDismissDisabled(required)
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .pageBackground()
    }
}
