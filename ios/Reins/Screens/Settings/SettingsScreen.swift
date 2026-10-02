import SwiftUI
import UIKit

/// The Settings section: the account, whether this phone approves requests, Autopilot, the AI connections, the
/// integrations, sounds and haptics, the version, and signing out.
struct SettingsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false
    @State private var message: String?
    @State private var error: String?
    @State private var confirmSignOut = false
    @State private var sounds = FeedbackSettings.load()

    var body: some View {
        List {
            Section {
                PageHeader(title: "Settings") { EmptyView() }
                    .listRowBackground(Color.clear)
                    .listRowInsets(EdgeInsets(top: 0, leading: 4, bottom: 0, trailing: 4))
            }

            if case let .signedIn(info) = model.session {
                Section {
                    InfoRow("Email", subtitle: info.email, symbol: "envelope", ltrSubtitle: true).cardRow()
                    Button {
                        UIPasteboard.general.string = info.serverUrl
                        feedback.play(.copied)
                        model.notice = "Server address copied."
                    } label: {
                        InfoRow(title: "Server", subtitle: info.serverUrl, symbol: "link", ltrSubtitle: true) {
                            Image(systemName: "doc.on.doc")
                                .font(.system(size: 15))
                                .foregroundStyle(Palette.tertiary)
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityHint("Copies the server address")
                    .accessibilityIdentifier("copyServer")
                    .cardRow()
                } header: {
                    GroupHeader("Account")
                }
            }

            Section {
                approvalButton.cardRow()
                if let message {
                    Banner(text: message, kind: .info).cardRow()
                }
                if let error = error ?? (model.approvalDevice ? nil : model.registrationError) {
                    Banner(text: error).cardRow()
                }
            } header: {
                GroupHeader("Approval device")
            } footer: {
                GroupFooter(approvalFooter)
            }

            Section {
                let mode = model.autopilot?.mode ?? .manual
                row("Autopilot", subtitle: SettingsText.autopilotSummary(model.autopilot), symbol: SettingsText.modeSymbol(mode),
                    tint: SettingsText.modeTint(mode), id: "openAutopilot") {
                    model.show(.autopilot)
                }
            } header: {
                GroupHeader("Autopilot")
            }

            Section {
                if model.connections.isEmpty {
                    InfoRow("No AI is connected yet", subtitle: "Add Reins to Claude or ChatGPT with your server's /mcp address.").cardRow()
                }
                ForEach(model.connections, id: \.id) { connection in
                    Button {
                        feedback.play(.tap)
                        model.show(.connection(connection.id))
                    } label: {
                        HStack(spacing: 14) {
                            ConnectionAvatar(label: untrusted(connection.label), pick: connection.icon, size: 40)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(untrusted(connection.label))
                                    .font(RFont.sans(16, .medium))
                                    .foregroundStyle(Palette.text)
                                    .lineLimit(1)
                                Text(SettingsText.connectionLine(connection))
                                    .font(RFont.sans(13))
                                    .foregroundStyle(Palette.secondary)
                                    .lineLimit(1)
                                    .environment(\.layoutDirection, .leftToRight)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            Chevron()
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("connection:\(connection.id)")
                    .cardRow()
                }
            } header: {
                GroupHeader("AI connections")
            }

            Section {
                row("Integrations", subtitle: SettingsText.accountsLine(model.accounts.count), symbol: "square.grid.2x2",
                    tint: Palette.read, id: "openIntegrations") {
                    model.show(.integrations)
                }
            } header: {
                GroupHeader("Integrations")
            }

            Section {
                row("Sounds & haptics", subtitle: SettingsText.soundsSummary(sounds), symbol: "speaker.wave.2",
                    tint: Palette.secondary, id: "openSounds") {
                    model.show(.sounds)
                }
            } header: {
                GroupHeader("Sounds & haptics")
            }

            Section {
                InfoRow("Version", subtitle: SettingsText.version(), symbol: "info.circle", ltrSubtitle: true)
                    .accessibilityIdentifier("appVersion")
                    .cardRow()
            } header: {
                GroupHeader("Version")
            } footer: {
                GroupFooter("The App Store keeps Reins up to date.")
            }

            Section {
                Button(role: .destructive) {
                    confirmSignOut = true
                } label: {
                    Label("Sign out", systemImage: "rectangle.portrait.and.arrow.right")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .danger, height: 50))
                .disabled(busy)
                .accessibilityIdentifier("signOut")
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            } header: {
                GroupHeader("Session")
            }
        }
        .reinsGrouped()
        .navigationTitle("Settings")
        .toolbar(.hidden, for: .navigationBar)
        .refreshable {
            await model.refreshConnections()
            await model.refreshPending()
            feedback.play(.refresh)
        }
        .task { await model.refreshConnections() }
        .onAppear { sounds = FeedbackSettings.load() }
        .onChange(of: model.path(.settings).isEmpty) { sounds = FeedbackSettings.load() }
        .confirmationDialog("Sign out?", isPresented: $confirmSignOut, titleVisibility: .visible) {
            Button("Sign out", role: .destructive) { signOut() }
        } message: {
            Text("This phone stops receiving approval requests until you sign in again.")
        }
    }

    // MARK: Approval device

    @ViewBuilder private var approvalButton: some View {
        if model.approvalDevice {
            Label("This phone is used for approvals", systemImage: "checkmark")
                .font(RFont.sans(16, .semibold))
                .foregroundStyle(Palette.secondary)
                .frame(maxWidth: .infinity, minHeight: 50)
                .background(Palette.controlFill, in: Capsule())
                .padding(.vertical, 6)
                .accessibilityAddTraits(.isButton)
                .accessibilityRemoveTraits(.isStaticText)
                .accessibilityValue("Unavailable")
                .accessibilityIdentifier("registerPhone")
        } else {
            Button(action: registerThisPhone) {
                HStack(spacing: 8) {
                    if busy { ProgressView() } else { Image(systemName: "iphone") }
                    Text("Use this phone for approvals")
                }
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 50))
            .disabled(busy)
            .padding(.vertical, 6)
            .accessibilityIdentifier("registerPhone")
        }
    }

    private var approvalFooter: String {
        if model.approvalDevice {
            return model.pushToken != nil
                ? "Requests reach this phone by push notification and while the app is open."
                : "Push notifications are not set up yet. Requests arrive while the app is open."
        }
        return "Only one phone at a time approves requests. Use this one to take over."
    }

    private func registerThisPhone() {
        guard !busy else { return }
        busy = true
        message = nil
        error = nil
        feedback.play(.tap)
        Task {
            do {
                try await model.registerDevice(force: true)
                model.registrationError = nil
                message = "This phone is now your approval device."
            } catch {
                feedback.play(.error)
                self.error = error.userMessage
            }
            busy = false
        }
    }

    private func signOut() {
        busy = true
        Task {
            await model.signOut()
            busy = false
        }
    }

    // MARK: Rows

    private func row(_ title: String, subtitle: String, symbol: String, tint: Color, id: String, action: @escaping () -> Void) -> some View {
        Button {
            feedback.play(.tap)
            action()
        } label: {
            InfoRow(title: title, subtitle: subtitle, symbol: symbol, tint: tint) { Chevron() }
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(id)
        .cardRow()
    }
}

/// The disclosure mark at the end of a row that opens a page.
struct Chevron: View {
    var body: some View {
        Image(systemName: "chevron.right")
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(Palette.tertiary)
            .accessibilityHidden(true)
    }
}
