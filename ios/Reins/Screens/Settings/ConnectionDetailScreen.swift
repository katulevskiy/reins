import SwiftUI

/// One AI app or computer: its icon and name, its Autopilot mode (like every AI, its own, a bypass or a lockdown), the
/// profile it teaches, its grants, when it connected, its icon, and disconnecting it (held to confirm).
///
/// Each group is its own small view, and no group declares a `let` inside the list's builder: one big `List` body
/// with local `let`s miscompiles with Xcode 27.1 beta (the app crashes destroying the list value).
struct ConnectionDetailScreen: View {
    var connectionId: String
    @Environment(AppModel.self) private var model
    /// Autopilot's profiles, for "Learns into"; read each time the page shows (they change as Autopilot learns).
    @State private var profiles: [ProfileView] = []

    init(connectionId: String) {
        self.connectionId = connectionId
    }

    var body: some View {
        Group {
            if let connection = model.connection(connectionId) {
                List {
                    ConnectionHeader(connection: connection)
                    ConnectionAutopilotIfRead(connection: connection)
                    ConnectionInfoGroup(connection: connection, profiles: profiles, activeGrants: activeGrants(connection.id)) {
                        await loadProfiles()
                    }
                    ConnectionIconGroup(connection: connection)
                    DisconnectGroup(connection: connection)
                }
                .reinsGrouped()
            } else {
                EmptyState(symbol: "link", title: "Not connected", message: "This AI is no longer connected.")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .pageBackground()
                    .accessibilityIdentifier("connectionGone")
            }
        }
        .navigationTitle(model.connection(connectionId).map { untrusted($0.label) } ?? "Connection")
        .navigationBarTitleDisplayMode(.inline)
        // The name is the page's own heading, large and centred under the icon.
        .toolbar { ToolbarItem(placement: .principal) { EmptyView() } }
        .task {
            await model.refreshAutopilot()
            await loadProfiles()
        }
    }

    private func activeGrants(_ id: String) -> Int {
        model.grants.filter { $0.connectionId == id && $0.active }.count
    }

    private func loadProfiles() async {
        if let read = try? await model.core.autopilotProfiles() { profiles = read }
    }
}

/// The icon, the name, and where it connects from and when it was last used.
private struct ConnectionHeader: View {
    var connection: ConnectionView

    var body: some View {
        Section {
            VStack(spacing: 2) {
                ConnectionMark(connection: connection, size: 76)
                Text(untrusted(connection.label))
                    .font(RFont.sans(24, .semibold))
                    .foregroundStyle(Palette.text)
                    .multilineTextAlignment(.center)
                    .padding(.top, 10)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("connectionName")
                Text(SettingsText.connectionHeadline(connection))
                    .font(RFont.mono(13))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityIdentifier("connectionHeadline")
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 4)
            .listRowBackground(Color.clear)
        }
    }
}

// MARK: Autopilot

/// The Autopilot group, once Autopilot's settings have been read.
private struct ConnectionAutopilotIfRead: View {
    var connection: ConnectionView
    @Environment(AppModel.self) private var model

    var body: some View {
        if let settings = model.autopilot {
            ConnectionAutopilotGroup(connection: connection, settings: settings)
        }
    }
}

/// The Autopilot part of an AI's page: like every AI or its own mode (Manual, Auto) in a segmented control, then a
/// bypass for just this AI (the bypass sheet, held to confirm) and a lockdown for just this AI (a tap).
private struct ConnectionAutopilotGroup: View {
    var connection: ConnectionView
    var settings: AutopilotSettings
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var askBypass = false
    @State private var error: String?

    private var own: ConnectionAutopilot? { settings.connections.first { $0.connectionId == connection.id } }
    private var mode: AutopilotMode { own?.mode ?? settings.mode }
    /// What the user chose for this AI; nil = like every AI.
    private var chosen: AutopilotMode? { own?.bypassUntil != nil ? .bypass : own?.baseMode }
    private var label: String { untrusted(connection.label) }
    /// Every AI is locked down, this one included.
    private var globalLockdown: Bool { mode == .lockdown && settings.mode == .lockdown && own?.baseMode != .lockdown }
    /// The choices of the segmented control: like every AI (nil), then the modes that are not one of the buttons.
    private var choices: [AutopilotMode?] {
        [nil] + SettingsText.modes.filter { ![.overnight, .bypass, .lockdown].contains($0) }.map { Optional($0) }
    }

    static let help = "Its own mode wins over the global one, except a global Lockdown. A bypass is not possible in an AI's first 10 minutes."

    var body: some View {
        Section {
            VStack(alignment: .leading, spacing: 12) {
                segments
                HStack(spacing: 8) {
                    bypassButton
                    lockButton
                }
                if globalLockdown {
                    Text("Every AI is locked down")
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.warning)
                        .accessibilityIdentifier("connectionModeLine")
                }
                if let error {
                    FormBanner(text: error).accessibilityIdentifier("connAutopilotError")
                }
            }
            .padding(.vertical, 6)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("connectionMode")
            .cardRow()
        } header: {
            HelpHeader(title: "Autopilot", help: Self.help)
        }
        .sheet(isPresented: $askBypass) {
            BypassSheet(who: label) { length in
                startBypass(minutes: length.minutes)
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(askBypass)
    }

    private var segments: some View {
        HStack(spacing: 2) {
            ForEach(choices, id: \.self) { m in
                Button {
                    guard m != chosen else { return }
                    feedback.play(.selection)
                    setMode(m)
                } label: {
                    HStack(spacing: 4) {
                        if let m {
                            Image(systemName: m == .manual ? SettingsText.modeSymbol(m) : SettingsText.modeValueSymbol(m))
                                .font(.system(size: 12, weight: .semibold))
                                .foregroundStyle(m == .manual ? SettingsText.modeTint(m) : Palette.secondary)
                        }
                        Text(m.map(SettingsText.modeName) ?? "Like all")
                            .font(RFont.sans(14, m == chosen ? .semibold : .medium))
                            .foregroundStyle(m == chosen ? Palette.text : Palette.secondary)
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, minHeight: 34)
                    .background(m == chosen ? Palette.dynamic(0xFFFFFF, 0x2C2C31) : Color.clear, in: Capsule())
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(m == chosen ? .isSelected : [])
                .accessibilityIdentifier(m.map { "connMode:\(SettingsText.modeName($0).uppercased())" } ?? "connMode:follow")
            }
        }
        .padding(3)
        .background(Palette.controlFill, in: Capsule())
        .animation(.smooth(duration: 0.2), value: chosen)
    }

    @ViewBuilder private var bypassButton: some View {
        if let until = own?.bypassUntil {
            Button(role: .destructive, action: stopBypass) {
                // Stopping a bypass is words only, everywhere.
                HStack(spacing: 6) {
                    if AutopilotText.isUntilOff(until) {
                        Text("Stop · until off")
                            .accessibilityIdentifier("connectionModeLine")
                    } else {
                        TimelineView(.periodic(from: .now, by: 15)) { ctx in
                            Text("Stop · " + AutopilotText.timeLeft(until: until, now: nowSeconds(ctx.date)))
                                .accessibilityIdentifier("connectionModeLine")
                        }
                    }
                }
                .modePill(fill: Palette.danger.opacity(0.16), text: Palette.danger)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("stopConnectionBypass")
        } else {
            Button {
                feedback.play(.tap)
                askBypass = true
            } label: {
                Label("Bypass", systemImage: "bolt.fill")
                    .modePill(fill: Palette.danger.opacity(0.12), text: Palette.danger)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("connMode:BYPASS")
        }
    }

    @ViewBuilder private var lockButton: some View {
        if chosen == .lockdown {
            Button {
                setMode(nil)
            } label: {
                Label("Unlock", systemImage: "lock.open.fill")
                    .modePill(fill: Palette.warning.opacity(0.14), text: Palette.warning)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("connMode:LOCKDOWN")
        } else {
            Button {
                setMode(.lockdown)
            } label: {
                Label("Lock down", systemImage: "lock.fill")
                    .modePill(fill: Palette.controlFill, text: Palette.text)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("connMode:LOCKDOWN")
        }
    }

    /// Sets this AI's mode (`nil`: back to following the global one); not Bypass, which `startBypass` turns on. The
    /// change is felt at once; a refusal from the core says why.
    private func setMode(_ new: AutopilotMode?) {
        if let event = SettingsText.modeChangeEvent(mode, new ?? settings.mode) { feedback.play(event) }
        let id = connection.id
        model.patchAutopilot { $0.applyMode(new, minutes: 0, connectionId: id) }
        run {
            try await model.core.setAutopilotMode(connectionId: id, mode: new, minutes: nil)
        }
    }

    /// A bypass of this AI for `minutes`, or until turned off (nil), after its sheet was held. Holding proves intent,
    /// not identity: the phone's owner confirms it too (Face ID or the passcode), and without that nothing changes,
    /// not even for a moment. A refusal from the core (a just-paired connection cannot be bypassed) says why.
    private func startBypass(minutes: UInt32?) {
        let id = connection.id
        let who = untrusted(connection.label)
        Task {
            guard await model.authenticator.confirm("Turn on Bypass for \(who)") else { return }
            feedback.play(.bypassOn)
            model.patchAutopilot { $0.applyBypass(minutes: minutes, connectionId: id) }
            run {
                try await model.core.startBypass(connectionId: id, minutes: minutes)
            }
        }
    }

    /// Ends this AI's bypass: it goes back to its own setting.
    private func stopBypass() {
        feedback.play(.bypassOff)
        let (id, base) = (connection.id, own?.baseMode)
        model.patchAutopilot { $0.applyMode(base, minutes: 0, connectionId: id) }
        run { try await model.core.setAutopilotMode(connectionId: id, mode: base, minutes: nil) }
    }

    private func run(_ work: @escaping () async throws -> Void) {
        error = nil
        Task {
            do {
                try await work()
            } catch {
                feedback.play(.error)
                self.error = error.userMessage
            }
            await model.refreshAutopilot()
        }
    }
}

private extension View {
    /// One of the two buttons under the mode control: a 40-point pill in its colour.
    func modePill(fill: Color, text: Color) -> some View {
        font(RFont.sans(14.5, .semibold))
            .foregroundStyle(text)
            .lineLimit(1)
            .frame(maxWidth: .infinity, minHeight: 40)
            .background(fill, in: Capsule())
            .contentShape(Capsule())
    }
}

// MARK: Profile, grants, date

/// "Learns into" (the profile its answers teach, picked from a menu), its active grants, and when it connected.
private struct ConnectionInfoGroup: View {
    var connection: ConnectionView
    var profiles: [ProfileView]
    var activeGrants: Int
    var reload: () async -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var error: String?

    private var settings: AutopilotSettings? { model.autopilot }
    private var profileId: String? {
        guard let s = settings else { return nil }
        return s.connections.first { $0.connectionId == connection.id }?.profileId ?? s.defaultProfileId
    }
    private var profileName: String? { profiles.first { $0.id == profileId }.map { untrusted($0.name) } }

    var body: some View {
        Section {
            if settings != nil && !profiles.isEmpty {
                Menu {
                    Picker("Learns into", selection: Binding(get: { profileId ?? "" }, set: assign)) {
                        ForEach(profiles, id: \.id) { p in
                            Text(untrusted(p.name) + (p.id == settings?.defaultProfileId ? " (default)" : "")).tag(p.id)
                        }
                    }
                } label: {
                    SettingsRowLabel(title: "Learns into") {
                        SettingsTile(symbol: "brain.head.profile", tint: Palette.accent)
                    } trailing: {
                        if let profileName { RowValue(profileName) }
                    }
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Learns into")
                .accessibilityValue(profileName ?? "")
                .accessibilityIdentifier("learnsInto")
                .cardRow()
            }
            SettingsRowLabel(title: "Active grants", chevron: false) {
                SettingsTile(symbol: "key.fill", tint: Palette.send)
            } trailing: {
                RowValue(activeGrants == 0 ? "None" : "\(activeGrants)")
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("activeGrants")
            .cardRow()
            SettingsRowLabel(title: "Connected", chevron: false) {
                SettingsTile(symbol: "clock.arrow.circlepath", tint: Palette.tertiary)
            } trailing: {
                RowValue(SettingsText.shortDate(connection.createdAt))
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("connectedOn")
            .cardRow()
            if let error {
                FormBanner(text: error).cardRow()
            }
        }
    }

    /// Which profile this AI's decisions train (nil = the default profile).
    private func assign(_ picked: String) {
        guard picked != profileId, let s = settings else { return }
        feedback.play(.selection)
        let id = connection.id
        let profile: String? = picked == s.defaultProfileId ? nil : picked
        model.patchAutopilot { $0.applyProfile(profile, connectionId: id) }
        error = nil
        Task {
            do {
                try await model.core.assignProfile(connectionId: id, profileId: profile)
                await reload()
            } catch {
                feedback.play(.error)
                self.error = error.userMessage
            }
            await model.refreshAutopilot()
        }
    }
}

// MARK: Icon, disconnect

/// The icon it shows everywhere: the current pick, opening the picker.
private struct ConnectionIconGroup: View {
    var connection: ConnectionView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var picking = false
    @State private var error: String?

    var body: some View {
        Section {
            Button {
                picking = true
                feedback.defaultTap()
            } label: {
                SettingsRowLabel(title: "Icon") {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .fill(Palette.controlFill)
                        .frame(width: 32, height: 32)
                        .overlay { ConnectionAvatar(label: untrusted(connection.label), pick: connection.icon, size: 24) }
                        .accessibilityHidden(true)
                } trailing: {
                    RowValue(SettingsText.iconName(connection.icon))
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Icon")
            .accessibilityValue(SettingsText.iconName(connection.icon))
            .accessibilityIdentifier("connectionIcon")
            .cardRow()
            // On the row: a section's modifiers go to each of its rows, the error banner's too.
            .sheet(isPresented: $picking) {
                IconPickerSheet(label: untrusted(connection.label), selection: connection.icon) { icon in
                    picking = false
                    setIcon(icon)
                }
                .environment(\.feedback, feedback)
            }
            .presentationFeedback(picking)
            if let error { FormBanner(text: error).cardRow() }
        }
    }

    private func setIcon(_ icon: String?) {
        error = nil
        let id = connection.id
        Task {
            do {
                try await model.core.setConnectionIcon(connectionId: id, icon: icon)
                await model.refreshConnections()
            } catch {
                feedback.play(.error)
                self.error = error.userMessage
            }
        }
    }
}

/// The icons to pick from; picking one closes the sheet.
private struct IconPickerSheet: View {
    var label: String
    var selection: String?
    var onPick: (String?) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Icon")
                    .font(RFont.sans(18, .semibold))
                    .foregroundStyle(Palette.text)
                    .accessibilityAddTraits(.isHeader)
                ProviderPicker(label: label, selection: selection, size: 50, onPick: onPick)
                GroupFooter(ProviderPicker.footnote)
            }
            .padding(20)
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(Palette.background)
        .accessibilityIdentifier("iconSheet")
    }
}

/// Disconnecting it: held for two seconds, no dialog.
private struct DisconnectGroup: View {
    var connection: ConnectionView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        Section {
            VStack(spacing: 10) {
                if let error { FormBanner(text: error) }
                HoldButton(title: "Hold to disconnect", symbol: "trash", busy: busy, height: 52, onConfirm: disconnect)
                    .accessibilityIdentifier("disconnect")
            }
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets())
        }
    }

    private func disconnect() {
        guard !busy else { return }
        busy = true
        error = nil
        feedback.play(.revoked)
        let id = connection.id
        Task {
            do {
                try await model.core.revokeConnection(connectionId: id)
                await model.refreshConnections()
                await model.refreshPending()
                model.back()
            } catch {
                feedback.play(.error)
                self.error = error.userMessage
            }
            busy = false
        }
    }
}
