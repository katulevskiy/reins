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
            // Its mark, its name in the serif (one short clean line), where it is and when it was last used.
            HStack(spacing: 14) {
                ConnectionAvatar(connection: connection, size: 56)
                VStack(alignment: .leading, spacing: 2) {
                    Text(untrustedName(connection.label, limit: 40))
                        .font(RFont.serif(40))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                        .minimumScaleFactor(0.6)
                        .accessibilityAddTraits(.isHeader)
                        .accessibilityIdentifier("connectionName")
                    Text(SettingsText.connectionHeadline(connection))
                        .textRole(.meta)
                        .foregroundStyle(Palette.inkMeta)
                        .lineLimit(1)
                        .environment(\.layoutDirection, .leftToRight)
                        .accessibilityIdentifier("connectionHeadline")
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
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

/// The Autopilot part of an AI's page: like every AI or its own mode (Manual, Auto) in a chooser, then an
/// Overnight and a bypass for just this AI (their sheets, held to confirm) and a lockdown for just this AI (a tap).
private struct ConnectionAutopilotGroup: View {
    var connection: ConnectionView
    var settings: AutopilotSettings
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var askBypass = false
    @State private var askOvernight = false
    @State private var error: String?
    /// Overnight starts and stops as on the Autopilot page (the owner's check, the mode it interrupted).
    @State private var ap = AutopilotModel()

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
                overnightButton
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
                if let error = error ?? ap.error {
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
        .sheet(isPresented: $askOvernight) {
            OvernightSheet(who: label, ap: ap) { length in
                feedback.quietClose()
                error = nil
                Task { await ap.startOvernight(connectionId: connection.id, minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(askOvernight)
        .onAppear { ap.bind(model) }
    }

    /// Like all, Manual, Auto: the Signal chooser, the chosen one raised.
    private var segments: some View {
        Chooser(choices.map { m in
            Chooser.Segment(
                id: m.map { "connMode:\(SettingsText.modeName($0).uppercased())" } ?? "connMode:follow",
                title: m.map(SettingsText.modeName) ?? "Like all",
                selected: m == chosen
            ) {
                feedback.play(.selection)
                setMode(m)
            }
        })
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
                .modePill(fill: Palette.dangerWash, text: Palette.danger)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("stopConnectionBypass")
        } else {
            Button {
                feedback.play(.tap)
                askBypass = true
            } label: {
                Label { Text("Bypass") } icon: { GlyphIcon(.shieldPaused, size: 16) }
                    .modePill(fill: Palette.dangerWash, text: Palette.danger)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("connMode:BYPASS")
        }
    }

    /// Overnight for just this AI, or, while it runs, Stop with the time it has left. Words only to stop, as a bypass.
    @ViewBuilder private var overnightButton: some View {
        if let until = own?.overnightUntil {
            Button {
                error = nil
                Task { await ap.stopOvernight(connection.id) }
            } label: {
                Group {
                    if AutopilotText.isUntilOff(until) {
                        Text("Stop \(AutopilotText.overnightName) · until off")
                    } else {
                        TimelineView(.periodic(from: .now, by: 15)) { ctx in
                            Text("Stop \(AutopilotText.overnightName) · " + AutopilotText.timeLeft(until: until, now: nowSeconds(ctx.date)))
                        }
                    }
                }
                .modePill(fill: Palette.overnight.opacity(0.16), text: Palette.overnight)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("stopConnectionOvernight")
        } else {
            Button {
                feedback.play(.tap)
                askOvernight = true
            } label: {
                Label { Text(AutopilotText.overnightName) } icon: { GlyphIcon(.moon, size: 16) }
                    .modePill(fill: Palette.overnight.opacity(0.12), text: Palette.overnight)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("connMode:OVERNIGHT")
        }
    }

    @ViewBuilder private var lockButton: some View {
        if chosen == .lockdown {
            Button {
                setMode(nil)
            } label: {
                Label { Text("Unlock") } icon: { GlyphIcon(.lockOpen, size: 16) }
                    .modePill(fill: Palette.warning.opacity(0.14), text: Palette.warning)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("connMode:LOCKDOWN")
        } else {
            Button {
                setMode(.lockdown)
            } label: {
                Label { Text("Lock down") } icon: { GlyphIcon(.lock, size: 16) }
                    .modePill(fill: Palette.paperSunken, text: Palette.ink)
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
        // Leaving its Overnight by picking another mode: what it interrupted no longer matters.
        if own?.baseMode == .overnight { OvernightReturn.forget(id) }
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
            model.patchAutopilot { $0.applyBypass(minutes: minutes, connectionId: id, overnightReturn: OvernightReturn.mode(id)) }
            run {
                try await model.core.startBypass(connectionId: id, minutes: minutes)
            }
        }
    }

    /// Ends this AI's bypass: it goes back to its own setting (never to Overnight: the bypass ended that).
    private func stopBypass() {
        feedback.play(.bypassOff)
        let (id, s) = (connection.id, settings)
        model.patchAutopilot { $0.applyBypassEnd(connectionId: id) }
        run { try await model.endBypass(id, in: s) }
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
        font(RFont.sans(14.5, .medium))
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
                        .overlay { ConnectionAvatar(connection: connection, size: 24) }
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
                IconPickerSheet(connectionId: connection.id) { icon in
                    picking = false
                    setIcon(icon)
                }
                .environment(model)
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

/// The icons to pick from (picking one closes the sheet), then the blobatar's shape, colour and Shuffle, which stay
/// open and redraw as the core keeps each change.
private struct IconPickerSheet: View {
    var connectionId: String
    var onPick: (String?) -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var error: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Icon")
                    .font(RFont.sans(18, .semibold))
                    .foregroundStyle(Palette.text)
                    .accessibilityAddTraits(.isHeader)
                if let connection = model.connection(connectionId) {
                    ProviderPicker(connection: connection, size: 50, onPick: onPick)
                    GroupFooter(ProviderPicker.footnote)
                    editor(connection)
                        .padding(.top, 8)
                    if let error { FormBanner(text: error) }
                }
            }
            .padding(20)
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(Palette.background)
        .accessibilityIdentifier("iconSheet")
    }

    private func editor(_ connection: ConnectionView) -> some View {
        let kept = ConnectionLook.blobatar(connection)
        return BlobatarEditor(
            seed: kept?.seed ?? untrusted(connection.label),
            shape: kept?.shape,
            colour: kept?.colour,
            onShape: { change(.shape(UInt8($0))) },
            onColour: { change(.colour(UInt8($0))) },
            onShuffle: { change(.shuffle) }
        )
    }

    private func change(_ change: AppModel.BlobatarChange) {
        error = nil
        let id = connectionId
        Task {
            do {
                try await model.changeBlobatar(id, change)
            } catch {
                feedback.play(.error)
                self.error = error.userMessage
            }
        }
    }
}

/// Disconnecting it: held, no dialog.
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
