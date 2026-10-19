import SwiftUI
import UIKit

/// The Settings section (iOS-Settings.png, 01), short on purpose: you and this phone. Who is signed in (email, server,
/// whether this phone approves and is notified, with the one fix when something is off); this phone (Devices, Sounds and
/// haptics, Appearance); security; the tour; below the fold version, sign out, delete account. What is connected lives
/// on Connections, Autopilot on Trust.
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
        .accessibilityIdentifier("settingsList")
        .navigationTitle("Settings")
        .rootNavigationBar()
        .refreshable {
            await model.refreshConnections()
            await model.refreshPending()
            feedback.play(.refresh)
        }
        .task { await model.refreshConnections() }
        #if DEBUG
        .sheet(item: $debugSheet) { sheet in
            switch sheet {
            case .delete: DeleteAccountSheet(email: "dana@acme.dev") { debugSheet = nil }
            case .recovery: RecoveryCodeSheet(code: "K7QD-2MZR-9XHT-P4WE-B8NC-6JVL-T3FA-Y5GU-M1RS-Q9KD-H2LP-W7EZ-C4XN", required: true) { debugSheet = nil }
            }
        }
        .onAppear {
            // `-openSheet delete|recovery` (screenshots): open that sheet at launch.
            let args = ProcessInfo.processInfo.arguments
            if let i = args.firstIndex(of: "-openSheet"), i + 1 < args.count { debugSheet = DebugSheet(rawValue: args[i + 1]) }
        }
        #endif
    }

    #if DEBUG
    private enum DebugSheet: String, Identifiable {
        case delete, recovery
        var id: String { rawValue }
    }

    @State private var debugSheet: DebugSheet?
    #endif
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
                BlobAvatar(seed: email, size: 48)
                VStack(alignment: .leading, spacing: 3) {
                    Text(email)
                        .font(RFont.sans(16.5, .semibold))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .environment(\.layoutDirection, .leftToRight)
                    Button(action: copyServer) {
                        HStack(spacing: 6) {
                            Text(SettingsText.host(serverUrl))
                                .font(RFont.mono(12.5))
                                .foregroundStyle(Palette.inkMeta)
                                .lineLimit(1)
                            Image(systemName: "doc.on.doc")
                                .font(.system(size: 12))
                                .foregroundStyle(Palette.inkMeta)
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
            .padding(.top, 12)
            if access.state.needsAttention {
                attention.padding(.top, 14)
            }
            if !model.approvalDevice {
                Button(action: register) {
                    HStack(spacing: 8) {
                        if busy { ProgressView().tint(Palette.paper) } else { Image(systemName: "iphone") }
                        Text("Use this phone for approvals")
                    }
                }
                .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 50))
                .disabled(busy)
                .padding(.top, 14)
                .accessibilityIdentifier("registerPhone")
            }
        }
        // On paper, a hairline under it: nothing here asks anything.
        .padding(.horizontal, 4)
        .padding(.vertical, 16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
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
                .foregroundStyle(Palette.ink)
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
        case .plain: Palette.ink2
        }
    }

    var body: some View {
        HStack(spacing: 5) {
            if tone == .ok {
                GlyphIcon(.check, size: 12).accessibilityHidden(true)
            } else {
                Image(systemName: symbol).font(.system(size: 11, weight: .semibold)).accessibilityHidden(true)
            }
            Text(text).font(TextRole.tag.font()).lineLimit(1)
        }
        .foregroundStyle(tint)
        .padding(.leading, 8)
        .padding(.trailing, 10)
        .padding(.vertical, 5)
        .background(tone == .plain ? Palette.paperSunken : tint.opacity(0.12), in: Capsule())
        .accessibilityElement(children: .combine)
    }
}

// MARK: Pages

/// This phone: its devices, Sounds and haptics, Appearance, each with its value on the right.
private struct PagesGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @AppStorage(Appearance.themeKey, store: AppGroup.defaults) private var theme = Appearance.Theme.system.rawValue

    /// The switches as the Sounds page keeps them (in memory, observed), so the value follows them.
    private var sounds: FeedbackSettings { ((feedback as? FeedbackPreviewing)?.store ?? .shared).settings }

    var body: some View {
        Section {
            if case .signedIn = model.session { DevicesRow() }
            SettingsLinkRow(
                title: "Sounds and haptics", value: SettingsText.soundsValue(sounds), symbol: "speaker.wave.2",
                tint: Palette.kindAccounts, id: "openSounds"
            ) { model.show(.sounds) }
            SettingsLinkRow(
                title: "Appearance", value: Appearance.Theme(rawValue: theme)?.title ?? "System", symbol: "circle.lefthalf.filled",
                tint: Palette.ink2, id: "openAppearance"
            ) { model.show(.appearance) }
        } header: {
            GroupHeader("This phone")
        }
    }
}

// MARK: Security

/// Devices, the recovery code (when this phone keeps the account secret) and the vault's passkeys.
private struct SecurityGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if case .signedIn = model.session {
            if model.recoveryCodeAvailable {
                Section {
                    RecoveryCodeRow()
                    VaultPasskeysRow()
                } header: {
                    GroupHeader("Security")
                }
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
            title: "Devices", value: count.map { String($0) }, symbol: "laptopcomputer.and.iphone", tint: Palette.kindRead,
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
            title: "Vault passkeys", value: SettingsText.passkeysValue(count), symbol: "person.badge.key.fill", tint: Palette.violet,
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
            SettingsLinkRow(title: "Take the tour", symbol: "safari", tint: Palette.inkMeta, id: "takeTour") { model.startTour() }
            SettingsRowLabel(title: "Version", chevron: false) {
                SettingsTile(symbol: "info", tint: Palette.inkMeta)
            } trailing: {
                RowValue(SettingsText.version(), mono: true).environment(\.layoutDirection, .leftToRight)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Version")
            .accessibilityValue(SettingsText.version())
            .accessibilityIdentifier("appVersion")
            .cardRow()
        } header: {
            GroupHeader("Reins")
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
                .foregroundStyle(Palette.inkMeta)
                .tint(Palette.ink2)
                .frame(maxWidth: .infinity)
                .multilineTextAlignment(.center)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
                .accessibilityIdentifier("settingsFooter")
        }
    }
}

#if DEBUG
/// Debug builds only: the Signal gallery.
private struct DeveloperGroup: View {
    @State private var gallery = false

    var body: some View {
        Section {
            Button("Signal gallery") { gallery = true }
                .font(RFont.sans(16, .medium))
                .foregroundStyle(Palette.ink)
                .cardRow()
                .accessibilityIdentifier("openSignalGallery")
        } header: {
            GroupHeader("Developer")
        }
        .fullScreenCover(isPresented: $gallery) {
            SignalGallery(page: .type, onClose: { gallery = false })
        }
    }
}
#endif
