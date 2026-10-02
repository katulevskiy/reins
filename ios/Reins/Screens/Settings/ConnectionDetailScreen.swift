import SwiftUI

/// One AI connection: its icon (a provider's or one drawn from the name), its own Autopilot mode and the profile its
/// answers teach, its history, and disconnecting it.
///
/// Each group is its own small view, and no group declares a `let` inside the list's builder: one big `List` body
/// with local `let`s miscompiles with Xcode 27.1 beta (the app crashes destroying the list value).
struct ConnectionDetailScreen: View {
    var connectionId: String
    @Environment(AppModel.self) private var model

    init(connectionId: String) {
        self.connectionId = connectionId
    }

    var body: some View {
        Group {
            if let connection = model.connection(connectionId) {
                List {
                    ConnectionHeader(connection: connection)
                    ConnectionAutopilotIfRead(connection: connection)
                    ConnectionIconGroup(connection: connection)
                    ConnectionActivityGroup(connection: connection, activeGrants: activeGrants(connection.id))
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
        .task { await model.refreshAutopilot() }
    }

    private func activeGrants(_ id: String) -> Int {
        model.grants.filter { $0.connectionId == id && $0.active }.count
    }
}

private struct ConnectionHeader: View {
    var connection: ConnectionView

    var body: some View {
        Section {
            VStack(spacing: 4) {
                ConnectionAvatar(label: untrusted(connection.label), pick: connection.icon, size: 84)
                Text(untrusted(connection.label))
                    .font(RFont.sans(24, .semibold))
                    .foregroundStyle(Palette.text)
                    .multilineTextAlignment(.center)
                    .padding(.top, 8)
                    .accessibilityAddTraits(.isHeader)
                Text(untrusted(connection.clientHost))
                    .font(RFont.mono(13))
                    .foregroundStyle(Palette.secondary)
                    .environment(\.layoutDirection, .leftToRight)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 6)
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

/// The Autopilot part of an AI's page: its own mode (or the global one), a bypass for just this AI, and the profile
/// its answers teach.
private struct ConnectionAutopilotGroup: View {
    var connection: ConnectionView
    var settings: AutopilotSettings
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var profiles: [ProfileView] = []
    @State private var askBypass = false
    @State private var askLockdown = false
    @State private var error: String?

    private var own: ConnectionAutopilot? { settings.connections.first { $0.connectionId == connection.id } }
    private var mode: AutopilotMode { own?.mode ?? settings.mode }
    /// What the user chose for this AI; nil = like every AI.
    private var chosen: AutopilotMode? { own?.bypassUntil != nil ? .bypass : own?.baseMode }
    private var profileId: String { own?.profileId ?? settings.defaultProfileId }
    private var label: String { untrusted(connection.label) }

    var body: some View {
        Section {
            modeRow.cardRow()
            modeChips.padding(.vertical, 6).cardRow()
            if !profiles.isEmpty {
                profileChips.padding(.vertical, 6).cardRow()
            }
            if let error {
                FormBanner(text: error).accessibilityIdentifier("connAutopilotError").cardRow()
            }
        } header: {
            GroupHeader("Autopilot")
        } footer: {
            GroupFooter("Its own mode wins over the global one, except a global Lockdown. A bypass is not possible in an AI's first 10 minutes.")
        }
        .task { await loadProfiles() }
        .confirmationDialog("Lock down \(label)?", isPresented: $askLockdown, titleVisibility: .visible) {
            Button("Lock down", role: .destructive) { setMode(.lockdown) }
        } message: {
            Text("Everything it asks for is denied at once, what waits now included. Other AIs are not affected.")
        }
        .sheet(isPresented: $askBypass) {
            ConnectionBypassSheet(who: label) { minutes in
                askBypass = false
                setMode(.bypass, minutes: minutes)
            }
            .environment(\.feedback, feedback)
        }
    }

    private var modeRow: some View {
        HStack(spacing: 14) {
            Image(systemName: SettingsText.modeSymbol(mode))
                .font(.system(size: 19, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 44, height: 44)
                .background(SettingsText.modeTint(mode), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(SettingsText.modeName(mode))
                    .font(RFont.sans(17, .semibold))
                    .foregroundStyle(Palette.text)
                TimelineView(.periodic(from: .now, by: 15)) { ctx in
                    Text(modeLine(now: nowSeconds(ctx.date)))
                        .font(RFont.sans(13))
                        .foregroundStyle(own?.bypassUntil != nil ? Palette.danger : Palette.secondary)
                        .lineLimit(2)
                        .accessibilityIdentifier("connectionModeLine")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if own?.bypassUntil != nil {
                Button(role: .destructive, action: stopBypass) {
                    Label("Stop", systemImage: "stop.fill")
                }
                .buttonStyle(SmallCapsuleStyle(kind: .danger))
                .accessibilityIdentifier("stopConnectionBypass")
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("connectionMode")
    }

    private var modeChips: some View {
        ChipFlow {
            OptionChip("Like every AI", selected: chosen == nil) {
                if chosen != nil { setMode(nil) }
            }
            .accessibilityIdentifier("connMode:follow")
            ForEach(SettingsText.modes, id: \.self) { m in
                OptionChip(SettingsText.modeName(m), selected: chosen == m) { pick(m) }
                    .accessibilityIdentifier("connMode:\(SettingsText.modeName(m).uppercased())")
            }
        }
    }

    private var profileChips: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Learns into").font(RFont.sans(13, .medium)).foregroundStyle(Palette.tertiary)
            ChipFlow {
                ForEach(profiles, id: \.id) { p in
                    OptionChip(profileTitle(p), selected: p.id == profileId) {
                        if p.id != profileId { assignProfile(p.id == settings.defaultProfileId ? nil : p.id) }
                    }
                    .accessibilityIdentifier("connProfile:\(p.id)")
                }
            }
        }
    }

    private func profileTitle(_ p: ProfileView) -> String {
        "\(SettingsText.profileIcon(p))  \(untrusted(p.name))" + (p.id == settings.defaultProfileId ? " (default)" : "")
    }

    private func modeLine(now: Int64) -> String {
        if mode == .lockdown && settings.mode == .lockdown && own?.baseMode != .lockdown { return "Every AI is locked down" }
        if let until = own?.bypassUntil { return "Bypass for this AI · " + SettingsText.minutesLeft(until, now: now) }
        if own?.baseMode == nil { return "Like every AI (set in Autopilot)" }
        return "Its own mode"
    }

    private func pick(_ m: AutopilotMode) {
        if m == .bypass {
            askBypass = true
        } else if m == .lockdown && chosen != m {
            askLockdown = true
        } else if m != chosen {
            setMode(m)
        }
    }

    /// Sets this AI's mode (`nil`: back to following the global one). The change is felt at once; a refusal from the
    /// core (a just-paired connection cannot be bypassed) says why.
    private func setMode(_ new: AutopilotMode?, minutes: UInt32? = nil) {
        let old = mode
        let restart = new == .bypass && old == .bypass
        if let event = restart ? .bypassOn : SettingsText.modeChangeEvent(old, new ?? settings.mode) { feedback.play(event) }
        let id = connection.id
        run {
            try await model.core.setAutopilotMode(
                connectionId: id, mode: new, minutes: new == .bypass ? (minutes ?? SettingsText.bypassMinutes[0]) : nil
            )
        }
    }

    /// Ends this AI's bypass: it goes back to its own setting.
    private func stopBypass() {
        feedback.play(.bypassOff)
        let (id, base) = (connection.id, own?.baseMode)
        run { try await model.core.setAutopilotMode(connectionId: id, mode: base, minutes: nil) }
    }

    /// Which profile this AI's decisions train (nil = the default profile).
    private func assignProfile(_ profileId: String?) {
        let id = connection.id
        run {
            try await model.core.assignProfile(connectionId: id, profileId: profileId)
            await loadProfiles()
        }
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

    private func loadProfiles() async {
        do {
            profiles = try await model.core.autopilotProfiles()
        } catch {
            self.error = error.userMessage
        }
    }
}

// MARK: Icon, activity, disconnect

private struct ConnectionIconGroup: View {
    var connection: ConnectionView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var error: String?

    var body: some View {
        Section {
            IconPicker(label: untrusted(connection.label), selected: connection.icon, onPick: setIcon)
                .padding(.vertical, 8)
                .cardRow()
            if let error { FormBanner(text: error).cardRow() }
        } header: {
            GroupHeader("Icon")
        } footer: {
            GroupFooter("Auto picks a known AI from the name, or draws a blobatar for it.")
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

private struct ConnectionActivityGroup: View {
    var connection: ConnectionView
    var activeGrants: Int

    var body: some View {
        Section {
            InfoRow("Connected", subtitle: GrantText.full(connection.createdAt)).cardRow()
            InfoRow("Last used", subtitle: lastUsed).cardRow()
            InfoRow("Active grants", subtitle: activeGrants == 0 ? "None" : "\(activeGrants)").cardRow()
        } header: {
            GroupHeader("Activity")
        }
    }

    private var lastUsed: String {
        connection.lastUsedAt.map { "\(GrantText.relative($0)) · \(GrantText.full($0))" } ?? "Never"
    }
}

private struct DisconnectGroup: View {
    var connection: ConnectionView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var confirm = false
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        Section {
            VStack(spacing: 10) {
                if let error { FormBanner(text: error) }
                Button(role: .destructive) {
                    confirm = true
                } label: {
                    Label("Disconnect", systemImage: "trash")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .danger, height: 50))
                .disabled(busy)
                .accessibilityIdentifier("disconnect")
            }
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets())
            .confirmationDialog("Disconnect \(untrusted(connection.label))?", isPresented: $confirm, titleVisibility: .visible) {
                Button("Disconnect", role: .destructive, action: disconnect)
            } message: {
                Text("It loses access immediately, and its saved grants stop working.")
            }
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

/// The icons an AI connection can wear: Auto (the provider its name suggests, or a blobatar), a blobatar, or a
/// provider's logo. A plain grid for now; the shared provider picker can replace it once it exists.
private struct IconPicker: View {
    var label: String
    var selected: String?
    var onPick: (String?) -> Void
    @Environment(\.feedback) private var feedback

    /// Provider keys (the `provider-<key>` images) and their names, in the Android app's order.
    static let providers: [(key: String, name: String)] = [
        ("claude", "Claude"), ("openai", "ChatGPT"), ("gemini", "Gemini"), ("grok", "Grok"), ("hermes", "Hermes"),
        ("perplexity", "Perplexity"), ("mistral", "Mistral"), ("deepseek", "DeepSeek"), ("copilot", "Copilot"),
        ("cursor", "Cursor"), ("qwen", "Qwen"), ("kimi", "Kimi"), ("meta", "Meta AI"), ("ollama", "Ollama"),
    ]

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 66), spacing: 10)], spacing: 14) {
            choice(tag: "auto", name: "Auto", isOn: selected == nil, avatarPick: nil, avatarLabel: label) { onPick(nil) }
            choice(tag: "blob", name: "Blobatar", isOn: selected == "blob", avatarPick: "blob", avatarLabel: label) { onPick("blob") }
            ForEach(Self.providers, id: \.key) { p in
                choice(tag: p.key, name: p.name, isOn: selected == p.key, avatarPick: p.key, avatarLabel: p.name) { onPick(p.key) }
            }
        }
    }

    private func choice(tag: String, name: String, isOn: Bool, avatarPick: String?, avatarLabel: String, action: @escaping () -> Void) -> some View {
        Button {
            feedback.play(.selection)
            action()
        } label: {
            VStack(spacing: 5) {
                ConnectionAvatar(label: avatarLabel, pick: avatarPick, size: 50)
                    .padding(3)
                    .overlay(Circle().strokeBorder(isOn ? Palette.accent : .clear, lineWidth: 2.5))
                Text(name)
                    .font(RFont.sans(12, isOn ? .semibold : .regular))
                    .foregroundStyle(isOn ? Palette.accent : Palette.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(name)
        .accessibilityAddTraits(isOn ? [.isSelected] : [])
        .accessibilityIdentifier("icon:\(tag)")
    }
}

/// Turning on a bypass for one AI: how long, and what it means.
private struct ConnectionBypassSheet: View {
    var who: String
    var onConfirm: (UInt32) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var minutes: UInt32 = SettingsText.bypassMinutes[0]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Image(systemName: "bolt.fill")
                .font(.system(size: 21, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 48, height: 48)
                .background(Palette.danger, in: RoundedRectangle(cornerRadius: 15, style: .continuous))
                .accessibilityHidden(true)
            Text("Bypass \(who)?")
                .font(RFont.sans(20, .semibold))
                .foregroundStyle(Palette.text)
                .padding(.top, 14)
            Text("Requests are approved without asking until the time runs out. Approved means done: an email sent cannot be unsent.")
                .font(RFont.sans(15))
                .foregroundStyle(Palette.secondary)
                .padding(.top, 8)
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "shield").font(.system(size: 16)).foregroundStyle(Palette.secondary)
                Text("Still asked every time: new connections, permissions, passwords and secrets, deletions and other one-off changes, SSH and flagged files.")
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.secondary)
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .padding(.top, 14)
            Text("For").font(RFont.sans(13, .medium)).foregroundStyle(Palette.tertiary).padding(.top, 16).padding(.bottom, 8)
            HStack(spacing: 8) {
                ForEach(SettingsText.bypassMinutes, id: \.self) { m in
                    OptionChip(m == 60 ? "1 hour" : "\(m) min", selected: minutes == m) { minutes = m }
                        .accessibilityIdentifier("bypass:\(m)")
                }
            }
            Spacer(minLength: 22)
            HStack(spacing: 10) {
                Button("Cancel") { dismiss() }
                    .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 50))
                Button {
                    onConfirm(minutes)
                } label: {
                    Label("Turn on", systemImage: "bolt.fill")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .danger, height: 50))
                .accessibilityIdentifier("confirmBypass")
            }
        }
        .padding(22)
        .presentationDetents([.height(500), .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(Palette.background)
        .accessibilityIdentifier("bypassDialog")
    }
}
