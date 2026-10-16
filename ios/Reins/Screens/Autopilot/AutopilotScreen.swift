import SwiftUI

// Autopilot's parts of Trust (TrustScreen): the mode, this week's numbers, the model and its sheet, "Try it" and the
// profiles.

/// The mode tiles (iOS-Trust.png, 01): Manual and Auto side by side (the one in force ringed, violet for Auto), then
/// Lock down and Bypass…, then Overnight…. While a bypass or an Overnight runs, the field above Trust has them; a
/// lockdown keeps the tiles, to pick what follows it, and the field ends it.
struct ModeHero: View {
    var settings: AutopilotSettings
    var modelReady: Bool
    var onPick: (AutopilotMode) -> Void
    var onBypass: () -> Void
    var onOvernight: () -> Void
    var onLockdown: () -> Void
    @Environment(\.feedback) private var feedback

    private var mode: AutopilotMode { settings.mode }

    var body: some View {
        if let running = TrustRunning.of(settings), running != .lockdown {
            EmptyView()
        } else {
            VStack(spacing: 10) {
                HStack(spacing: 8) {
                    ModeTile(mode: .manual, selected: mode == .manual, dimmed: false) { onPick(.manual) }
                    ModeTile(mode: .auto, selected: mode == .auto, dimmed: !modelReady) { onPick(.auto) }
                }
                .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 8) {
                    Button {
                        onLockdown()
                    } label: {
                        Label(mode == .lockdown ? "Locked down" : "Lock down", systemImage: "lock.fill")
                    }
                    .buttonStyle(SignalButtonStyle(.soft, size: .small))
                    .disabled(mode == .lockdown)
                    .accessibilityIdentifier("mode:lockdown")
                    Button {
                        feedback.play(.tap)
                        onBypass()
                    } label: {
                        Label("Bypass…", systemImage: "bolt.fill")
                    }
                    .buttonStyle(SignalButtonStyle(.danger, size: .small))
                    .accessibilityIdentifier("mode:bypass")
                }
                Button {
                    feedback.play(.tap)
                    onOvernight()
                } label: {
                    HStack(spacing: 8) {
                        GlyphIcon(.moon, size: 18)
                        Text("\(AutopilotText.overnightName)…").font(RFont.sans(14.5, .semibold))
                    }
                    .foregroundStyle(Palette.violet)
                    .frame(maxWidth: .infinity, minHeight: 40)
                    .background(Palette.violet.opacity(0.12), in: Capsule())
                    .contentShape(Capsule())
                }
                .buttonStyle(PressDim())
                .accessibilityIdentifier("mode:overnight")
            }
            .animation(.spring(response: 0.4, dampingFraction: 0.85), value: mode)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("modeHero")
        }
    }
}

/// Manual or Auto as a tile: its glyph, its name in the serif, what it does; the one in force has a 2 pt ring (violet
/// for Auto, with its check). Auto without the model is faded, still there to tap.
private struct ModeTile: View {
    var mode: AutopilotMode
    var selected: Bool
    var dimmed: Bool
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        let auto = mode == .auto
        let accent = auto ? Palette.violet : Palette.ink
        let shape = RoundedRectangle(cornerRadius: 22, style: .continuous)
        Button {
            feedback.play(.selection)
            action()
        } label: {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    GlyphIcon(auto ? .autopilot : .person, size: 20).foregroundStyle(accent)
                    Spacer(minLength: 0)
                    if selected && auto { GlyphIcon(.check, size: 18).foregroundStyle(Palette.violet) }
                }
                Text(AutopilotText.name(mode))
                    .font(RFont.serif(26))
                    .foregroundStyle(Palette.ink)
                    .padding(.top, 8)
                Text(dimmed ? "Needs the model" : auto ? AutopilotText.cardLine(mode) : "You decide, with a hint")
                    .textRole(.meta)
                    .foregroundStyle(Palette.inkMeta)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .padding(14)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(Palette.paperRaised, in: shape)
            .overlay(shape.strokeBorder(selected ? accent : Palette.rule, lineWidth: selected ? 2 : 1))
            .opacity(dimmed ? 0.5 : 1)
            .contentShape(shape)
        }
        .buttonStyle(ModeCardPress())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityHint(dimmed ? "Needs the on-device model" : "")
        .accessibilityIdentifier("mode:\(AutopilotText.key(mode))")
    }
}

/// This week in three numbers, in the serif between two hairlines: requests scored, how often Autopilot matched the
/// user, kinds on Auto.
struct WeekStats: View {
    var stats: AutopilotStats?

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            stat(stats.map { "\($0.scored)" } ?? "—", "scored this week").accessibilityIdentifier("stat:scored")
            stat(AutopilotText.share(stats?.matched), "matched you", tint: stats?.matched == nil ? Palette.ink : Palette.success)
                .accessibilityIdentifier("stat:matched")
            stat(stats.map { "\($0.kindsOnAuto)" } ?? "—", "kinds on Auto").accessibilityIdentifier("stat:kinds")
        }
        .padding(.vertical, 12)
        .overlay(alignment: .top) { Rectangle().fill(Palette.rule).frame(height: 1) }
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
    }

    private func stat(_ value: String, _ label: String, tint: Color = Palette.ink) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(value).font(RFont.serif(32)).foregroundStyle(tint).contentTransition(.numericText()).lineLimit(1)
            Text(label).textRole(.meta).foregroundStyle(Palette.inkMeta).lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

/// Without the model: what it gives and getting it, with the download's progress while it comes.
struct ModelOffer: View {
    var ap: AutopilotModel
    var model: ModelStatus
    var onDownload: () -> Void

    var body: some View {
        let downloading = model.state == .downloading || ap.downloadJob == .running
        let waiting = ap.downloadJob == .waiting && !downloading
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                SettingsTile(symbol: "cpu", tint: Palette.kindRead)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Get Autopilot's score").font(RFont.sans(16, .semibold)).foregroundStyle(Palette.ink)
                    Text("\(AutopilotText.modelSize(model)) · runs on this phone")
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.inkMeta)
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                if !downloading && !waiting {
                    ActionButton(
                        title: model.state == .failed ? "Try again" : "Download",
                        symbol: model.state == .failed ? "arrow.clockwise" : "arrow.down.circle",
                        kind: .accent, compact: true, action: onDownload
                    )
                    .accessibilityIdentifier("downloadModel")
                }
            }
            if downloading {
                MeterBar(fraction: AutopilotText.fraction(model), tint: Palette.kindRead, height: 6)
                    .padding(.top, 14)
                    .accessibilityIdentifier("modelProgress")
                    .accessibilityLabel("Downloading")
                    .accessibilityValue(AutopilotText.fraction(model).map { "\(Int($0 * 100)) percent" } ?? "")
                Text(AutopilotText.progressText(model))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.ink2)
                    .padding(.top, 8)
                    .accessibilityIdentifier("modelState")
            } else if waiting {
                HStack {
                    Text(AutopilotText.modelState(model, waitingForNetwork: true, wifiOnly: ap.waitingForWifi))
                        .font(RFont.sans(12.5))
                        .foregroundStyle(Palette.ink2)
                        .accessibilityIdentifier("modelState")
                    Spacer()
                    Button("Cancel") { ap.cancelDownload() }
                        .font(RFont.sans(12.5, .medium))
                        .foregroundStyle(Palette.violet)
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("cancelDownload")
                }
                .padding(.top, 12)
            } else if model.state == .failed {
                Text(AutopilotText.modelError(model.error))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.danger)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 10)
                    .accessibilityIdentifier("modelError")
            }
        }
        .padding(16)
        .background(
            LinearGradient(colors: [Palette.kindRead.opacity(0.14), Palette.kindRead.opacity(0.03)], startPoint: .top, endPoint: .bottom),
            in: RoundedRectangle(cornerRadius: 24, style: .continuous)
        )
        .overlay(RoundedRectangle(cornerRadius: 24, style: .continuous).strokeBorder(Palette.kindRead.opacity(0.26), lineWidth: 1))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("modelOffer")
    }
}

/// "On-device model": its version and when it was updated ("v2 · Updated today"), the download's percent (a first
/// download or an update coming down by itself), or Download; opens the model's details.
struct ModelRow: View {
    var ap: AutopilotModel
    var action: () -> Void

    var body: some View {
        AutopilotLinkRow(title: "On-device model", symbol: "cpu", tint: Palette.kindRead, identifier: "modelRow") {
            if let m = ap.model {
                if m.state == .installed {
                    HStack(spacing: 5) {
                        Image(systemName: "checkmark.circle").font(.system(size: 14, weight: .medium))
                            .foregroundStyle(Palette.success)
                        Text(AutopilotText.modelInstalledLine(m))
                            .font(RFont.sans(15))
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(1)
                    }
                    .accessibilityIdentifier("modelInstalled")
                } else if m.state == .downloading || ap.downloadJob == .running {
                    Text(AutopilotText.fraction(m).map { "\(Int($0 * 100))%" } ?? "…")
                        .font(RFont.mono(14))
                        .foregroundStyle(Palette.ink2)
                } else {
                    Text("Download").font(RFont.sans(15)).foregroundStyle(Palette.violet)
                }
            }
        } action: {
            action()
        }
    }
}

/// A row of the Autopilot list: a coloured tile, a title, what goes on the right, and the disclosure mark.
struct AutopilotLinkRow<Trailing: View>: View {
    var title: String
    var symbol: String
    var tint: Color
    var identifier: String
    @ViewBuilder var trailing: Trailing
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            feedback.play(.tap)
            action()
        } label: {
            SettingsRowLabel(title: title) {
                SettingsTile(symbol: symbol, tint: tint)
            } trailing: {
                trailing
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .buttonStyle(RowPress())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier(identifier)
    }
}

/// A profile: its emoji, name, the AIs it learns from and how much it remembers.
struct ProfileRow: View {
    var profile: ProfileView
    var line: String
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            feedback.play(.tap)
            action()
        } label: {
            HStack(spacing: 12) {
                ProfileTile(profile: profile, size: 36)
                VStack(alignment: .leading, spacing: 2) {
                    Text(untrusted(profile.name)).font(RFont.sans(16, .medium)).foregroundStyle(Palette.ink).lineLimit(1)
                    Text(line).font(RFont.sans(13)).foregroundStyle(Palette.inkMeta).lineLimit(1)
                }
                Spacer(minLength: 6)
                Chevron()
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowPress())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("profile:\(profile.id)")
    }
}

/// The model's details: what it is, getting it or removing it, the Wi-Fi only and Update automatically switches, and
/// "Check now" under More.
struct ModelSheet: View {
    var ap: AutopilotModel
    @State private var deleteAsk = false
    @State private var mobileDataAsk = false
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack(alignment: .center) {
                    AskText([AskText.Run(text: "On-device "), AskText.Run(text: "model", target: true)], role: .sheetTitle)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                    Spacer()
                    RoundHeaderButton(glyph: .close, label: "Close", size: 34) {
                        feedback.play(.tap)
                        dismiss()
                    }
                    .accessibilityIdentifier("closeModel")
                }
                ModelCard(ap: ap, wifiOnly: ap.settings?.wifiOnly ?? true, autoUpdate: ap.settings?.autoUpdate ?? true,
                          onDownload: download, onDelete: { deleteAsk = true })
                if AutopilotText.modelUsable(ap.model) {
                    ModelMore(ap: ap)
                }
            }
            .padding(20)
        }
        .scrollBounceBehavior(.basedOnSize)
        .background(Palette.paper.ignoresSafeArea())
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationSizing(.form)
        .accessibilityIdentifier("modelSheet")
        .alert("Delete the model?", isPresented: $deleteAsk) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                feedback.quietClose()
                Task { await ap.deleteModel() }
            }
        } message: {
            Text("Manual asks without hints and Auto pauses until you download it again.")
        }
        .presentationFeedback(deleteAsk)
        .mobileDataPrompt(isPresented: $mobileDataAsk, ap: ap)
    }

    private func download() {
        if ap.needsMobileDataConsent() { mobileDataAsk = true } else { ap.download() }
    }
}

/// The on-device model: what it is, whether it is here, getting it and removing it, and its two switches.
private struct ModelCard: View {
    var ap: AutopilotModel
    var wifiOnly: Bool
    var autoUpdate: Bool
    var onDownload: () -> Void
    var onDelete: () -> Void

    var body: some View {
        AutopilotGroup(header: nil, footer: "Runs on this phone, nothing leaves it. Without it, every request is left to you, without hints.") {
            if let m = ap.model {
                details(m)
                RowDivider(inset: 64)
            }
            ModelSwitchRow(title: "Download on Wi-Fi only", symbol: "wifi", on: wifiOnly, identifier: "wifiOnly") { on in
                Task { await ap.setWifiOnly(on) }
            }
            RowDivider(inset: 64)
            ModelSwitchRow(title: "Update automatically", symbol: "arrow.triangle.2.circlepath", on: autoUpdate, identifier: "autoUpdate") { on in
                Task { await ap.setAutoUpdate(on) }
            }
        }
    }

    private func details(_ m: ModelStatus) -> some View {
        let downloading = m.state == .downloading || ap.downloadJob == .running
        let waiting = ap.downloadJob == .waiting && m.state != .downloading
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 14) {
                IconTile(symbol: "cpu", tint: Palette.kindRead, size: 46, filled: m.state == .installed)
                VStack(alignment: .leading, spacing: 2) {
                    Text(m.label).font(RFont.sans(16.5, .semibold)).foregroundStyle(Palette.ink).lineLimit(3)
                    Text("Version \(m.version) · \(AutopilotText.modelSize(m))")
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.ink2)
                        .lineLimit(2)
                }
                .layoutPriority(1)
                Spacer(minLength: 8)
                if !downloading {
                    if waiting {
                        StatusPill(text: "Waiting", tint: Palette.warning)
                    } else if m.state == .installed {
                        StatusPill(text: "Installed", tint: Palette.success)
                    } else if m.state == .failed {
                        StatusPill(text: "Failed", tint: Palette.danger)
                    }
                }
            }
            Text(AutopilotText.modelState(m, waitingForNetwork: waiting, wifiOnly: ap.waitingForWifi))
                .font(RFont.sans(13.5, .medium))
                .foregroundStyle(m.state == .failed ? Palette.danger : Palette.ink2)
                .padding(.top, 12)
            if m.state == .installed, let ago = AutopilotText.updatedAgo(m.updatedAt) {
                Text(ago)
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.top, 4)
                    .accessibilityIdentifier("modelUpdated")
            }
            if downloading {
                MeterBar(fraction: AutopilotText.fraction(m), tint: Palette.violet, height: 8)
                    .padding(.top, 10)
                    .accessibilityLabel("Downloading")
                    .accessibilityValue(AutopilotText.fraction(m).map { "\(Int($0 * 100)) percent" } ?? "")
            }
            if m.state == .failed {
                IntegrationBanner(text: AutopilotText.modelError(m.error), kind: .error)
                    .padding(.top, 12)
            }
            Group {
                if downloading {
                    EmptyView()
                } else if waiting {
                    ActionButton(title: "Cancel", kind: .secondary, compact: true) { ap.cancelDownload() }
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
                    .accessibilityIdentifier("downloadModelDetails")
                    .padding(.top, 14)
                }
            }
        }
        .padding(16)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("modelCard")
    }
}

/// A switch row of the model's card ("Download on Wi-Fi only", "Update automatically").
private struct ModelSwitchRow: View {
    var title: String
    var symbol: String
    var on: Bool
    var identifier: String
    var onChange: (Bool) -> Void

    var body: some View {
        Toggle(isOn: Binding(get: { on }, set: onChange)) {
            HStack(spacing: 14) {
                IconTile(symbol: symbol, tint: Palette.violet, size: 34)
                Text(title).font(RFont.sans(16, .medium)).foregroundStyle(Palette.ink)
            }
        }
        .tint(Palette.violet)
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .accessibilityIdentifier(identifier)
    }
}

/// "More", closed at first: "Check now" and what it came to ("Up to date", "Updated", "Waiting for Wi-Fi").
private struct ModelMore: View {
    var ap: AutopilotModel
    @Environment(\.feedback) private var feedback
    @State private var open = false

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
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("modelMore")
            .accessibilityValue(open ? "Open" : "Closed")
            if open {
                HStack(spacing: 12) {
                    ActionButton(title: "Check now", symbol: "arrow.clockwise", kind: .secondary, busy: ap.checkingModel, compact: true) {
                        Task { await ap.checkModelNow() }
                    }
                    .accessibilityIdentifier("checkModelNow")
                    if let result = ap.modelCheckResult {
                        Text(result.text)
                            .font(RFont.sans(13.5, .medium))
                            .foregroundStyle(result.failed ? Palette.danger : Palette.ink2)
                            .lineLimit(3)
                            .accessibilityIdentifier("checkModelResult")
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 12)
                .transition(.opacity)
            }
        }
    }
}

extension View {
    /// Asks before downloading the model over mobile data while Wi-Fi only is on.
    func mobileDataPrompt(isPresented: Binding<Bool>, ap: AutopilotModel) -> some View {
        confirmationDialog("You are on mobile data", isPresented: isPresented, titleVisibility: .visible) {
            Button("Download on mobile data") { ap.downloadNow() }
            Button("Wait for Wi-Fi") { ap.download() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The model is \(ap.model.map(AutopilotText.modelSize) ?? "a few hundred MB"). Download it now over mobile data, or wait until this phone is on Wi-Fi?")
        }
        .presentationFeedback(isPresented.wrappedValue)
    }
}
