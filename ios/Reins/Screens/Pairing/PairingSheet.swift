import Observation
import SwiftUI

/// Connecting a new AI: tap the two-digit code the browser shows (one of three), name the connection, confirm with
/// Face ID, Touch ID or the passcode. The Rewarden desktop app also shows its key to compare.
struct PairingSheet: View {
    var pairingId: String
    @Environment(AppModel.self) private var model
    @State private var vm: PairingModel

    init(pairingId: String) {
        self.pairingId = pairingId
        _vm = State(initialValue: PairingModel(pairingId: pairingId))
    }

    init(model: PairingModel) {
        pairingId = model.pairingId
        _vm = State(initialValue: model)
    }

    var body: some View {
        Group {
            if let view = vm.view {
                PairingContent(vm: vm, view: view)
            } else {
                DecisionLoading(error: vm.loading ? nil : vm.error)
            }
        }
        .task { await vm.load(model) }
        .onChange(of: vm.finished) { _, done in if done { model.closeSheet() } }
    }
}

/// The pairing sheet's state (the Android app's PairingViewModel).
@Observable
@MainActor
final class PairingModel {
    static let maxLabel = 64
    let pairingId: String
    private(set) var loading = true
    private(set) var view: PairingView?
    /// The two-digit code the user tapped; the AI is connected only if it is the one their browser shows.
    var chosen: UInt8? {
        didSet { error = nil }
    }
    var label = "" {
        didSet { if label.count > Self.maxLabel { label = String(label.prefix(Self.maxLabel)) } }
    }
    private(set) var busy = false
    var error: String?
    private(set) var finished = false

    init(pairingId: String) {
        self.pairingId = pairingId
    }

    init(view: PairingView) {
        pairingId = view.id
        self.view = view
        label = String(untrusted(view.clientName).prefix(Self.maxLabel))
        loading = false
    }

    /// The three codes to pick from.
    var codes: [UInt8] { view.map { Array($0.choices) } ?? [] }

    func load(_ app: AppModel) async {
        guard view == nil else { return }
        do {
            let v = try await app.core.pairingView(pairingId: pairingId)
            view = v
            label = String(untrusted(v.clientName).prefix(Self.maxLabel))
            loading = false
        } catch {
            loading = false
            self.error = decisionErrorMessage(error)
        }
    }

    func approve(_ app: AppModel) async {
        guard !busy else { return }
        guard let code = chosen else {
            app.feedback.play(.error)
            error = "Tap the code your browser shows."
            return
        }
        busy = true
        error = nil
        let name = untrusted(view?.clientName ?? "")
        switch await OwnerCheck.confirm(app, reason: "Connect \(name.isEmpty ? "this AI" : name)") {
        case .confirmed:
            app.feedback.play(.connected)
            do {
                let trimmed = label.trimmingCharacters(in: .whitespacesAndNewlines)
                try await app.core.answerPairing(pairingId: pairingId, approve: true, chosenCode: code, label: trimmed.isEmpty ? nil : trimmed)
                await app.refreshPending()
                await app.refreshConnections()
                busy = false
                finished = true
            } catch {
                app.feedback.play(.error)
                busy = false
                self.error = decisionErrorMessage(error)
            }
        case .cancelled:
            busy = false
        case .unavailable:
            app.feedback.play(.error)
            busy = false
            error = OwnerCheck.unavailableMessage
        }
    }

    func deny(_ app: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        app.feedback.play(.denied)
        do {
            try await app.core.answerPairing(pairingId: pairingId, approve: false, chosenCode: nil, label: nil)
            await app.refreshPending()
            busy = false
            finished = true
        } catch {
            app.feedback.play(.error)
            busy = false
            self.error = decisionErrorMessage(error)
        }
    }
}

private struct PairingContent: View {
    @Bindable var vm: PairingModel
    var view: PairingView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @FocusState private var naming: Bool

    var body: some View {
        DecisionLayout {
            VStack(alignment: .leading, spacing: 0) {
                DecisionHeader(
                    connectionId: "",
                    label: view.clientName,
                    subtitle: untrusted(view.clientHost),
                    monoSubtitle: true,
                    kind: .pair,
                    title: "Connect",
                    known: false
                ) {
                    Text("Only continue if you just started this connection yourself. Tap the two-digit code that your browser is showing.")
                        .font(RFont.sans(15))
                        .foregroundStyle(Palette.secondary)
                        .padding(.top, 14)
                }
                if let fingerprint = view.keyFingerprint { DesktopKeyCard(fingerprint: fingerprint) }
                codes.padding(.horizontal, 16).padding(.vertical, 20)
                SectionHeader("Name this connection").padding(.leading, 20).padding(.bottom, 8)
                TextField("Name", text: $vm.label)
                    .font(RFont.sans(16))
                    .focused($naming)
                    .submitLabel(.done)
                    .textInputAutocapitalization(.words)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 14)
                    .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .padding(.horizontal, 16)
                    .accessibilityLabel("Name this connection")
                    .accessibilityIdentifier("label")
                if let error = vm.error {
                    Banner(error, kind: .error).padding(16).accessibilityIdentifier("pairingError")
                }
            }
        } recap: {
            Text("Connect \(untrusted(view.clientName))?").font(RFont.sans(22, .semibold)).foregroundStyle(Palette.text)
        } decision: {
            DecisionBar(
                approveTitle: "Connect",
                busy: vm.busy,
                approveEnabled: vm.chosen != nil,
                onDeny: { Task { await vm.deny(model) } },
                onApprove: {
                    naming = false
                    Task { await vm.approve(model) }
                }
            )
        }
        .accessibilityIdentifier("pairingSheet")
    }

    private var codes: some View {
        HStack(spacing: 12) {
            ForEach(vm.codes, id: \.self) { code in
                let label = String(format: "%02d", code)
                let picked = vm.chosen == code
                Button {
                    vm.chosen = code
                    feedback.play(.selection)
                } label: {
                    Text(label)
                        .font(RFont.mono(20, .semibold))
                        .frame(maxWidth: .infinity, minHeight: 56)
                }
                .buttonStyle(CapsuleButtonStyle(kind: picked ? .accent : .secondary, height: 56))
                .accessibilityLabel("Code \(label)")
                .accessibilityAddTraits(picked ? .isSelected : [])
                .accessibilityIdentifier("code:\(label)")
            }
        }
    }
}

/// The Rewarden desktop app's key, as eight digits the computer shows too. Matching them is what stops the server from
/// slipping in a key of its own, so they come first, large, and set apart like a permission request.
private struct DesktopKeyCard: View {
    var fingerprint: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "key.horizontal.fill").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.accent)
                Text("Desktop app key").font(RFont.sans(16, .semibold)).foregroundStyle(Palette.text)
            }
            Text(fingerprint)
                .font(RFont.mono(36, .semibold))
                .tracking(1.5)
                .foregroundStyle(Palette.text)
                .lineLimit(1)
                .minimumScaleFactor(0.6)
                .environment(\.layoutDirection, .leftToRight)
            Text("Check that your computer shows the same numbers. If they differ, deny.")
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.secondary)
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.accent.opacity(0.09), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(Palette.accent.opacity(0.55), lineWidth: 1.5))
        .padding(.horizontal, 16)
        .padding(.top, 18)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("keyFingerprint")
    }
}

#Preview("Desktop app") {
    PreviewHost { PairingSheet(pairingId: "pair2") }
}
