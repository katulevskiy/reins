import SwiftUI
import UIKit

/// The Settings section: the account card (email, server, whether this phone approves and is notified, with the one
/// fix when something is off), what is connected, Autopilot and the other pages, security, the tour and version, and
/// leaving.
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
            AccountCardSection()
            ConnectedGroup()
            PagesGroup()
            SecurityGroup()
            AboutGroup()
            SessionGroup()
            FooterSection()
            #if DEBUG
            DeveloperGroup()
            #endif
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

// MARK: Account

/// The account card, and under it what "Use this phone for approvals" answered.
private struct AccountCardSection: View {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @State private var busy = false
    @State private var message: String?
    @State private var error: String?
    /// Whether the signed-in server can wake the app; nil while unknown or when it does not say.
    @State private var serverPush: Bool?

    var body: some View {
        if case let .signedIn(info) = model.session {
            Section {
                AccountCard(email: info.email, serverUrl: info.serverUrl, serverPush: serverPush, busy: $busy, message: $message, error: $error)
                    .listRowBackground(Color.clear)
                    .listRowInsets(EdgeInsets())
                if let message {
                    FormBanner(text: message, kind: .info).cardRow()
                }
                if let error = error ?? (model.approvalDevice ? nil : model.registrationError) {
                    FormBanner(text: error).cardRow()
                }
            }
            .task {
                await NotificationAccess.shared.refresh()
                serverPush = (try? await model.core.serverInfo(serverUrl: info.serverUrl))?.pushIos
            }
            .onChange(of: scenePhase) { _, phase in
                if phase == .active { Task { await NotificationAccess.shared.refresh() } }
            }
        }
    }
}

/// Who is signed in and to which server (a tap copies it), then whether this phone approves and is notified: green
/// chips when all is well, or the one thing to do about it.
private struct AccountCard: View {
    var email: String
    var serverUrl: String
    var serverPush: Bool?
    @Binding var busy: Bool
    @Binding var message: String?
    @Binding var error: String?
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.openURL) private var openURL
    private var access: NotificationAccess { .shared }

    private var push: SettingsText.PushChip {
        SettingsText.pushChip(
            approvalDevice: model.approvalDevice, notificationsOn: !access.state.needsAttention,
            appPush: model.pushToken != nil, serverPush: serverPush
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 14) {
                BlobAvatar(seed: email, size: 52)
                VStack(alignment: .leading, spacing: 3) {
                    Text(email)
                        .font(RFont.sans(17, .semibold))
                        .foregroundStyle(Palette.text)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .environment(\.layoutDirection, .leftToRight)
                    Button(action: copyServer) {
                        HStack(spacing: 6) {
                            Text(SettingsText.host(serverUrl))
                                .font(RFont.mono(13))
                                .foregroundStyle(Palette.secondary)
                                .lineLimit(1)
                            Image(systemName: "doc.on.doc")
                                .font(.system(size: 12))
                                .foregroundStyle(Palette.tertiary)
                        }
                        .environment(\.layoutDirection, .leftToRight)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Server, \(SettingsText.host(serverUrl))")
                    .accessibilityHint("Copies the server address")
                    .accessibilityIdentifier("copyServer")
                }
            }
            HStack(spacing: 8) {
                if model.approvalDevice {
                    StatusChip(text: "Approval phone", symbol: "iphone", tone: .ok).accessibilityIdentifier("approvalChip")
                } else {
                    StatusChip(text: "Another phone approves", symbol: "iphone", tone: .warn).accessibilityIdentifier("approvalChip")
                }
                switch push {
                case .on: StatusChip(text: "Notifications on", symbol: "bell", tone: .ok).accessibilityIdentifier("notificationsChip")
                case .onlyWhileOpen: StatusChip(text: "Only while open", symbol: "bell.slash", tone: .plain).accessibilityIdentifier("notificationsChip")
                case .hidden: EmptyView()
                }
            }
            .padding(.top, 16)
            if access.state.needsAttention {
                attention.padding(.top, 14)
            }
            if !model.approvalDevice {
                Button(action: register) {
                    HStack(spacing: 8) {
                        if busy { ProgressView().tint(Palette.background) } else { Image(systemName: "iphone") }
                        Text("Use this phone for approvals")
                    }
                }
                .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 50))
                .disabled(busy)
                .padding(.top, 14)
                .accessibilityIdentifier("registerPhone")
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            LinearGradient(colors: [Palette.accent.opacity(0.16), Palette.accent.opacity(0.03)], startPoint: .topLeading, endPoint: .bottomTrailing),
            in: RoundedRectangle(cornerRadius: 26, style: .continuous)
        )
        .overlay(RoundedRectangle(cornerRadius: 26, style: .continuous).strokeBorder(Palette.accent.opacity(0.24), lineWidth: 1))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("accountCard")
    }

    /// Notifications are off (or quiet): say so, and turn them on (the system prompt while it can still show, else
    /// this app's page in the Settings app).
    private var attention: some View {
        HStack(spacing: 12) {
            Image(systemName: "exclamationmark.triangle")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(Palette.warning)
                .accessibilityHidden(true)
            Text(SettingsText.notificationsAttention(access.state))
                .font(RFont.sans(14.5, .medium))
                .foregroundStyle(Palette.text)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button {
                feedback.play(.tap)
                Task { await access.turnOn(openURL: openURL) }
            } label: {
                Text("Turn on")
                    .font(RFont.sans(14, .semibold))
                    .foregroundStyle(Palette.dynamic(0xFFFFFF, 0x231A00))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Palette.warning, in: Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("turnOnNotifications")
        }
        .padding(.leading, 14)
        .padding(.trailing, 10)
        .padding(.vertical, 10)
        .background(Palette.warning.opacity(0.08), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Palette.warning.opacity(0.22), lineWidth: 1))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("notificationsRow")
    }

    private func copyServer() {
        UIPasteboard.general.string = serverUrl
        feedback.play(.copied)
        model.notice = "Server address copied."
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

/// A small status chip on the account card: an icon and a few words, green when all is well, yellow when something
/// needs a look.
private struct StatusChip: View {
    enum Tone { case ok, warn, plain }
    var text: String
    var symbol: String
    var tone: Tone

    private var tint: Color {
        switch tone {
        case .ok: Palette.success
        case .warn: Palette.warning
        case .plain: Palette.secondary
        }
    }

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: symbol).font(.system(size: 12, weight: .semibold)).accessibilityHidden(true)
            Text(text).font(RFont.sans(13, .semibold)).lineLimit(1)
        }
        .foregroundStyle(tint)
        .padding(.leading, 9)
        .padding(.trailing, 12)
        .padding(.vertical, 7)
        .background(tone == .plain ? Palette.controlFill : tint.opacity(0.12), in: Capsule())
        .accessibilityElement(children: .combine)
    }
}

// MARK: Connected

/// What chose to connect: the AI apps, then the computers, each opening its page, and "+ Connect".
private struct ConnectedGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var connecting = false
    /// What the connect sheet chose; done once it has gone (a sheet cannot open while another is still closing).
    @State private var chosen: ConnectChoice?

    /// AI apps first, then the computers, which pinned their key when they paired.
    private var connections: [ConnectionView] {
        model.connections.filter { $0.keyFingerprint == nil } + model.connections.filter { $0.keyFingerprint != nil }
    }

    var body: some View {
        Section {
            ForEach(connections, id: \.id) { connectionRow($0) }
            Button {
                connecting = true
                feedback.defaultTap()
            } label: {
                SettingsRowLabel(title: "Connect", titleColor: Palette.accent, chevron: false) {
                    SettingsTile(symbol: "plus", tint: Palette.accent, washed: true)
                } trailing: {
                    EmptyView()
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Connect an AI app or a computer")
            .accessibilityIdentifier("connect")
            .cardRow()
            // On the row, not the section: a List gives a section's modifiers to each of its rows, and one sheet per
            // connection fights over `connecting`.
            .sheet(isPresented: $connecting, onDismiss: openChosen) {
                ConnectSheet { choice in
                    chosen = choice
                    connecting = false
                }
                .environment(model)
                .environment(\.feedback, feedback)
            }
            .presentationFeedback(connecting)
        } header: {
            GroupHeader("Connected")
        }
    }

    private func openChosen() {
        guard let choice = chosen else { return }
        chosen = nil
        model.connectStartsTyped = choice == .typeCode
        model.openSheet(.connectComputer)
    }

    private func connectionRow(_ connection: ConnectionView) -> some View {
        Button {
            model.show(.connection(connection.id))
            feedback.defaultTap()
        } label: {
            SettingsRowLabel(title: untrusted(connection.label), subtitle: SettingsText.connectionLine(connection)) {
                ConnectionMark(connection: connection, size: 32)
            } trailing: {
                EmptyView()
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(untrusted(connection.label)), \(SettingsText.connectionLine(connection))")
        .accessibilityIdentifier("connection:\(connection.id)")
        .cardRow()
    }
}

/// A connection's mark: the icon picked for it, else a computer's laptop on a cyan tile, else the avatar its name
/// suggests.
struct ConnectionMark: View {
    var connection: ConnectionView
    var size: CGFloat

    var body: some View {
        if connection.keyFingerprint != nil && connection.icon == nil {
            RoundedRectangle(cornerRadius: size * 0.25, style: .continuous)
                .fill(Palette.pair)
                .frame(width: size, height: size)
                .overlay {
                    Image(systemName: "laptopcomputer")
                        .font(.system(size: size * 0.45, weight: .semibold))
                        .foregroundStyle(.white)
                }
                .accessibilityHidden(true)
        } else {
            ConnectionAvatar(label: untrusted(connection.label), pick: connection.icon, size: size)
        }
    }
}

private enum ConnectChoice { case scan, typeCode }

/// "+ Connect": an AI app by the address to paste (copied here), or a computer by the code it shows.
private struct ConnectSheet: View {
    var onChoose: (ConnectChoice) -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.dismiss) private var dismiss
    @State private var help = false
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Connect")
                    .font(RFont.sans(18, .semibold))
                    .foregroundStyle(Palette.text)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                GlassIconButton(symbol: "xmark", size: 34, label: "Close") { dismiss() }
                    .accessibilityIdentifier("closeSheet")
            }
            .padding(.bottom, 4)
            aiApp
            computer
        }
        .padding(.horizontal, 16)
        .padding(.top, 20)
        .frame(maxHeight: .infinity, alignment: .top)
        .presentationDetents([.height(help ? 470 : 400), .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(Palette.background)
        .accessibilityIdentifier("connectSheet")
    }

    private var aiApp: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                HStack(spacing: -10) {
                    ConnectionAvatar(label: "Claude", pick: "claude", size: 34)
                        .overlay(Circle().strokeBorder(Palette.elevated, lineWidth: 2.5))
                        .zIndex(1)
                    ConnectionAvatar(label: "ChatGPT", pick: "openai", size: 34)
                }
                .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    Text("AI app").font(RFont.sans(16.5, .semibold)).foregroundStyle(Palette.text)
                    Text("Claude, ChatGPT and others").font(RFont.sans(13)).foregroundStyle(Palette.tertiary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Button {
                    let next = !help
                    withAnimation(.smooth(duration: 0.25)) { help = next }
                    feedback.play(.expand(next))
                } label: {
                    Image(systemName: "questionmark.circle")
                        .font(.system(size: 17))
                        .foregroundStyle(help ? Palette.accent : Palette.tertiary)
                        .frame(width: 36, height: 36)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(help ? "Hide help" : "Help")
                .accessibilityIdentifier("help")
            }
            if help {
                Text("In Claude.ai, ChatGPT or another AI app, add a custom connector (MCP) and paste this address. It then asks you here before it reads or sends anything.")
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 10)
                    .transition(.opacity)
            }
            Button(action: copy) {
                HStack(spacing: 10) {
                    Text(SettingsText.host(model.mcpAddress))
                        .font(RFont.mono(13))
                        .foregroundStyle(Palette.text)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Label(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc")
                        .font(RFont.sans(14, .semibold))
                        .foregroundStyle(Palette.accent)
                }
                .environment(\.layoutDirection, .leftToRight)
                .padding(.leading, 14)
                .padding(.trailing, 12)
                .padding(.vertical, 11)
                .background(Palette.background, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.top, 14)
            .accessibilityLabel("Copy \(model.mcpAddress)")
            .accessibilityIdentifier("copyMcpUrl")
        }
        .padding(16)
        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("connectAi")
    }

    private var computer: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                SettingsTile(symbol: "laptopcomputer", tint: Palette.pair)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Computer").font(RFont.sans(16.5, .semibold)).foregroundStyle(Palette.text)
                    // A key, not a String, so the command shows in code type.
                    Text(LocalizedStringKey("Desktop app or `reins login`")).font(RFont.sans(13)).foregroundStyle(Palette.tertiary)
                }
            }
            HStack(spacing: 10) {
                Button {
                    feedback.play(.tap)
                    onChoose(.scan)
                } label: {
                    Label("Scan QR code", systemImage: "qrcode.viewfinder")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 44))
                .accessibilityIdentifier("sheetScanQr")
                Button {
                    feedback.play(.tap)
                    onChoose(.typeCode)
                } label: {
                    Text("Type code").font(RFont.sans(15, .semibold))
                }
                .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 44))
                .frame(width: 124)
                .accessibilityIdentifier("sheetTypeCode")
            }
        }
        .padding(16)
        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("connectComputerCard")
    }

    private func copy() {
        UIPasteboard.general.string = model.mcpAddress
        feedback.play(.copied)
        withAnimation(.smooth(duration: 0.2)) { copied = true }
        Task {
            try? await Task.sleep(for: .seconds(2))
            withAnimation(.smooth(duration: 0.2)) { copied = false }
        }
    }
}

// MARK: Pages

/// Autopilot, Integrations and Sounds & haptics, each with its value on the right.
private struct PagesGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    /// The switches as the Sounds page keeps them (in memory, observed), so the value follows them.
    private var sounds: FeedbackSettings { ((feedback as? FeedbackPreviewing)?.store ?? .shared).settings }
    private var mode: AutopilotMode { model.autopilot?.mode ?? .manual }
    /// The vault comes with the account: it is not something the user connected.
    private var accounts: Int { model.accounts.filter { $0.service != "vault" }.count }

    var body: some View {
        Section {
            Button {
                model.show(.autopilot)
                feedback.defaultTap()
            } label: {
                SettingsRowLabel(title: "Autopilot") {
                    SettingsTile(symbol: "bolt.fill", tint: Palette.accent)
                } trailing: {
                    HStack(spacing: 5) {
                        Image(systemName: SettingsText.modeValueSymbol(mode))
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(SettingsText.modeTint(mode))
                        RowValue(SettingsText.modeName(mode))
                    }
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Autopilot")
            .accessibilityValue(SettingsText.modeName(mode))
            .accessibilityIdentifier("openAutopilot")
            .cardRow()
            SettingsLinkRow(
                title: "Integrations", value: SettingsText.accountsValue(accounts), symbol: "puzzlepiece.extension.fill",
                tint: Palette.read, id: "openIntegrations"
            ) { model.show(.integrations) }
            SettingsLinkRow(
                title: "Sounds & haptics", value: SettingsText.soundsValue(sounds), symbol: "speaker.wave.2.fill",
                tint: Palette.accounts, id: "openSounds"
            ) { model.show(.sounds) }
        }
    }
}

// MARK: Security

/// Devices, the recovery code (when this phone keeps the account secret) and the vault's passkeys.
private struct SecurityGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if case .signedIn = model.session {
            Section {
                DevicesRow()
                if model.recoveryCodeAvailable {
                    RecoveryCodeRow()
                    VaultPasskeysRow()
                }
            } header: {
                GroupHeader("Security")
            }
        }
    }
}

/// Settings > Devices: how many are signed in, read again each time Settings shows (only the approval phone may list
/// them; elsewhere the row has no count).
private struct DevicesRow: View {
    @Environment(AppModel.self) private var model
    @State private var count: Int?

    var body: some View {
        SettingsLinkRow(
            title: "Devices", value: count.map { String($0) }, symbol: "laptopcomputer.and.iphone", tint: Palette.search,
            id: "devicesRow", spokenValue: count.map { $0 == 1 ? "1 device" : "\($0) devices" }
        ) { model.show(.devices) }
        .task { count = try? await model.core.devices().count }
    }
}

/// Settings > Vault passkeys: how many open the vault, read again each time Settings shows.
private struct VaultPasskeysRow: View {
    @Environment(AppModel.self) private var model
    @State private var count: Int?

    var body: some View {
        SettingsLinkRow(
            title: "Vault passkeys", value: SettingsText.passkeysValue(count), symbol: "person.badge.key.fill", tint: Palette.accent,
            id: "vaultPasskeysRow", spokenValue: SettingsText.passkeysSummary(count)
        ) { model.show(.vaultPasskeys) }
        .task { count = try? await model.core.vaultPasskeys().count }
    }
}

// MARK: Tour, version

/// "Take the tour" (the setup pages again) and the app's version; updates come from the App Store or TestFlight.
private struct AboutGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Section {
            SettingsLinkRow(title: "Take the tour", symbol: "lifepreserver", tint: Palette.tertiary, id: "takeTour") { model.startTour() }
            SettingsRowLabel(title: "Version", chevron: false) {
                SettingsTile(symbol: "info", tint: Palette.tertiary)
            } trailing: {
                RowValue(SettingsText.version(), mono: true).environment(\.layoutDirection, .leftToRight)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Version")
            .accessibilityValue(SettingsText.version())
            .accessibilityIdentifier("appVersion")
            .cardRow()
        }
    }
}

// MARK: Leaving

/// Signing out, after a confirmation, and deleting the account, after its own sheet.
private struct SessionGroup: View {
    @Environment(AppModel.self) private var model
    @State private var confirm = false
    @State private var busy = false
    @State private var deleting: AccountToDelete?

    private struct AccountToDelete: Identifiable {
        var email: String
        var id: String { email }
    }

    var body: some View {
        Section {
            DestructiveRow(title: "Sign out", symbol: "rectangle.portrait.and.arrow.right", id: "signOut", disabled: busy) {
                confirm = true
            }
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
                DestructiveRow(title: "Delete account", symbol: "trash", id: "deleteAccount", disabled: busy) {
                    deleting = AccountToDelete(email: info.email)
                }
                .sheet(item: $deleting) { shown in
                    DeleteAccountSheet(email: shown.email) { deleting = nil }
                }
                .presentationFeedback(deleting != nil)
            }
        }
    }
}

/// "Reins 1.4 · Terms · Privacy", small and centred under the last group.
private struct FooterSection: View {
    var body: some View {
        Section {
            Text(SettingsText.footer())
                .font(RFont.sans(12.5))
                .foregroundStyle(Palette.tertiary)
                .tint(Palette.secondary)
                .frame(maxWidth: .infinity)
                .multilineTextAlignment(.center)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
                .accessibilityIdentifier("settingsFooter")
        }
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
