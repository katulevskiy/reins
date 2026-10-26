import SwiftUI

// Autopilot's parts of Trust (TrustScreen): what Auto needs before it runs, the model row's value and the model's
// sheet (the Autopilot board, 07).

/// Without the model, Auto pauses: one row under the mode picker that says so and gets it, with the download's
/// progress while it comes.
struct ModelOffer: View {
    var ap: AutopilotModel
    var model: ModelStatus
    var onDownload: () -> Void

    var body: some View {
        let downloading = model.state == .downloading || ap.downloadJob == .running
        let waiting = ap.downloadJob == .waiting && !downloading
        RaisedGroup {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    GlyphTile(.chip)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("Auto needs the model").textRole(.row).foregroundStyle(Palette.ink)
                        Text(AutopilotText.modelSize(model)).textRole(.sub).foregroundStyle(Palette.inkMeta).lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if !downloading && !waiting {
                        Button(model.state == .failed ? "Try again" : "Download", action: onDownload)
                            .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                            .fixedSize()
                            .accessibilityIdentifier("downloadModel")
                    }
                }
                if downloading {
                    MeterBar(fraction: AutopilotText.fraction(model), tint: Palette.violet, height: 4)
                        .padding(.top, 12)
                        .accessibilityIdentifier("modelProgress")
                        .accessibilityLabel("Downloading")
                        .accessibilityValue(AutopilotText.fraction(model).map { "\(Int($0 * 100)) percent" } ?? "")
                    Text(AutopilotText.progressText(model))
                        .textRole(.meta)
                        .foregroundStyle(Palette.ink2)
                        .padding(.top, 6)
                        .accessibilityIdentifier("modelState")
                } else if waiting {
                    HStack {
                        Text(AutopilotText.modelState(model, waitingForNetwork: true, wifiOnly: ap.waitingForWifi))
                            .textRole(.meta)
                            .foregroundStyle(Palette.ink2)
                            .accessibilityIdentifier("modelState")
                        Spacer()
                        Button("Cancel") { ap.cancelDownload() }
                            .font(RFont.sans(13, .semibold))
                            .foregroundStyle(Palette.violet)
                            .buttonStyle(.plain)
                            .frame(minHeight: 44)
                            .accessibilityIdentifier("cancelDownload")
                    }
                    .padding(.top, 6)
                } else if model.state == .failed {
                    Text(AutopilotText.modelError(model.error))
                        .textRole(.meta)
                        .foregroundStyle(Palette.danger)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 8)
                        .accessibilityIdentifier("modelError")
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("modelOffer")
    }
}

/// The model row's value on Trust: its version ("v2"), the download's percent, or Download.
struct ModelRowValue: View {
    var ap: AutopilotModel

    var body: some View {
        if let m = ap.model {
            if m.state == .installed {
                Text(AutopilotText.modelVersion(m))
                    .font(RFont.mono(13))
                    .foregroundStyle(Palette.ink2)
                    .fixedSize()
                    .accessibilityLabel(AutopilotText.modelVersionSpoken(m))
                    .accessibilityIdentifier("modelInstalled")
            } else if m.state == .downloading || ap.downloadJob == .running {
                Text(AutopilotText.fraction(m).map { "\(Int($0 * 100))%" } ?? "…")
                    .font(RFont.mono(13))
                    .foregroundStyle(Palette.ink2)
            } else {
                Text("Download").font(RFont.sans(15)).foregroundStyle(Palette.violet)
            }
        }
    }
}

/// The model (the Autopilot board, 07): what it is and whether it is ready, its version and size, deleting it (or
/// getting it), the Wi-Fi only and Auto-update switches, and "Check now" under More.
struct ModelSheet: View {
    var ap: AutopilotModel
    @State private var deleteAsk = false
    @State private var mobileDataAsk = false
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                TrustSheetTitle(title: "Model", closeID: "closeModel") {
                    feedback.play(.tap)
                    dismiss()
                }
                ModelCard(ap: ap, onDownload: download, onDelete: { deleteAsk = true })
                RaisedGroup(inset: 62) {
                    ModelSwitchRow(title: "Wi-Fi only", glyph: .download, on: ap.settings?.wifiOnly ?? true, identifier: "wifiOnly") { on in
                        Task { await ap.setWifiOnly(on) }
                    }
                    ModelSwitchRow(title: "Auto-update", glyph: .refresh, on: ap.settings?.autoUpdate ?? true, identifier: "autoUpdate") { on in
                        Task { await ap.setAutoUpdate(on) }
                    }
                }
                if AutopilotText.modelUsable(ap.model) {
                    ModelMore(ap: ap)
                }
            }
            .padding(.top, 20)
            .padding(.bottom, 16)
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

/// A sheet's title in the display face with its round close button (the kit's `sheet(..., title)`).
struct TrustSheetTitle: View {
    var title: String
    var closeID: String
    var onClose: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Text(title)
                .textRole(.cardAsk)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
                .frame(maxWidth: .infinity, alignment: .leading)
            RoundHeaderButton(glyph: .close, label: "Close", action: onClose)
                .accessibilityIdentifier(closeID)
        }
        .padding(.horizontal, 20)
        .padding(.bottom, 4)
    }
}

/// The model: its tile, name and state; its version and size; Delete, or Download while it is not here.
private struct ModelCard: View {
    var ap: AutopilotModel
    var onDownload: () -> Void
    var onDelete: () -> Void

    var body: some View {
        if let m = ap.model {
            let downloading = m.state == .downloading || ap.downloadJob == .running
            let waiting = ap.downloadJob == .waiting && m.state != .downloading
            RaisedGroup(inset: 62) {
                HStack(spacing: 12) {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(m.state == .installed ? Palette.violetFill : Palette.paperSunken)
                        .overlay { if m.state == .installed { Grain().clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous)) } }
                        .frame(width: 40, height: 40)
                        .overlay { GlyphIcon(.chip, size: 20).foregroundStyle(m.state == .installed ? Color.white : Palette.ink2) }
                        .accessibilityHidden(true)
                    Text(m.label).textRole(.row).foregroundStyle(Palette.ink).lineLimit(3)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if !downloading {
                        if waiting {
                            Pill("Waiting", tone: .warning)
                        } else if m.state == .installed {
                            Pill("Ready", tone: .success)
                        } else if m.state == .failed {
                            Pill("Failed", tone: .danger)
                        }
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .frame(minHeight: 60)

                VStack(alignment: .leading, spacing: 8) {
                    TileRow("\(AutopilotText.modelVersion(m)) · \(AutopilotText.modelSize(m))", glyph: .hardDrive, chevron: false) {
                        if m.state == .installed, let ago = AutopilotText.updatedAgo(m.updatedAt) {
                            Text(ago).textRole(.meta).foregroundStyle(Palette.inkMeta).lineLimit(1)
                                .accessibilityIdentifier("modelUpdated")
                        }
                    }
                    .font(RFont.mono(14.5))
                    if downloading {
                        MeterBar(fraction: AutopilotText.fraction(m), tint: Palette.violet, height: 4)
                            .padding(.horizontal, 14)
                            .padding(.bottom, 12)
                            .accessibilityLabel("Downloading")
                            .accessibilityValue(AutopilotText.fraction(m).map { "\(Int($0 * 100)) percent" } ?? "")
                    }
                    if m.state == .failed {
                        Text(AutopilotText.modelError(m.error))
                            .textRole(.meta)
                            .foregroundStyle(Palette.danger)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 14)
                            .padding(.bottom, 12)
                    }
                }

                if downloading {
                    EmptyView()
                } else if waiting {
                    Button { ap.cancelDownload() } label: {
                        TileRow(AutopilotText.modelState(m, waitingForNetwork: true, wifiOnly: ap.waitingForWifi), glyph: .close, chevron: false)
                    }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityIdentifier("cancelDownload")
                } else if m.state == .installed {
                    Button(action: onDelete) {
                        TileRow("Delete model", glyph: .trash, danger: true, chevron: false)
                    }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityIdentifier("deleteModel")
                } else {
                    Button(action: onDownload) {
                        TileRow(m.state == .failed ? "Try again" : "Download", glyph: .download, chevron: false)
                    }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityIdentifier("downloadModelDetails")
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("modelCard")
        }
    }
}

/// A switch row of the model's sheet ("Wi-Fi only", "Auto-update").
private struct ModelSwitchRow: View {
    var title: String
    var glyph: Glyph
    var on: Bool
    var identifier: String
    var onChange: (Bool) -> Void

    var body: some View {
        Toggle(isOn: Binding(get: { on }, set: onChange)) {
            HStack(spacing: 12) {
                GlyphTile(glyph)
                Text(title).textRole(.row).foregroundStyle(Palette.ink)
            }
        }
        .toggleStyle(SignalSwitchStyle())
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
        .frame(minHeight: 56)
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
                .padding(.horizontal, 24)
                .frame(minHeight: 44)
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
                .padding(.horizontal, 20)
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
