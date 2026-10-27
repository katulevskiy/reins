import SwiftUI

/// One AI app or computer (the Connections board, 02): a quiet page. Its avatar and name; its own Autopilot choice
/// (like every AI, Manual, Auto) and its own modes for a while (Overnight, Lockdown, Bypass); the profile it teaches,
/// its grants, when it connected and was last seen; disconnecting it, held. The pencil renames it; its avatar picks
/// its icon. Every mode change goes through `AutopilotModel`, which asks the owner wherever a change loosens what runs
/// (a bypass, Overnight, leaving a lockdown).
struct ConnectionDetailScreen: View {
    var connectionId: String
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    /// Autopilot's profiles, for the profile row; read each time the page shows (they change as Autopilot learns).
    @State private var profiles: [ProfileView] = []
    @State private var naming = false
    @State private var picking = false

    init(connectionId: String) {
        self.connectionId = connectionId
    }

    var body: some View {
        Group {
            if let connection = model.connection(connectionId) {
                NextPage {
                    ConnectionTitle(connection: connection) { picking = true }
                    if let settings = model.autopilot {
                        ConnectionAutopilotGroup(connection: connection, settings: settings)
                    }
                    ConnectionInfoGroup(connection: connection, profiles: profiles, activeGrants: activeGrants(connection.id)) {
                        await loadProfiles()
                    }
                    SigningKeyMore()
                    DisconnectRow(connection: connection)
                }
                .sheet(isPresented: $naming) {
                    RenameSheet(connection: connection) { naming = false }
                        .environment(model)
                        .environment(\.feedback, feedback)
                }
                .presentationFeedback(naming)
                .sheet(isPresented: $picking) {
                    IconPickerSheet(connectionId: connection.id) { icon in
                        picking = false
                        setIcon(icon)
                    }
                    .environment(model)
                    .environment(\.feedback, feedback)
                }
                .presentationFeedback(picking)
            } else {
                EmptyState(symbol: "link", title: "Not connected", message: "This AI is no longer connected.")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .pageBackground()
                    .accessibilityIdentifier("connectionGone")
            }
        }
        .quietPageBar(model.connection(connectionId).map { untrustedName($0.label, limit: 40) } ?? "Connection")
        .toolbar {
            if model.connection(connectionId) != nil {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        naming = true
                        feedback.defaultTap()
                    } label: {
                        GlyphIcon(.pencil, size: 20).foregroundStyle(Palette.ink)
                    }
                    .accessibilityLabel("Name and icon")
                    .accessibilityIdentifier("connectionRename")
                }
            }
        }
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

    private func setIcon(_ icon: String?) {
        let id = connectionId
        Task {
            do {
                try await model.core.setConnectionIcon(connectionId: id, icon: icon)
                await model.refreshConnections()
            } catch {
                feedback.play(.error)
                model.notice = error.userMessage
            }
        }
    }
}

/// Its avatar (which opens the icon picker) and its name: one short clean line, the page's heading. A renamed one says
/// what it calls itself, quoted.
private struct ConnectionTitle: View {
    var connection: ConnectionView
    var onIcon: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            MarkedTitle(title: untrustedName(connection.label, limit: 40), titleId: "connectionName") {
                Button {
                    onIcon()
                    feedback.defaultTap()
                } label: {
                    MarkPlate(size: 36) { ConnectionAvatar(connection: connection, size: 36) }
                        .frame(width: 44, height: 44)
                        .contentShape(Circle())
                }
                .buttonStyle(PressDimsStyle())
                .padding(-4)
                .accessibilityLabel("Icon")
                .accessibilityValue(SettingsText.iconName(connection.icon))
                .accessibilityIdentifier("connectionIcon")
            }
            if let itself = SettingsText.calledItself(connection) {
                Text(itself)
                    .textRole(.meta)
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .padding(.leading, 68)
                    .padding(.trailing, 20)
                    .accessibilityIdentifier("connectionCalledItself")
            }
        }
    }
}

// MARK: Autopilot

/// Its own Autopilot choice (like every AI, Manual, Auto), then "For a while": Overnight, Lockdown or Bypass for just
/// this AI, and while one runs, the mode with Stop. The sheets of Overnight and Bypass are held to confirm, and every
/// change goes through `AutopilotModel` (the owner's check where it loosens what runs).
private struct ConnectionAutopilotGroup: View {
    var connection: ConnectionView
    var settings: AutopilotSettings
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var askBypass = false
    @State private var askOvernight = false
    @State private var help = false
    @State private var ap = AutopilotModel()

    private var own: ConnectionAutopilot? { settings.connections.first { $0.connectionId == connection.id } }
    private var mode: AutopilotMode { own?.mode ?? settings.mode }
    /// What the user chose for this AI; nil = like every AI.
    private var chosen: AutopilotMode? { own?.bypassUntil != nil ? .bypass : own?.baseMode }
    private var label: String { untrustedName(connection.label, limit: 40) }
    /// Every AI is locked down, this one included.
    private var globalLockdown: Bool { mode == .lockdown && settings.mode == .lockdown && own?.baseMode != .lockdown }

    static let help = "Its own mode wins over the global one, except a global Lockdown. A bypass is not possible in an AI's first 10 minutes."

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader("Autopilot", glyph: .autopilot, quiet: true) {
                Button {
                    let next = !help
                    withAnimation(.smooth(duration: 0.25)) { help = next }
                    feedback.play(.expand(next))
                } label: {
                    GlyphIcon(.help, size: 16)
                        .foregroundStyle(help ? Palette.violet : Palette.inkMeta)
                        .frame(width: 44, height: 30)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(help ? "Hide help" : "Help")
                .accessibilityIdentifier("help")
            }
            if help {
                Text(Self.help)
                    .textRole(.sub)
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 20)
                    .padding(.bottom, 8)
                    .transition(.opacity)
            }
            segments.padding(.horizontal, 20)
            FlatGroup(inset: 0) {
                timedRow
            }
            .padding(.top, 10)
            if globalLockdown {
                Label { Text("Every AI is locked down") } icon: { ModeGlyph(.lockdown, size: 14) }
                    .font(RFont.sans(13, .medium))
                    .foregroundStyle(Palette.warning)
                    .padding(.horizontal, 20)
                    .padding(.top, 8)
                    .accessibilityIdentifier("connectionModeLine")
            }
            if let error = ap.error {
                FormBanner(text: error).padding(.horizontal, 20).padding(.top, 10).accessibilityIdentifier("connAutopilotError")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("connectionMode")
        .sheet(isPresented: $askBypass) {
            BypassSheet(who: label) { length in
                let id = connection.id
                Task { await ap.startBypass(connectionId: id, minutes: length.minutes, label: connection.label) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(askBypass)
        .sheet(isPresented: $askOvernight) {
            OvernightSheet(who: label, ap: ap) { length in
                feedback.quietClose()
                let id = connection.id
                Task { await ap.startOvernight(connectionId: id, minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(askOvernight)
        .onAppear { ap.bind(model) }
    }

    /// Like all, Manual, Auto: a sunken well, the chosen one raised; Manual and Auto carry their glyphs.
    private var segments: some View {
        HStack(spacing: 2) {
            segment(nil, "Like all", id: "connMode:follow")
            segment(.manual, "Manual", id: "connMode:MANUAL", glyph: .manual)
            segment(.auto, "Auto", id: "connMode:AUTO", glyph: .auto)
        }
        .padding(4)
        .background(Palette.paperSunken, in: Capsule())
        .animation(.easeOut(duration: Motion.fade), value: chosen)
    }

    private func segment(_ m: AutopilotMode?, _ title: String, id: String, glyph: ModeKind? = nil) -> some View {
        let selected = m == chosen
        return Button {
            guard !selected else { return }
            feedback.play(.selection)
            setMode(m)
        } label: {
            HStack(spacing: 5) {
                if let glyph { ModeGlyph(glyph, size: 14).foregroundStyle(glyph == .auto ? Palette.violet : (selected ? Palette.ink : Palette.ink2)) }
                Text(title).lineLimit(1).minimumScaleFactor(0.8)
            }
            .font(RFont.sans(14, .semibold))
            .foregroundStyle(selected ? Palette.ink : Palette.ink2)
            .frame(maxWidth: .infinity, minHeight: 38)
            .background {
                if selected {
                    Capsule().fill(Palette.paperRaised)
                        .shadow(color: .black.opacity(0.08), radius: 1, y: 1)
                        .overlay(Capsule().strokeBorder(Palette.rule, lineWidth: 1))
                }
            }
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier(id)
    }

    /// "For a while" and its three modes, a menu of them; a running one shows itself with Stop instead.
    @ViewBuilder private var timedRow: some View {
        if let until = own?.bypassUntil {
            running(.bypass, until: until, stopId: "stopConnectionBypass") {
                Task { await ap.stopBypass(connection.id) }
            }
        } else if let until = own?.overnightUntil {
            running(.overnight, until: until, stopId: "stopConnectionOvernight") {
                Task { await ap.stopOvernight(connection.id) }
            }
        } else if chosen == .lockdown {
            HStack(spacing: 12) {
                ModeTile(mode: .lockdown, side: 24)
                Text(ModeKind.lockdown.name).textRole(.row).foregroundStyle(Palette.ink).frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("connectionModeLine")
                Button("Unlock") { setMode(nil) }
                    .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                    .accessibilityIdentifier("connMode:LOCKDOWN")
            }
            .frame(minHeight: 52)
        } else {
            Menu {
                Button { feedback.play(.tap); askOvernight = true } label: { Label(AutopilotText.overnightName, image: Glyph.moon.rawValue) }
                    .accessibilityIdentifier("connMode:OVERNIGHT")
                Button { setMode(.lockdown) } label: { Label(ModeKind.lockdown.name, image: Glyph.block.rawValue) }
                    .accessibilityIdentifier("connMode:LOCKDOWN")
                Button(role: .destructive) { feedback.play(.tap); askBypass = true } label: { Label(ModeKind.bypass.name, image: Glyph.shieldAlert.rawValue) }
                    .accessibilityIdentifier("connMode:BYPASS")
            } label: {
                ForAWhileRow().padding(.horizontal, -14)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("For a while")
            .accessibilityIdentifier("connForAWhile")
        }
    }

    private func running(_ kind: ModeKind, until: Int64, stopId: String, stop: @escaping () -> Void) -> some View {
        HStack(spacing: 12) {
            ModeTile(mode: kind, side: 24)
            Group {
                if AutopilotText.isUntilOff(until) {
                    Text("\(kind.name) · until off")
                } else {
                    TimelineView(.periodic(from: .now, by: 15)) { ctx in
                        Text("\(kind.name) · " + AutopilotText.timeLeft(until: until, now: nowSeconds(ctx.date)))
                    }
                }
            }
            .textRole(.row)
            .foregroundStyle(kind == .bypass ? Palette.danger : Palette.ink)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityIdentifier("connectionModeLine")
            // Stopping is words only, everywhere.
            Button("Stop", action: stop)
                .buttonStyle(SignalButtonStyle(kind == .bypass ? .danger : .soft, size: .small, fill: false))
                .accessibilityIdentifier(stopId)
        }
        .frame(minHeight: 52)
    }

    /// This AI's mode (`nil`: back to following the global one), through `AutopilotModel`: leaving its lockdown asks
    /// the owner; the change is felt at once and a refusal from the core says why.
    private func setMode(_ new: AutopilotMode?) {
        let id = connection.id
        Task { await ap.setMode(new, connectionId: id, label: connection.label) }
    }
}

// MARK: Details

/// The profile its answers teach (picked from a menu), its active grants, when it connected and was last seen; a
/// computer's key, as it pinned it.
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
    private var profileName: String? { profiles.first { $0.id == profileId }.map { untrustedName($0.name, limit: 24) } }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader("Details", quiet: true)
            FlatGroup {
                if settings != nil && !profiles.isEmpty {
                    Menu {
                        Picker("Profile", selection: Binding(get: { profileId ?? "" }, set: assign)) {
                            ForEach(profiles, id: \.id) { p in
                                Text(untrustedName(p.name, limit: 32) + (p.id == settings?.defaultProfileId ? " (default)" : "")).tag(p.id)
                            }
                        }
                    } label: {
                        TileRow("Profile", glyph: .fileText, value: profileName).padding(.horizontal, -14)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Profile")
                    .accessibilityValue(profileName ?? "")
                    .accessibilityIdentifier("learnsInto")
                }
                Button {
                    model.select(.trust)
                    feedback.defaultTap()
                } label: {
                    TileRow("Grants", glyph: .key, value: activeGrants == 0 ? "None" : "\(activeGrants)").padding(.horizontal, -14)
                }
                .buttonStyle(.plain)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("activeGrants")
                TileRow("Connected", glyph: .calendar, value: Date(timeIntervalSince1970: TimeInterval(connection.createdAt)).formatted(.dateTime.month(.abbreviated).day().year()), chevron: false)
                    .padding(.horizontal, -14)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("connectedOn")
                TileRow("Last seen", glyph: .timer, value: SettingsText.lastUsed(connection), chevron: false)
                    .padding(.horizontal, -14)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("lastUsed")
                if let key = connection.keyFingerprint {
                    // Its key's digits as this phone verified them when it paired, on a mono row.
                    TileRow("Key", glyph: .fingerprint, value: key, mono: true, chevron: false)
                        .padding(.horizontal, -14)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("connectionKey")
                }
            }
            if let error {
                FormBanner(text: error).padding(.horizontal, 20).padding(.top, 10)
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

/// "More", closed at first: this phone's signing key, which agents and the desktop app pin to check what it signs. The
/// same for every connection; read when opened. One mono row, its (?) beside it.
private struct SigningKeyMore: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var open = false
    @State private var key: String?
    @State private var failed = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                withAnimation(.smooth) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack {
                    Text("More").font(RFont.sans(15, .medium))
                    Spacer()
                    Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).rotationEffect(.degrees(open ? 90 : 0))
                }
                .foregroundStyle(Palette.violet)
                .padding(.horizontal, 24)
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("connectionMore")
            .accessibilityValue(open ? "Open" : "Closed")
            if open {
                FlatGroup {
                    HStack(spacing: 8) {
                        TileRow("Signing key", glyph: .fingerprint, value: key ?? (failed ? "Unavailable" : "…"), mono: key != nil, chevron: false)
                            .accessibilityElement(children: .combine)
                            .accessibilityIdentifier("signingKey")
                        HelpDot(id: "signingKey", text: SettingsText.signingKeyHelp, size: 28)
                    }
                    .padding(.horizontal, -14)
                }
                .transition(.opacity)
                .task { await load() }
            }
        }
    }

    private func load() async {
        guard key == nil else { return }
        do {
            key = SettingsText.signingKey(try await model.core.signingKeyFingerprint())
            failed = key == nil
        } catch {
            failed = true
        }
    }
}

/// The name field: the user's name for it, its own as the placeholder. Empty (or the clear button) gives it its own
/// name back. What it calls itself stays under the field.
struct RenameSheet: View {
    var connection: ConnectionView
    var done: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var text: String
    @State private var busy = false
    @State private var error: String?
    @FocusState private var focused: Bool

    init(connection: ConnectionView, done: @escaping () -> Void) {
        self.connection = connection
        self.done = done
        _text = State(initialValue: connection.name ?? "")
    }

    private var own: String {
        untrustedName(connection.ownLabel.isEmpty ? connection.label : connection.ownLabel, limit: 64)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Name")
                .font(RFont.sans(18, .semibold))
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
            HStack(spacing: 8) {
                TextField(own, text: $text)
                    .font(RFont.sans(16))
                    .focused($focused)
                    .submitLabel(.done)
                    .textInputAutocapitalization(.words)
                    .onSubmit(save)
                    .onChange(of: text) { _, new in if new.count > PairingModel.maxLabel { text = String(new.prefix(PairingModel.maxLabel)) } }
                    .accessibilityLabel("Name")
                    .accessibilityIdentifier("renameField")
                if !text.isEmpty {
                    Button {
                        text = ""
                        feedback.play(.selection)
                    } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(Palette.inkFaint)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Use its own name")
                    .accessibilityIdentifier("renameClear")
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
            .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
            Text("Calls itself " + PairWords.quoted(connection.clientName.isEmpty ? own : connection.clientName))
                .textRole(.meta)
                .foregroundStyle(Palette.inkMeta)
                .lineLimit(1)
                .accessibilityIdentifier("renameItself")
            if let error { FormBanner(text: error) }
            Button(action: save) {
                Label { Text("Save") } icon: { Image(systemName: "checkmark") }
                    .font(RFont.sans(16, .semibold))
                    .frame(maxWidth: .infinity, minHeight: 50)
            }
            .buttonStyle(.borderedProminent)
            .disabled(busy)
            .accessibilityIdentifier("renameSave")
        }
        .padding(20)
        .presentationDetents([.height(300)])
        .presentationDragIndicator(.visible)
        .presentationBackground(Palette.paper)
        .onAppear { focused = true }
        // A container: its own id must not replace the field's, Save's and "Use its own name"'s.
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("renameSheet")
    }

    private func save() {
        guard !busy else { return }
        busy = true
        error = nil
        let id = connection.id
        let typed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            do {
                try await model.renameConnection(id, to: typed.isEmpty ? nil : typed)
                feedback.play(.close)
                done()
            } catch {
                feedback.play(.error)
                self.error = error.userMessage
            }
            busy = false
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
                    .foregroundStyle(Palette.ink)
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
        .presentationBackground(Palette.paper)
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

/// Disconnecting it: the quiet danger row, held, no dialog. Its grants end with it.
private struct DisconnectRow: View {
    var connection: ConnectionView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            QuietDangerRow(
                title: "Disconnect \(untrustedName(connection.label, limit: 32))", glyph: .unlink, note: "Hold to disconnect. Its grants end.",
                busy: busy, onConfirm: disconnect
            )
            .accessibilityIdentifier("disconnect")
            if let error { FormBanner(text: error).padding(.horizontal, 20) }
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
