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
            NotificationsGroup()
            NavigationGroups()
            HelpGroup()
            #if DEBUG
            DeveloperGroup()
            #endif
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
                if model.recoveryCodeAvailable {
                    RecoveryCodeRow()
                    VaultPasskeysRow()
                }
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

/// Settings > Account > Vault passkeys: how many open the vault, read again each time Settings shows.
private struct VaultPasskeysRow: View {
    @Environment(AppModel.self) private var model
    @State private var count: Int?

    var body: some View {
        SettingsLinkRow(
            title: "Vault passkeys", subtitle: SettingsText.passkeysSummary(count), symbol: "person.badge.key", tint: Palette.accent,
            id: "vaultPasskeysRow"
        ) { model.show(.vaultPasskeys) }
        .task { count = try? await model.core.vaultPasskeys().count }
    }
}

/// Whether this phone receives the approval requests, and the button that makes it the one that does.
private struct ApprovalDeviceGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false
    @State private var message: String?
    @State private var error: String?
    /// Whether the signed-in server can wake the app; nil while unknown or when it does not say.
    @State private var serverPush: Bool?

    var body: some View {
        Section {
            button
                .cardRow()
                .task {
                    guard case let .signedIn(info) = model.session else { return }
                    serverPush = (try? await model.core.serverInfo(serverUrl: info.serverUrl))?.pushIos
                }
            if let message {
                FormBanner(text: message, kind: .info).cardRow()
            }
            if let error = error ?? (model.approvalDevice ? nil : model.registrationError) {
                FormBanner(text: error).cardRow()
            }
        } header: {
            GroupHeader("Approval device")
        } footer: {
            GroupFooter(SettingsText.approvalFooter(approvalDevice: model.approvalDevice, appPush: model.pushToken != nil, serverPush: serverPush))
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
    @Environment(\.feedback) private var feedback

    /// The switches as the Sounds page keeps them (in memory, observed), so the summary follows them.
    private var sounds: FeedbackSettings { ((feedback as? FeedbackPreviewing)?.store ?? .shared).settings }

    var body: some View {
        Section {
            SettingsLinkRow(
                // The vault comes with the account: it is not something the user connected.
                title: "Integrations", subtitle: SettingsText.accountsLine(model.accounts.filter { $0.service != "vault" }.count), symbol: "square.grid.2x2",
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
    }
}

/// Whether requests can ring this phone: "On", or what is wrong and a tap to turn them on (the system prompt while it
/// can still show, else this app's page in the Settings app).
private struct NotificationsGroup: View {
    @Environment(\.openURL) private var openURL
    @Environment(\.scenePhase) private var scenePhase
    private var access: NotificationAccess { .shared }

    var body: some View {
        let state = access.state
        Section {
            SettingsLinkRow(
                title: "Approval notifications", subtitle: state.summary, symbol: state.needsAttention ? "bell.slash" : "bell",
                tint: state.needsAttention ? Palette.warning : Palette.success, id: "notificationsRow"
            ) {
                Task { await access.turnOn(openURL: openURL) }
            }
        } header: {
            GroupHeader("Notifications")
        }
        .task { await access.refresh() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { Task { await access.refresh() } }
        }
    }
}

/// "Take the tour": the setup pages again (how Reins works, integrations, the private model).
private struct HelpGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Section {
            SettingsLinkRow(
                title: "Take the tour", subtitle: nil, symbol: "questionmark.circle",
                tint: Palette.accent, id: "takeTour"
            ) { model.startTour() }
        } header: {
            GroupHeader("Help")
        }
    }
}

/// The AI connections, each opening its own page.
private struct ConnectionsGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        // Computers pinned their key when they paired; Claude.ai, ChatGPT and other AI apps have none.
        let computers = model.connections.filter { $0.keyFingerprint != nil }
        let aiApps = model.connections.filter { $0.keyFingerprint == nil }
        Section {
            ForEach(computers, id: \.id) { connectionRow($0) }
            SettingsLinkRow(
                title: "Connect a computer", subtitle: nil, symbol: "qrcode.viewfinder",
                tint: Palette.pair, id: "connectComputer"
            ) { model.openSheet(.connectComputer) }
        } header: {
            GroupHeader("Computers")
        }
        Section {
            ForEach(aiApps, id: \.id) { connectionRow($0) }
            // Always there, as "Connect a computer" is above: the address to paste, not a description of it, so a
            // second AI app needs no typing. Tapping copies it.
            let address = model.mcpAddress
            Button {
                UIPasteboard.general.string = address
                feedback.play(.copied)
                model.notice = "MCP address copied."
            } label: {
                InfoRow(
                    title: aiApps.isEmpty ? "No AI app is connected yet" : "Connect another AI app",
                    subtitle: "In Claude.ai or ChatGPT, add a custom connector with \(address). Tap to copy.",
                    symbol: "link", tint: Palette.accent, ltrSubtitle: false
                ) {
                    Image(systemName: "doc.on.doc").foregroundStyle(Palette.tertiary).accessibilityHidden(true)
                }
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("copyMcpUrl")
            .cardRow()
        } header: {
            GroupHeader("AI apps")
        }
    }

    private func connectionRow(_ connection: ConnectionView) -> some View {
        Button {
            model.show(.connection(connection.id))
            feedback.defaultTap()
        } label: {
            ConnectionRow(connection: connection)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("connection:\(connection.id)")
        .cardRow()
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
                Text("Requests can't reach this phone until you sign in again.")
            }
            .presentationFeedback(confirm)
            if case let .signedIn(info) = model.session {
                Button(role: .destructive) {
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
                .presentationFeedback(deleting != nil)
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
    var subtitle: String?
    var symbol: String
    var tint: Color
    var id: String
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    init(title: String, subtitle: String?, symbol: String, tint: Color, id: String, action: @escaping () -> Void) {
        self.title = title
        self.subtitle = subtitle
        self.symbol = symbol
        self.tint = tint
        self.id = id
        self.action = action
    }

    var body: some View {
        Button {
            action()
            // The page or sheet it opens has the sound.
            feedback.defaultTap()
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

#if DEBUG
/// Debug builds only: what the header's Autopilot pill opens, to compare the designs on a phone.
private struct DeveloperGroup: View {
    @AppStorage(QuickStyle.key) private var style = QuickStyle.menu.rawValue

    var body: some View {
        Section {
            Picker("Autopilot pill", selection: $style) {
                ForEach(QuickStyle.allCases) { Text($0.label).tag($0.rawValue) }
            }
            .pickerStyle(.segmented)
            .cardRow()
            .accessibilityIdentifier("quickStyle")
        } header: {
            GroupHeader("Developer")
        }
    }
}
#endif
