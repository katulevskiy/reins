import SwiftUI

/// Autopilot: the global mode, the model that runs on this phone, and the profiles that learn from the user's
/// answers. The Autopilot section's root, and also pushed from Activity's mode pill and Settings.
struct AutopilotScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var ap = AutopilotModel()
    @State private var bypassAsk = false
    @State private var lockdownAsk = false
    @State private var deleteAsk = false
    @State private var mobileDataAsk = false
    @State private var newProfile = false

    var body: some View {
        ScrollView {
            if let s = ap.settings {
                content(s)
                    .frame(maxWidth: 680)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 32)
            } else {
                ProgressView().padding(.top, 60).frame(maxWidth: .infinity)
            }
        }
        .background(Palette.background.ignoresSafeArea())
        .navigationTitle("Autopilot")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable {
            feedback.play(.refresh)
            await ap.refresh()
        }
        .task {
            ap.bind(model)
            await ap.refresh()
        }
        .sheet(isPresented: $bypassAsk) {
            BypassSheet(who: nil) { minutes in
                feedback.quietClose()
                Task { await ap.setMode(.bypass, minutes: minutes) }
            }
        }
        .presentationFeedback(bypassAsk)
        .sheet(isPresented: $newProfile) {
            ProfileEditorSheet(title: "New profile", initialName: "", initialIcon: AutopilotText.icons[2], confirmLabel: "Create") { name, icon in
                feedback.quietClose()
                Task {
                    if let id = await ap.createProfile(name: name, icon: icon) { model.openFromAutopilot(.autopilotProfile(id)) }
                }
            }
        }
        .presentationFeedback(newProfile)
        .alert("Lock down?", isPresented: $lockdownAsk) {
            Button("Cancel", role: .cancel) {}
            Button("Lock down", role: .destructive) {
                feedback.quietClose()
                Task { await ap.setMode(.lockdown) }
            }
        } message: {
            Text("Every request is denied, waiting ones too.")
        }
        .presentationFeedback(lockdownAsk)
        .alert("Delete the model?", isPresented: $deleteAsk) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                feedback.quietClose()
                Task { await ap.deleteModel() }
            }
        } message: {
            Text("Assisted and Auto pause until you download it again.")
        }
        .presentationFeedback(deleteAsk)
        .confirmationDialog("You are on mobile data", isPresented: $mobileDataAsk, titleVisibility: .visible) {
            Button("Download on mobile data") { ap.downloadNow() }
            Button("Wait for Wi-Fi") { ap.download() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The model is \(ap.model.map(AutopilotText.modelSize) ?? "a few hundred MB"). Download it now over mobile data, or wait until this phone is on Wi-Fi?")
        }
        .presentationFeedback(mobileDataAsk)
    }

    private func content(_ s: AutopilotSettings) -> some View {
        VStack(alignment: .leading, spacing: 22) {
            ModeHero(settings: s, modelReady: ap.modelReady, onStopBypass: { Task { await ap.stopBypass() } }, onEndLockdown: { Task { await ap.endLockdown() } })
                .padding(.top, 8)
            if ap.error != nil || ap.notice != nil {
                VStack(spacing: 8) {
                    if let error = ap.error { IntegrationBanner(text: error, kind: .error).accessibilityIdentifier("autopilotError") }
                    if let notice = ap.notice { IntegrationBanner(text: notice).accessibilityIdentifier("autopilotNotice") }
                }
            }

            AutopilotGroupHeaderless(header: "Mode", footer: "For every AI, unless one has a mode of its own (Settings, then the AI). Riskier requests, such as passwords, deletions and new connections, always wait for you.") {
                ModePicker(selected: s.mode, modelReady: ap.modelReady) { pick($0, s) }
            }

            ModelCard(ap: ap, wifiOnly: s.wifiOnly, onDownload: download, onDelete: { deleteAsk = true })

            AutopilotGroup(header: "Profiles", footer: "Each AI's answers teach its profile. Every AI uses the default one unless you pick another on its page.") {
                ForEach(ap.profiles, id: \.id) { profile in
                    ProfileRow(profile: profile) { model.openFromAutopilot(.autopilotProfile(profile.id)) }
                    RowDivider(inset: 70)
                }
                AutopilotRow(title: "New profile", symbol: "plus", identifier: "newProfile") { newProfile = true }
            }

            AutopilotGroup(header: "Try it", footer: "Type a request and see what Autopilot would do with it. Nothing is kept.") {
                AutopilotRow(
                    title: "See how Autopilot judges",
                    subtitle: ap.modelReady ? nil : "Download the model first",
                    symbol: "flask",
                    chevron: true,
                    identifier: "openTryIt"
                ) { model.openFromAutopilot(.tryIt(nil)) }
            }
        }
        .task(id: s.bypassUntil) {
            // The core ends a bypass when it is read after its time; read it then, so the screen moves on by itself.
            guard let until = s.bypassUntil else { return }
            let left = Double(until) - Date().timeIntervalSince1970
            try? await Task.sleep(for: .seconds(max(left, 0) + 1))
            if !Task.isCancelled { await model.refreshAutopilot() }
        }
    }

    private func pick(_ mode: AutopilotMode, _ s: AutopilotSettings) {
        switch mode {
        case .bypass: bypassAsk = true
        case .lockdown where s.mode != .lockdown: lockdownAsk = true
        default: if mode != s.mode { Task { await ap.setMode(mode) } }
        }
    }

    private func download() {
        if ap.needsMobileDataConsent() { mobileDataAsk = true } else { ap.download() }
    }
}

/// A group whose content draws its own surface (the mode picker).
private struct AutopilotGroupHeaderless<Content: View>: View {
    var header: String
    var footer: String
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader(header).padding(.horizontal, 12)
            content
            GroupFooter(footer).padding(.horizontal, 12)
        }
    }
}

/// The mode in force, large: its tile and colour, what it means right now, and the way out of Bypass or Lockdown.
private struct ModeHero: View {
    var settings: AutopilotSettings
    var modelReady: Bool
    var onStopBypass: () -> Void
    var onEndLockdown: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let mode = settings.mode
        let tint = AutopilotStyle.tint(mode)
        VStack(spacing: 0) {
            if mode == .bypass, let until = settings.bypassUntil {
                BypassClock(until: until)
            } else {
                IconTile(symbol: AutopilotText.symbol(mode), tint: tint, size: 64, filled: true)
            }
            Text(AutopilotText.name(mode))
                .font(RFont.sans(28, .semibold))
                .foregroundStyle(Palette.text)
                .padding(.top, 14)
                .accessibilityIdentifier("heroMode")
            Text(AutopilotText.heroLine(mode, modelReady: modelReady))
                .font(RFont.sans(15))
                .foregroundStyle(Palette.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 4)
            switch mode {
            case .bypass:
                ActionButton(title: "Stop bypass", symbol: "stop.fill", kind: .destructive, compact: true, action: onStopBypass)
                    .padding(.top, 16)
                    .accessibilityIdentifier("stopBypass")
            case .lockdown:
                ActionButton(title: "End lockdown", symbol: "lock.open.fill", kind: .secondary, compact: true, action: onEndLockdown)
                    .padding(.top, 16)
                    .accessibilityIdentifier("endLockdown")
            default:
                EmptyView()
            }
        }
        .id(mode)
        .transition(.opacity.combined(with: .scale(scale: 0.96)))
        .padding(20)
        .frame(maxWidth: .infinity)
        .background {
            RoundedRectangle(cornerRadius: 26, style: .continuous)
                .fill(Palette.elevated)
                .overlay(
                    RoundedRectangle(cornerRadius: 26, style: .continuous)
                        .fill(LinearGradient(colors: [tint.opacity(scheme == .dark ? 0.22 : 0.12), .clear], startPoint: .top, endPoint: .bottom))
                )
        }
        .overlay(RoundedRectangle(cornerRadius: 26, style: .continuous).strokeBorder(tint.opacity(0.22), lineWidth: 0.75))
        .animation(.spring(response: 0.4, dampingFraction: 0.8), value: mode)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("modeHero")
    }
}

/// The on-device model: what it is, whether it is here, getting it and removing it, and the Wi-Fi only switch.
private struct ModelCard: View {
    var ap: AutopilotModel
    var wifiOnly: Bool
    var onDownload: () -> Void
    var onDelete: () -> Void

    var body: some View {
        AutopilotGroup(header: "On-device model", footer: "Runs on this phone, nothing leaves it. Without it, Assisted and Auto simply leave every request to you.") {
            if let m = ap.model {
                details(m)
                RowDivider(inset: 64)
            }
            WifiOnlyRow(on: wifiOnly) { on in Task { await ap.setWifiOnly(on) } }
        }
    }

    private func details(_ m: ModelStatus) -> some View {
        let downloading = m.state == .downloading || ap.downloadJob == .running
        let waiting = ap.downloadJob == .waiting && m.state != .downloading
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 14) {
                IconTile(symbol: "cpu", tint: Palette.accent, size: 46, filled: m.state == .installed)
                VStack(alignment: .leading, spacing: 2) {
                    Text(m.label).font(RFont.sans(16.5, .semibold)).foregroundStyle(Palette.text).lineLimit(3)
                    Text("Version \(m.version) · \(AutopilotText.modelSize(m))")
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.secondary)
                        .lineLimit(2)
                }
                .layoutPriority(1)
                Spacer(minLength: 8)
                if !downloading {
                    if waiting {
                        StatusPill(text: "Waiting", tint: Palette.warning)
                    } else if m.state == .installed {
                        StatusPill(text: "Installed", tint: Palette.success).accessibilityIdentifier("modelInstalled")
                    } else if m.state == .failed {
                        StatusPill(text: "Failed", tint: Palette.danger)
                    }
                }
            }
            Text(AutopilotText.modelState(m, waitingForNetwork: waiting, wifiOnly: ap.waitingForWifi))
                .font(RFont.sans(13.5, .medium))
                .foregroundStyle(m.state == .failed ? Palette.danger : Palette.secondary)
                .padding(.top, 12)
                .accessibilityIdentifier("modelState")
            if downloading {
                MeterBar(fraction: AutopilotText.fraction(m), tint: Palette.accent, height: 8)
                    .padding(.top, 10)
                    .accessibilityIdentifier("modelProgress")
                    .accessibilityLabel("Downloading")
                    .accessibilityValue(AutopilotText.fraction(m).map { "\(Int($0 * 100)) percent" } ?? "")
            }
            if m.state == .failed {
                IntegrationBanner(text: AutopilotText.modelError(m.error), kind: .error)
                    .padding(.top, 12)
                    .accessibilityIdentifier("modelError")
            }
            Group {
                if downloading {
                    EmptyView()
                } else if waiting {
                    ActionButton(title: "Cancel", kind: .secondary, compact: true) { ap.cancelDownload() }
                        .accessibilityIdentifier("cancelDownload")
                        .padding(.top, 14)
                } else if m.state == .installed {
                    ActionButton(title: "Delete model", symbol: "trash", kind: .destructive, compact: true, action: onDelete)
                        .accessibilityIdentifier("deleteModel")
                        .padding(.top, 14)
                } else {
                    ActionButton(
                        title: m.state == .failed ? "Try again" : "Download",
                        symbol: m.state == .failed ? "arrow.clockwise" : "arrow.down.circle",
                        kind: .accent,
                        action: onDownload
                    )
                    .accessibilityIdentifier("downloadModel")
                    .padding(.top, 14)
                }
            }
        }
        .padding(16)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("modelCard")
    }
}

/// "Download on Wi-Fi only", a switch row.
private struct WifiOnlyRow: View {
    var on: Bool
    var onChange: (Bool) -> Void

    var body: some View {
        Toggle(isOn: Binding(get: { on }, set: onChange)) {
            HStack(spacing: 14) {
                IconTile(symbol: "wifi", tint: Palette.accent, size: 34)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Download on Wi-Fi only").font(RFont.sans(16, .medium)).foregroundStyle(Palette.text)
                    Text("The model is a few hundred megabytes").font(RFont.sans(13)).foregroundStyle(Palette.secondary)
                }
            }
        }
        .tint(Palette.accent)
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .accessibilityIdentifier("wifiOnly")
    }
}

/// A profile: its emoji, name, what it knows, and a ring of how many of its kinds run on their own.
private struct ProfileRow: View {
    var profile: ProfileView
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            feedback.play(.tap)
            action()
        } label: {
            HStack(spacing: 14) {
                ProfileAvatar(profile: profile, size: 40)
                VStack(alignment: .leading, spacing: 2) {
                    Text(untrusted(profile.name)).font(RFont.sans(16, .medium)).foregroundStyle(Palette.text).lineLimit(1)
                    Text(AutopilotText.profileSummary(profile)).font(RFont.sans(13)).foregroundStyle(Palette.secondary).lineLimit(1)
                }
                Spacer(minLength: 6)
                if !profile.classes.isEmpty {
                    ProgressRing(
                        fraction: Double(profile.classes.filter(\.autoApprove).count) / Double(profile.classes.count),
                        tint: Palette.accent, size: 26, stroke: 3
                    )
                }
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Palette.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowPress())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("profile:\(profile.id)")
    }
}
