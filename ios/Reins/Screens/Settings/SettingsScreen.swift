import SwiftUI
import UIKit

/// The Settings section: the account, whether this phone approves requests, Autopilot, the AI connections, the
/// integrations, sounds and haptics, the version, signing out and deleting the account.
///
/// Each group is its own small view: one `List` body holding all of them miscompiles with Xcode 27.1 beta (the app
/// crashes destroying the list value), and small views also redraw only what changed.
struct SettingsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        List {
            Section {
                PageHeader(title: "Settings") { EmptyView() }
                    .listRowBackground(Color.clear)
                    .listRowInsets(EdgeInsets(top: 0, leading: 4, bottom: 0, trailing: 4))
            }
            AccountGroup()
            ApprovalDeviceGroup()
            SettingsAutopilotGroup()
            ConnectionsGroup()
            NavigationGroups()
            VersionGroup()
            SessionGroup()
        }
        .reinsGrouped()
        .navigationTitle("Settings")
        .rootNavigationBar()
        .refreshable {
            await model.refreshConnections()
            await model.refreshPending()
            feedback.play(.refresh)
        }
        .task { await model.refreshConnections() }
    }
}

/// The signed-in email and the server, which copies on a tap.
private struct AccountGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
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
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityHint("Copies the server address")
                .accessibilityIdentifier("copyServer")
                .cardRow()
                if model.recoveryCodeAvailable { RecoveryCodeRow() }
            } header: {
                GroupHeader("Account")
            } footer: {
                if model.recoveryCodeAvailable {
                    GroupFooter("To add another phone, sign in on it; this phone asks you to approve it.")
                }
            }
        }
    }
}

/// Whether this phone receives the approval requests, and the button that makes it the one that does.
private struct ApprovalDeviceGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false
    @State private var message: String?
    @State private var error: String?

    var body: some View {
        Section {
            button.cardRow()
            if let message {
                FormBanner(text: message, kind: .info).cardRow()
            }
            if let error = error ?? (model.approvalDevice ? nil : model.registrationError) {
                FormBanner(text: error).cardRow()
            }
        } header: {
            GroupHeader("Approval device")
        } footer: {
            GroupFooter(footer)
        }
    }

    @ViewBuilder private var button: some View {
        if model.approvalDevice {
            Label("This phone is used for approvals", systemImage: "checkmark")
                .font(RFont.sans(16, .semibold))
                .foregroundStyle(Palette.secondary)
                .frame(maxWidth: .infinity, minHeight: 50)
                .background(Palette.controlFill, in: Capsule())
                .padding(.vertical, 6)
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isButton)
                .accessibilityValue("Unavailable")
                .accessibilityIdentifier("registerPhone")
        } else {
            Button(action: register) {
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

    private var footer: String {
        if model.approvalDevice {
            return model.pushToken != nil
                ? "Requests reach this phone by push notification and while the app is open."
                : "Push notifications are not set up yet. Requests arrive while the app is open."
        }
        return "Only one phone at a time approves requests. Use this one to take over."
    }

    private func register() {
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
}

/// Autopilot's mode and model, opening its page.
private struct SettingsAutopilotGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let mode = model.autopilot?.mode ?? .manual
        Section {
            SettingsLinkRow(
                title: "Autopilot", subtitle: SettingsText.autopilotSummary(model.autopilot), symbol: SettingsText.modeSymbol(mode),
                tint: SettingsText.modeTint(mode), id: "openAutopilot"
            ) { model.show(.autopilot) }
        } header: {
            GroupHeader("Autopilot")
        }
    }
}

/// Integrations and Sounds & haptics: one row each, opening their pages.
private struct NavigationGroups: View {
    @Environment(AppModel.self) private var model
    @State private var sounds = FeedbackSettings.load()

    var body: some View {
        Section {
            SettingsLinkRow(
                title: "Integrations", subtitle: SettingsText.accountsLine(model.accounts.count), symbol: "square.grid.2x2",
                tint: Palette.read, id: "openIntegrations"
            ) { model.show(.integrations) }
        } header: {
            GroupHeader("Integrations")
        }
        Section {
            SettingsLinkRow(
                title: "Sounds & haptics", subtitle: SettingsText.soundsSummary(sounds), symbol: "speaker.wave.2",
                tint: Palette.secondary, id: "openSounds"
            ) { model.show(.sounds) }
        } header: {
            GroupHeader("Sounds & haptics")
        }
        // Back from the Sounds page: read the switches again.
        .onChange(of: model.path(.settings).isEmpty) { sounds = FeedbackSettings.load() }
        .onAppear { sounds = FeedbackSettings.load() }
    }
}

/// The AI connections, each opening its own page.
private struct ConnectionsGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        Section {
            if model.connections.isEmpty {
                InfoRow("No AI is connected yet", subtitle: "Add Reins to Claude or ChatGPT with your server's /mcp address.").cardRow()
            }
            ForEach(model.connections, id: \.id) { connection in
                Button {
                    feedback.play(.tap)
                    model.show(.connection(connection.id))
                } label: {
                    ConnectionRow(connection: connection)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("connection:\(connection.id)")
                .cardRow()
            }
            SettingsLinkRow(
                title: "Connect a computer", subtitle: "Scan the QR code from reins login or the desktop app", symbol: "qrcode.viewfinder",
                tint: Palette.pair, id: "connectComputer"
            ) { model.openSheet(.connectComputer) }
        } header: {
            GroupHeader("AI connections")
        }
    }
}

private struct ConnectionRow: View {
    var connection: ConnectionView

    var body: some View {
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
        .accessibilityElement(children: .combine)
    }
}

/// The app's version; updates come from the App Store or TestFlight, never from the app itself.
private struct VersionGroup: View {
    var body: some View {
        Section {
            InfoRow("Version", subtitle: SettingsText.version(), symbol: "info.circle", ltrSubtitle: true)
                .accessibilityIdentifier("appVersion")
                .cardRow()
        } header: {
            GroupHeader("Version")
        } footer: {
            GroupFooter("The App Store keeps Reins up to date.")
        }
    }
}

/// Signing out, after a confirmation, and deleting the account, after its own sheet.
private struct SessionGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var confirm = false
    @State private var busy = false
    @State private var deleting: AccountToDelete?

    private struct AccountToDelete: Identifiable {
        var email: String
        var id: String { email }
    }

    var body: some View {
        Section {
            Button(role: .destructive) {
                confirm = true
            } label: {
                Label("Sign out", systemImage: "rectangle.portrait.and.arrow.right")
            }
            .buttonStyle(CapsuleButtonStyle(kind: .danger, height: 50))
            .disabled(busy)
            .accessibilityIdentifier("signOut")
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets())
            .confirmationDialog("Sign out?", isPresented: $confirm, titleVisibility: .visible) {
                Button("Sign out", role: .destructive) {
                    busy = true
                    Task {
                        await model.signOut()
                        busy = false
                    }
                }
            } message: {
                Text("This phone stops receiving approval requests until you sign in again.")
            }
            if case let .signedIn(info) = model.session {
                Button(role: .destructive) {
                    feedback.play(.tap)
                    deleting = AccountToDelete(email: info.email)
                } label: {
                    Label("Delete account", systemImage: "trash")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .danger, height: 50))
                .disabled(busy)
                .accessibilityIdentifier("deleteAccount")
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
                .sheet(item: $deleting) { shown in
                    DeleteAccountSheet(email: shown.email) { deleting = nil }
                }
            }
        } header: {
            GroupHeader("Session")
        } footer: {
            if case .signedIn = model.session {
                GroupFooter("Deleting your account removes it and everything in it from the server for good.")
            }
        }
    }
}

/// A row that opens a page: a tinted symbol, a title, a line under it, and the disclosure mark.
struct SettingsLinkRow: View {
    var title: String
    var subtitle: String
    var symbol: String
    var tint: Color
    var id: String
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    init(title: String, subtitle: String, symbol: String, tint: Color, id: String, action: @escaping () -> Void) {
        self.title = title
        self.subtitle = subtitle
        self.symbol = symbol
        self.tint = tint
        self.id = id
        self.action = action
    }

    var body: some View {
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
