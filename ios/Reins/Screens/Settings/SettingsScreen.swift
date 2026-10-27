import SwiftUI
import UIKit

/// The Settings tab (the Settings board, 01), a standard page like the other tab roots: the title, who is signed in
/// (whether this phone approves and is alerted, with the one fix when something is off; Copy puts the server address on
/// the clipboard), this phone (Devices, Sounds, Appearance, Wallpaper), security, the tour and the version, then Sign
/// out and Delete account. What is connected lives on Connections, Autopilot on Trust.
///
/// Each group is its own small view, so each redraws only what changed.
struct SettingsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
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
            .padding(.bottom, 24)
        }
        .stickyHeader {
            PageHeader(title: "Settings") { EmptyView() }
                .frame(minHeight: 44)
                .padding(.horizontal, 20)
                .padding(.bottom, 8)
        }
        .pageBackground()
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
            case .delete: DeleteAccountSheet(email: "me@example.com") { debugSheet = nil }
            case .recovery: RecoveryCodeSheet(code: "TKRQ-7HXM-2PLA-W4ZD-QE6N-B3VY-JF5C-K8SU-RM2G-XT7H-NAPQ-D6WL", required: true) { debugSheet = nil }
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

/// A row of a Settings group that opens something: the `TileRow` as a button, with its identifier and spoken value.
struct SettingsTileButton<Leading: View>: View {
    var title: String
    var value: String?
    var id: String
    var spokenValue: String?
    var chevron = true
    var danger = false
    var leading: Leading
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    init(
        _ title: String, value: String? = nil, id: String, spokenValue: String? = nil, chevron: Bool = true, danger: Bool = false,
        @ViewBuilder leading: () -> Leading, action: @escaping () -> Void
    ) {
        self.title = title
        self.value = value
        self.id = id
        self.spokenValue = spokenValue
        self.chevron = chevron
        self.danger = danger
        self.leading = leading()
        self.action = action
    }

    var body: some View {
        Button {
            action()
            // The page or sheet it opens has the sound.
            feedback.defaultTap()
        } label: {
            TileRow(title, value: value, danger: danger, chevron: chevron) { leading } trailing: { EmptyView() }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityValue(spokenValue ?? value ?? "")
        .accessibilityIdentifier(id)
    }
}

extension SettingsTileButton where Leading == GlyphTile {
    init(
        _ title: String, glyph: Glyph, value: String? = nil, id: String, spokenValue: String? = nil, chevron: Bool = true,
        danger: Bool = false, action: @escaping () -> Void
    ) {
        self.init(title, value: value, id: id, spokenValue: spokenValue, chevron: chevron, danger: danger, leading: { GlyphTile(glyph, danger: danger) }, action: action)
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
            VStack(spacing: 0) {
                RaisedGroup {
                    AccountCard(email: info.email, serverUrl: info.serverUrl, serverPush: serverPush, busy: $busy, message: $message, error: $error)
                }
                if let message {
                    FormBanner(text: message, kind: .info).padding(.horizontal, 16).padding(.top, 10)
                }
                if let error = error ?? (model.approvalDevice ? nil : model.registrationError) {
                    FormBanner(text: error).padding(.horizontal, 16).padding(.top, 10)
                }
            }
            .padding(.top, 16)
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

/// Who is signed in, whether this phone approves and is alerted (green tags when all is well, or the one thing to do
/// about it), and Copy for the server's address.
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
            HStack(spacing: 12) {
                BlobAvatar(seed: email, size: 44)
                VStack(alignment: .leading, spacing: 3) {
                    Text(email)
                        .font(RFont.sans(16.5, .semibold))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .environment(\.layoutDirection, .leftToRight)
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 12) { tags }
                        VStack(alignment: .leading, spacing: 3) { tags }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                RowIconButton(glyph: .copy, label: "Copy server", action: copyServer)
                    .accessibilityValue(SettingsText.host(serverUrl))
                    .accessibilityHint("Copies the server address")
                    .accessibilityIdentifier("copyServer")
                    .padding(.trailing, -4)
            }
            .frame(minHeight: 76)
            if access.state.needsAttention {
                attention.padding(.bottom, 14)
            }
            if !model.approvalDevice {
                Button(action: register) {
                    HStack(spacing: 8) {
                        if busy { ProgressView().tint(Palette.onInverse) } else { GlyphIcon(.phone, size: 18) }
                        Text("Use this phone for approvals")
                    }
                }
                .buttonStyle(SignalButtonStyle(.inverse, size: .small))
                .disabled(busy)
                .padding(.bottom, 14)
                .accessibilityIdentifier("registerPhone")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 0)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("accountCard")
    }

    @ViewBuilder private var tags: some View {
        if model.approvalDevice {
            StatusTag(text: "Approves", glyph: .shieldCheck).accessibilityLabel("Approval phone").accessibilityIdentifier("approvalChip")
        } else {
            StatusTag(text: "Another phone approves", glyph: .phone, tint: Palette.warning).accessibilityIdentifier("approvalChip")
        }
        switch push {
        case .on: StatusTag(text: "Alerts", glyph: .bell).accessibilityLabel("Notifications on").accessibilityIdentifier("notificationsChip")
        case .onlyWhileOpen: StatusTag(text: "Only while open", glyph: .bell, tint: Palette.ink2).accessibilityIdentifier("notificationsChip")
        case .hidden: EmptyView()
        }
    }

    /// Notifications are off (or quiet): say so, and turn them on (the system prompt while it can still show, else
    /// this app's page in the Settings app).
    private var attention: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) {
                attentionText
                turnOn
            }
            VStack(alignment: .leading, spacing: 8) {
                attentionText
                turnOn
            }
        }
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .padding(.vertical, 6)
        .background(Palette.warning.opacity(0.1), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("notificationsRow")
    }

    private var attentionText: some View {
        HStack(spacing: 10) {
            GlyphIcon(.warning, size: 16).foregroundStyle(Palette.warning).accessibilityHidden(true)
            Text(SettingsText.notificationsAttention(access.state))
                .font(RFont.sans(14.5, .medium))
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var turnOn: some View {
        Button {
            feedback.play(.tap)
            Task { await access.turnOn(openURL: openURL) }
        } label: {
            Text("Turn on").fixedSize()
        }
        .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
        .accessibilityIdentifier("turnOnNotifications")
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

// MARK: This phone

/// This phone: its devices, Sounds, Appearance and the wallpaper, each with its value on the right.
private struct PagesGroup: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.colorScheme) private var scheme
    @AppStorage(Appearance.themeKey, store: AppGroup.defaults) private var theme = Appearance.Theme.system.rawValue
    @AppStorage(WallpaperChoice.key(.dark), store: AppGroup.defaults) private var darkWall = WallpaperChoice.initial(.dark)
    @AppStorage(WallpaperChoice.key(.light), store: AppGroup.defaults) private var lightWall = WallpaperChoice.initial(.light)

    /// The switches as the Sounds page keeps them (in memory, observed), so the value follows them.
    private var sounds: FeedbackSettings { ((feedback as? FeedbackPreviewing)?.store ?? .shared).settings }

    /// The image shown for the theme (the default when your photo is chosen but gone).
    private var wallpaper: Wallpaper {
        let image = (scheme == .dark ? darkWall : lightWall).image
        return image == .own && !OwnWallpaper.exists(scheme) ? .defaultFor(scheme) : image
    }

    var body: some View {
        SectionHeader("This phone", glyph: .phone)
        RaisedGroup {
            if case .signedIn = model.session { DevicesRow() }
            SettingsTileButton("Sounds", glyph: .speaker, value: SettingsText.soundsValue(sounds), id: "openSounds") { model.show(.sounds) }
            SettingsTileButton("Appearance", glyph: .appearance, value: Appearance.Theme(rawValue: theme)?.title ?? "System", id: "openAppearance") {
                model.show(.appearance)
            }
            SettingsTileButton("Wallpaper", value: wallpaper.title, id: "openWallpaper") {
                ChosenWallpaperThumb(scheme: scheme, side: 32, radius: 9)
            } action: {
                model.show(.wallpaper)
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
        SettingsTileButton(
            "Devices", glyph: .devices, value: count.map { String($0) }, id: "devicesRow",
            spokenValue: count.map { $0 == 1 ? "1 device" : "\($0) devices" }
        ) { model.show(.devices) }
        .task { count = try? await model.core.devices().count }
    }
}

// MARK: Security

/// The recovery code (when this phone keeps the account secret) and the vault's passkeys.
private struct SecurityGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if case .signedIn = model.session, model.recoveryCodeAvailable {
            SectionHeader("Security", glyph: .shield)
            RaisedGroup {
                RecoveryCodeRow()
                VaultPasskeysRow()
            }
        }
    }
}

/// Settings > Passkeys: how many open the vault, read again each time Settings shows.
private struct VaultPasskeysRow: View {
    @Environment(AppModel.self) private var model
    @State private var count: Int?

    var body: some View {
        SettingsTileButton(
            "Passkeys", glyph: .faceId, value: SettingsText.passkeysValue(count), id: "vaultPasskeysRow",
            spokenValue: SettingsText.passkeysSummary(count)
        ) { model.show(.vaultPasskeys) }
        .task { count = try? await model.core.vaultPasskeys().count }
    }
}

// MARK: Tour, version

/// The tour (the setup pages again) and the app's version; updates come from the App Store or TestFlight.
private struct AboutGroup: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        SectionHeader("Reins", glyph: .help)
        RaisedGroup {
            SettingsTileButton("Tour", glyph: .play, id: "takeTour") { model.startTour() }
            TileRow("Version", glyph: .package, value: SettingsText.version(), mono: true, chevron: false)
                .environment(\.layoutDirection, .leftToRight)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("Version")
                .accessibilityValue(SettingsText.version())
                .accessibilityIdentifier("appVersion")
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
        RaisedGroup {
            SettingsTileButton("Sign out", glyph: .quit, id: "signOut", chevron: false, danger: true) { confirm = true }
                .disabled(busy)
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
                SettingsTileButton("Delete account", glyph: .trash, id: "deleteAccount", chevron: false, danger: true) {
                    deleting = AccountToDelete(email: info.email)
                }
                .disabled(busy)
                .sheet(item: $deleting) { shown in
                    DeleteAccountSheet(email: shown.email) { deleting = nil }
                }
                .presentationFeedback(deleting != nil)
            }
        }
        .padding(.top, 22)
    }
}

/// "Reins 1.4 · Terms · Privacy", small and centred under the last group.
private struct FooterSection: View {
    var body: some View {
        Text(SettingsText.footer())
            .font(RFont.sans(12.5))
            .foregroundStyle(Palette.inkMeta)
            .tint(Palette.ink2)
            .frame(maxWidth: .infinity)
            .multilineTextAlignment(.center)
            .padding(.top, 20)
            .accessibilityIdentifier("settingsFooter")
    }
}

#if DEBUG
/// Debug builds only: the Signal gallery.
private struct DeveloperGroup: View {
    @State private var gallery = false

    var body: some View {
        SectionHeader("Developer", glyph: .terminal)
        RaisedGroup {
            SettingsTileButton("Signal gallery", glyph: .appearance, id: "openSignalGallery") { gallery = true }
        }
        .fullScreenCover(isPresented: $gallery) {
            SignalGallery(page: .type, onClose: { gallery = false })
        }
    }
}
#endif
