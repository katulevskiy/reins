import SwiftUI

/// The Autopilot part of an AI's page (the Android app's `ConnectionAutopilotSection`): its own mode (or the global
/// one), a bypass for just this AI, and the profile its answers teach. Self-contained: it reads and changes
/// everything through `AppModel`, confirms Bypass and Lockdown with the owner, plays the same feedback as the
/// Autopilot screen, and shows its own errors.
///
/// For a `List` (an inset grouped settings list), as a `Section`:
///
///     List { ...; ConnectionAutopilotSection(connectionId: id); ... }
///
/// For a page of cards in a `ScrollView`, use `ConnectionAutopilotCard(connectionId:)`.
struct ConnectionAutopilotSection: View {
    var connectionId: String
    /// The AI's name in the dialogs; the connection's own label when nil.
    var label: String? = nil

    var body: some View {
        Section {
            ConnectionAutopilotContent(connectionId: connectionId, label: label)
                .listRowInsets(EdgeInsets())
        } header: {
            SectionHeader("Autopilot").textCase(nil).padding(.horizontal, -4)
        } footer: {
            Text(ConnectionAutopilotContent.footer)
                .font(RFont.sans(13))
                .foregroundStyle(Palette.secondary)
        }
        .listRowBackground(Palette.elevated)
    }
}

/// The same as `ConnectionAutopilotSection`, as a heading, a card and its footnote for a `ScrollView` page.
struct ConnectionAutopilotCard: View {
    var connectionId: String
    var label: String? = nil

    var body: some View {
        AutopilotGroup(header: "Autopilot", footer: ConnectionAutopilotContent.footer) {
            ConnectionAutopilotContent(connectionId: connectionId, label: label)
        }
    }
}

/// The rows both forms show.
struct ConnectionAutopilotContent: View {
    var connectionId: String
    var label: String?

    static let footer = "Its own mode wins over the global one, except a global Lockdown. A bypass is not possible in an AI's first 10 minutes."

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var ap = AutopilotModel()
    @State private var bypassAsk = false
    @State private var lockdownAsk = false

    private var name: String { label ?? model.connection(connectionId)?.label ?? "this AI" }

    var body: some View {
        Group {
            if let s = model.autopilot {
                rows(s)
            } else {
                ProgressView().frame(maxWidth: .infinity).padding(16)
            }
        }
        .task(id: connectionId) {
            ap.bind(model)
            await ap.refresh()
        }
        .sheet(isPresented: $bypassAsk) {
            BypassSheet(who: name) { minutes in
                feedback.quietClose()
                Task { await ap.setMode(.bypass, minutes: minutes, connectionId: connectionId, label: name) }
            }
        }
        .presentationFeedback(bypassAsk)
        .alert("Lock down \(untrusted(name))?", isPresented: $lockdownAsk) {
            Button("Cancel", role: .cancel) {}
            Button("Lock down", role: .destructive) {
                feedback.quietClose()
                Task { await ap.setMode(.lockdown, connectionId: connectionId, label: name) }
            }
        } message: {
            Text("Everything it asks for is denied at once, what waits now included. Other AIs are not affected.")
        }
        .presentationFeedback(lockdownAsk)
    }

    private func rows(_ s: AutopilotSettings) -> some View {
        let own = s.connections.first { $0.connectionId == connectionId }
        let mode = own?.mode ?? s.mode
        let bypassUntil = own?.bypassUntil
        let chosen: AutopilotMode? = bypassUntil != nil ? .bypass : own?.baseMode
        let profileId = own?.profileId ?? s.defaultProfileId
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 14) {
                IconTile(symbol: AutopilotText.symbol(mode), tint: AutopilotStyle.tint(mode), size: 44, filled: true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(AutopilotText.name(mode)).font(RFont.sans(17, .semibold)).foregroundStyle(Palette.text)
                    // Only a bypass counts down; otherwise the line does not change with time.
                    if bypassUntil != nil {
                        TimelineView(.periodic(from: .now, by: 1)) { context in
                            modeText(modeLine(s, own: own, mode: mode, chosen: chosen, now: Int64(context.date.timeIntervalSince1970)), bypass: true)
                        }
                    } else {
                        modeText(modeLine(s, own: own, mode: mode, chosen: chosen, now: Int64(Date().timeIntervalSince1970)), bypass: false)
                    }
                }
                Spacer(minLength: 8)
                if bypassUntil != nil {
                    ActionButton(title: "Stop", symbol: "stop.fill", kind: .destructive, compact: true) {
                        Task { await ap.stopBypass(connectionId) }
                    }
                    .accessibilityIdentifier("stopConnectionBypass")
                }
            }
            .padding(16)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("connectionMode")

            FlowRow(spacing: 8) {
                AutopilotChip(title: "Like every AI", selected: chosen == nil, identifier: "connMode:follow") {
                    if chosen != nil { Task { await ap.setMode(nil, connectionId: connectionId) } }
                }
                ForEach(AutopilotText.modes, id: \.self) { m in
                    AutopilotChip(title: AutopilotText.name(m), selected: chosen == m, identifier: "connMode:\(AutopilotText.key(m))") {
                        pick(m, chosen: chosen)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 16)

            if !ap.profiles.isEmpty {
                RowDivider()
                VStack(alignment: .leading, spacing: 10) {
                    AutopilotCaption("Learns into")
                    FlowRow(spacing: 8) {
                        ForEach(ap.profiles, id: \.id) { p in
                            let title = "\(AutopilotText.profileIcon(p))  \(untrusted(p.name))" + (p.id == s.defaultProfileId ? " (default)" : "")
                            AutopilotChip(title: title, selected: p.id == profileId, identifier: "connProfile:\(p.id)") {
                                if p.id != profileId {
                                    Task { await ap.assignProfile(connectionId, p.id == s.defaultProfileId ? nil : p.id) }
                                }
                            }
                        }
                    }
                }
                .padding(16)
            }

            if let error = ap.error {
                IntegrationBanner(text: error, kind: .error)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 16)
                    .accessibilityIdentifier("connAutopilotError")
            }
        }
    }

    private func modeText(_ line: String, bypass: Bool) -> some View {
        Text(line)
            .font(RFont.sans(13))
            .foregroundStyle(bypass ? Palette.danger : Palette.secondary)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier("connectionModeLine")
    }

    private func modeLine(_ s: AutopilotSettings, own: ConnectionAutopilot?, mode: AutopilotMode, chosen: AutopilotMode?, now: Int64) -> String {
        if mode == .lockdown && s.mode == .lockdown && own?.baseMode != .lockdown { return "Every AI is locked down" }
        if let until = own?.bypassUntil { return "Bypass for this AI · " + AutopilotText.minutesLeft(until: until, now: now) }
        if chosen == nil { return "Like every AI (Settings, then Autopilot)" }
        return "Its own mode"
    }

    private func pick(_ m: AutopilotMode, chosen: AutopilotMode?) {
        switch m {
        case .bypass:
            // Refused for an AI paired in the last 10 minutes; say so before asking for Face ID.
            if let refusal = ap.bypassRefusal(connectionId) {
                model.feedback.play(.error)
                ap.error = refusal
            } else {
                ap.error = nil
                bypassAsk = true
            }
        case .lockdown where chosen != .lockdown:
            lockdownAsk = true
        default:
            if m != chosen { Task { await ap.setMode(m, connectionId: connectionId) } }
        }
    }
}
