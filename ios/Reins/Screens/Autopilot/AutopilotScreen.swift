import SwiftUI

// Autopilot's parts of Trust (TrustScreen): the mode, this week's numbers, the model and its sheet, "Try it" and the
// profiles.

/// The mode, large. Manual and Auto side by side (the chosen one tinted), then Overnight, then Bypass and Lock down;
/// amber in Lockdown, where neither is chosen and the last button ends it. During a bypass: red, its ring, what it
/// does and Stop bypass; during an Overnight the same in night indigo, with what it does with what is too risky.
struct ModeHero: View {
    var settings: AutopilotSettings
    var modelReady: Bool
    var onPick: (AutopilotMode) -> Void
    var onBypass: () -> Void
    var onOvernight: () -> Void
    var onLockdown: () -> Void
    var onEndLockdown: () -> Void
    var onStopBypass: () -> Void
    var onStopOvernight: () -> Void
    var onWhenRisky: (OvernightRisky) -> Void
    @Environment(\.colorScheme) private var scheme
    @Environment(\.feedback) private var feedback

    private var mode: AutopilotMode { settings.mode }

    /// The colour the whole card takes on: red in a bypass, night indigo in an Overnight, amber in Lockdown.
    private var wash: Color? {
        switch mode {
        case .bypass: Palette.danger
        case .overnight: settings.overnightUntil != nil ? Palette.overnight : nil
        case .lockdown: Palette.warning
        default: nil
        }
    }

    var body: some View {
        Group {
            if mode == .bypass, let until = settings.bypassUntil {
                bypass(until: until)
            } else if mode == .overnight, let until = settings.overnightUntil {
                overnight(until: until)
            } else {
                choices
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity)
        .background {
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .fill(Palette.elevated)
                .overlay {
                    if let wash {
                        RoundedRectangle(cornerRadius: 28, style: .continuous)
                            .fill(LinearGradient(
                                colors: [wash.opacity(scheme == .dark ? 0.2 : 0.12), wash.opacity(0.05)],
                                startPoint: .top, endPoint: .bottom
                            ))
                    }
                }
        }
        .overlay(
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .strokeBorder(wash.map { $0.opacity(0.32) } ?? Palette.hairline, lineWidth: 1)
        )
        .animation(.spring(response: 0.4, dampingFraction: 0.85), value: mode)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("modeHero")
    }

    private var choices: some View {
        VStack(spacing: 10) {
            HStack(spacing: 8) {
                ModeCard(mode: .manual, selected: mode == .manual, dimmed: false) { onPick(.manual) }
                ModeCard(mode: .auto, selected: mode == .auto, dimmed: !modelReady) { onPick(.auto) }
            }
            HeroButton(
                title: AutopilotText.overnightName, symbol: AutopilotText.overnightSymbol, fill: Palette.overnight.opacity(0.13),
                text: Palette.overnight, identifier: "mode:overnight"
            ) {
                feedback.play(.tap)
                onOvernight()
            }
            HStack(spacing: 8) {
                HeroButton(title: "Bypass", symbol: "bolt.fill", fill: Palette.danger.opacity(0.12), text: Palette.danger, identifier: "mode:bypass") {
                    feedback.play(.tap)
                    onBypass()
                }
                if mode == .lockdown {
                    HeroButton(title: "End lockdown", symbol: "lock.open.fill", fill: Palette.warning.opacity(0.16), text: Palette.warning, identifier: "endLockdown", action: onEndLockdown)
                } else {
                    HeroButton(title: "Lock down", symbol: "lock.fill", fill: Palette.controlFill, text: Palette.text, identifier: "mode:lockdown", action: onLockdown)
                }
            }
        }
    }

    private func bypass(until: Int64) -> some View {
        VStack(spacing: 14) {
            HStack(spacing: 16) {
                BypassRing(until: until, since: settings.bypassSince, size: 76)
                VStack(alignment: .leading, spacing: 3) {
                    Text("Bypass")
                        .font(RFont.sans(22, .semibold))
                        .foregroundStyle(Palette.danger)
                        .accessibilityIdentifier("heroMode")
                    LiveClock(interval: AutopilotText.isUntilOff(until) ? 30 : 60) { now in
                        Text(AutopilotText.bypassLine(until: until, since: settings.bypassSince, now: Int64(now)))
                            .font(RFont.sans(13.5))
                            .foregroundStyle(Palette.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("bypassLine")
                    }
                }
                Spacer(minLength: 0)
            }
            // Words only, as everywhere a bypass is stopped.
            Button(action: onStopBypass) {
                Text("Stop bypass")
                    .font(RFont.sans(16, .semibold))
                    .foregroundStyle(Palette.dynamic(0xFFFFFF, 0x2A0606))
                    .frame(maxWidth: .infinity, minHeight: 46)
                    .background(Palette.danger, in: Capsule())
                    .contentShape(Capsule())
            }
            .buttonStyle(PressDim())
            .accessibilityIdentifier("stopBypass")
        }
    }

    private func overnight(until: Int64) -> some View {
        VStack(spacing: 14) {
            HStack(spacing: 16) {
                BypassRing(
                    until: until, since: settings.overnightSince, size: 76, tint: Palette.overnight,
                    name: AutopilotText.overnightName, identifier: "overnightClock"
                )
                VStack(alignment: .leading, spacing: 3) {
                    Text(AutopilotText.overnightName)
                        .font(RFont.sans(22, .semibold))
                        .foregroundStyle(Palette.overnight)
                        .accessibilityIdentifier("heroMode")
                    LiveClock(interval: AutopilotText.isUntilOff(until) ? 30 : 60) { now in
                        Text(AutopilotText.overnightLine(until: until, since: settings.overnightSince, now: Int64(now)))
                            .font(RFont.sans(13.5))
                            .foregroundStyle(Palette.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("overnightLine")
                    }
                }
                Spacer(minLength: 0)
            }
            WhenRiskyPicker(selected: settings.overnightWhenRisky, onPick: onWhenRisky)
            // Words only, as a bypass is stopped.
            Button(action: onStopOvernight) {
                Text("Stop \(AutopilotText.overnightName)")
                    .font(RFont.sans(16, .semibold))
                    .foregroundStyle(Palette.onField)
                    .frame(maxWidth: .infinity, minHeight: 46)
                    .background(Palette.violetFill, in: Capsule())
                    .contentShape(Capsule())
            }
            .buttonStyle(PressDim())
            .accessibilityIdentifier("stopOvernight")
        }
    }
}

/// Manual or Auto on the hero: its tile, name and what it does; tinted when chosen, faded (but still there to tap) for
/// Auto without the model.
private struct ModeCard: View {
    var mode: AutopilotMode
    var selected: Bool
    var dimmed: Bool
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        let tint = AutopilotStyle.tint(mode)
        Button {
            feedback.play(.selection)
            action()
        } label: {
            VStack(alignment: .leading, spacing: 0) {
                RoundedRectangle(cornerRadius: 13, style: .continuous)
                    .fill(tint.opacity(0.17))
                    .frame(width: 40, height: 40)
                    .overlay {
                        Image(systemName: AutopilotText.symbol(mode))
                            .font(.system(size: 18, weight: .semibold))
                            .foregroundStyle(tint)
                    }
                Text(AutopilotText.name(mode))
                    .font(RFont.sans(17, .semibold))
                    .foregroundStyle(Palette.text)
                    .padding(.top, 10)
                Text(dimmed ? "Needs the model" : AutopilotText.cardLine(mode))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 3)
                Spacer(minLength: 0)
            }
            .padding(14)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(
                selected ? tint.opacity(0.09) : Palette.dynamic(0xF7F7F9, 0x0D0D0F),
                in: RoundedRectangle(cornerRadius: 20, style: .continuous)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 20, style: .continuous)
                    .strokeBorder(selected ? tint : Palette.hairline, lineWidth: 1.5)
            )
            .opacity(dimmed ? 0.5 : 1)
            .contentShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        }
        .buttonStyle(ModeCardPress())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityHint(dimmed ? "Needs the on-device model" : "")
        .accessibilityIdentifier("mode:\(AutopilotText.key(mode))")
    }
}

/// Overnight, Bypass, Lock down and End lockdown: a pill in its colour.
private struct HeroButton: View {
    var title: String
    var symbol: String
    var fill: Color
    var text: Color
    var identifier: String
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol)
                .font(RFont.sans(15, .semibold))
                .foregroundStyle(text)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .frame(maxWidth: .infinity, minHeight: 40)
                .background(fill, in: Capsule())
                .contentShape(Capsule())
        }
        .buttonStyle(PressDim())
        .accessibilityIdentifier(identifier)
    }
}

/// This week in three numbers: requests scored, how often Autopilot matched the user, kinds on Auto.
struct WeekStats: View {
    var stats: AutopilotStats?

    var body: some View {
        HStack(spacing: 8) {
            StatTile(value: stats.map { "\($0.scored)" } ?? "—", label: "requests scored")
                .accessibilityIdentifier("stat:scored")
            StatTile(value: AutopilotText.share(stats?.matched), label: "matched you", tint: stats?.matched == nil ? Palette.text : Palette.success)
                .accessibilityIdentifier("stat:matched")
            StatTile(value: stats.map { "\($0.kindsOnAuto)" } ?? "—", label: "kinds on Auto")
                .accessibilityIdentifier("stat:kinds")
        }
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
                SettingsTile(symbol: "cpu", tint: Palette.search)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Get Autopilot's score").font(RFont.sans(16, .semibold)).foregroundStyle(Palette.text)
                    Text("\(AutopilotText.modelSize(model)) · runs on this phone")
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.tertiary)
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
                MeterBar(fraction: AutopilotText.fraction(model), tint: Palette.search, height: 6)
                    .padding(.top, 14)
                    .accessibilityIdentifier("modelProgress")
                    .accessibilityLabel("Downloading")
                    .accessibilityValue(AutopilotText.fraction(model).map { "\(Int($0 * 100)) percent" } ?? "")
                Text(AutopilotText.progressText(model))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.secondary)
                    .padding(.top, 8)
                    .accessibilityIdentifier("modelState")
            } else if waiting {
                HStack {
                    Text(AutopilotText.modelState(model, waitingForNetwork: true, wifiOnly: ap.waitingForWifi))
                        .font(RFont.sans(12.5))
                        .foregroundStyle(Palette.secondary)
                        .accessibilityIdentifier("modelState")
                    Spacer()
                    Button("Cancel") { ap.cancelDownload() }
                        .font(RFont.sans(12.5, .medium))
                        .foregroundStyle(Palette.accent)
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
            LinearGradient(colors: [Palette.search.opacity(0.14), Palette.search.opacity(0.03)], startPoint: .top, endPoint: .bottom),
            in: RoundedRectangle(cornerRadius: 24, style: .continuous)
        )
        .overlay(RoundedRectangle(cornerRadius: 24, style: .continuous).strokeBorder(Palette.search.opacity(0.26), lineWidth: 1))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("modelOffer")
    }
}

/// "On-device model": Ready, the download's percent, or Download; opens the model's details.
struct ModelRow: View {
    var ap: AutopilotModel
    var action: () -> Void

    var body: some View {
        AutopilotLinkRow(title: "On-device model", symbol: "cpu", tint: Palette.read, identifier: "modelRow") {
            if let m = ap.model {
                if m.state == .installed {
                    HStack(spacing: 5) {
                        Image(systemName: "checkmark.circle").font(.system(size: 14, weight: .medium))
                        Text("Ready").font(RFont.sans(15))
                    }
                    .foregroundStyle(Palette.success)
                    .accessibilityIdentifier("modelInstalled")
                } else if m.state == .downloading || ap.downloadJob == .running {
                    Text(AutopilotText.fraction(m).map { "\(Int($0 * 100))%" } ?? "…")
                        .font(RFont.mono(14))
                        .foregroundStyle(Palette.secondary)
                } else {
                    Text("Download").font(RFont.sans(15)).foregroundStyle(Palette.accent)
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
                    Text(untrusted(profile.name)).font(RFont.sans(16, .medium)).foregroundStyle(Palette.text).lineLimit(1)
                    Text(line).font(RFont.sans(13)).foregroundStyle(Palette.tertiary).lineLimit(1)
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

/// The model's details: what it is, getting it or removing it, and the Wi-Fi only switch.
struct ModelSheet: View {
    var ap: AutopilotModel
    @State private var deleteAsk = false
    @State private var mobileDataAsk = false
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("On-device model")
                        .font(RFont.sans(18, .semibold))
                        .foregroundStyle(Palette.text)
                        .accessibilityAddTraits(.isHeader)
                    Spacer()
                    Button {
                        feedback.play(.tap)
                        dismiss()
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 13, weight: .bold))
                            .foregroundStyle(Palette.secondary)
                            .frame(width: 30, height: 30)
                            .background(Palette.controlFill, in: Circle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Close")
                    .accessibilityIdentifier("closeModel")
                }
                ModelCard(ap: ap, wifiOnly: ap.settings?.wifiOnly ?? true, onDownload: download, onDelete: { deleteAsk = true })
            }
            .padding(20)
        }
        .scrollBounceBehavior(.basedOnSize)
        .background(Palette.background.ignoresSafeArea())
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

/// The on-device model: what it is, whether it is here, getting it and removing it, and the Wi-Fi only switch.
private struct ModelCard: View {
    var ap: AutopilotModel
    var wifiOnly: Bool
    var onDownload: () -> Void
    var onDelete: () -> Void

    var body: some View {
        AutopilotGroup(header: nil, footer: "Runs on this phone, nothing leaves it. Without it, every request is left to you, without hints.") {
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
                IconTile(symbol: "cpu", tint: Palette.read, size: 46, filled: m.state == .installed)
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
                        StatusPill(text: "Installed", tint: Palette.success)
                    } else if m.state == .failed {
                        StatusPill(text: "Failed", tint: Palette.danger)
                    }
                }
            }
            Text(AutopilotText.modelState(m, waitingForNetwork: waiting, wifiOnly: ap.waitingForWifi))
                .font(RFont.sans(13.5, .medium))
                .foregroundStyle(m.state == .failed ? Palette.danger : Palette.secondary)
                .padding(.top, 12)
            if downloading {
                MeterBar(fraction: AutopilotText.fraction(m), tint: Palette.accent, height: 8)
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

/// "Download on Wi-Fi only", a switch row.
private struct WifiOnlyRow: View {
    var on: Bool
    var onChange: (Bool) -> Void

    var body: some View {
        Toggle(isOn: Binding(get: { on }, set: onChange)) {
            HStack(spacing: 14) {
                IconTile(symbol: "wifi", tint: Palette.accent, size: 34)
                Text("Download on Wi-Fi only").font(RFont.sans(16, .medium)).foregroundStyle(Palette.text)
            }
        }
        .tint(Palette.accent)
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .accessibilityIdentifier("wifiOnly")
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
