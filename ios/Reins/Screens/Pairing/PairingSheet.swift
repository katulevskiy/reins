import Observation
import SwiftUI

/// Connecting a new AI or computer: tap the two-digit code it shows (one of three), name the connection, confirm with
/// Face ID, Touch ID or the passcode. The Reins desktop app also shows its key to compare.
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

/// A pairing's words (pure, unit-tested). The ask is Reins' own sentence; what the other side calls itself is only ever
/// quoted, on one short line.
enum PairWords {
    /// A computer shows its key's digits; an AI app on the web does not.
    static func computer(_ view: PairingView) -> Bool { view.keyFingerprint != nil }
    static func label(_ view: PairingView) -> String { computer(view) ? "Computer · Pair" : "AI app · Pair" }

    /// "Connect a *computer*", "Connect an *AI app*": a laptop or a link, with a plus, on the pair field.
    static func line(_ view: PairingView) -> AskLine {
        let computer = computer(view)
        return AskLine(
            kind: label(view), lead: computer ? "Connect a " : "Connect an ", target: computer ? "computer" : "AI app",
            icon: AskIcon(mark: .glyph(computer ? .laptop : .link), action: .plus, kind: .pair)
        )
    }

    static func ask(_ view: PairingView) -> String { line(view).text }

    /// “Gemini”: a name the other side sent, quoted, on one line, capped and cleaned; "No name" for none.
    static func quoted(_ name: String) -> String {
        let clean = untrustedName(name, limit: 48)
        return clean.isEmpty ? "No name" : "“\(clean)”"
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
    /// The two-digit code the user tapped; the AI is connected only if it is the one their computer or browser shows.
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
        label = untrustedName(view.clientName, limit: Self.maxLabel)
        loading = false
    }

    /// The three codes to pick from.
    var codes: [UInt8] { view.map { Array($0.choices) } ?? [] }

    func load(_ app: AppModel) async {
        guard view == nil else { return }
        do {
            let v = try await app.core.pairingView(pairingId: pairingId)
            view = v
            label = untrustedName(v.clientName, limit: Self.maxLabel)
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
            error = "Tap the code your computer or browser shows."
            return
        }
        busy = true
        error = nil
        let name = untrustedName(view?.clientName ?? "", limit: 40)
        let computer = view.map(PairWords.computer) ?? false
        switch await OwnerPrompt.confirm(app, reason: computer ? "Connect a computer" : "Connect an AI app") {
        case .confirmed:
            app.feedback.play(.connected)
            do {
                let trimmed = label.trimmingCharacters(in: .whitespacesAndNewlines)
                try await app.core.answerPairing(pairingId: pairingId, approve: true, chosenCode: code, label: trimmed.isEmpty ? nil : trimmed)
                await app.refreshPending()
                await app.refreshConnections()
                // A computer: say it worked, since the sheet that connected it closes now.
                if view?.keyFingerprint != nil {
                    app.notice = "\(trimmed.isEmpty ? name : untrustedName(trimmed, limit: 40)) is connected."
                }
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
            error = OwnerPrompt.unavailableMessage
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
        DecisionLayout(closeInField: true) {
            VStack(alignment: .leading, spacing: 0) {
                SheetField(kind: .pair, ask: PairWords.line(view))
                // What the other side calls itself: quoted, on rows of their own, never the ask.
                FactGroup {
                    FactRow(key: "Calls itself", value: PairWords.quoted(view.clientName), valueId: "pairName")
                    let host = asciiHost(untrustedName(view.clientHost, limit: 64))
                    if !host.isEmpty { FactRow(key: "From", value: host, mono: true, valueId: "pairHost") }
                    if let fingerprint = view.keyFingerprint { DesktopKeyRow(fingerprint: fingerprint) }
                }
                .padding(.top, 10)
                Text("Only continue if you just started this connection yourself. Tap the two-digit code shown on your computer or in your browser.")
                    .textRole(.body)
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 24)
                    .padding(.top, 16)
                codes.padding(.horizontal, 20).padding(.vertical, 18)
                Text("Name this connection").textRole(.label).foregroundStyle(Palette.inkMeta).padding(.leading, 24).padding(.bottom, 8)
                TextField("Name", text: $vm.label)
                    .font(RFont.sans(16))
                    .focused($naming)
                    .submitLabel(.done)
                    .textInputAutocapitalization(.words)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 14)
                    .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
                    .padding(.horizontal, 20)
                    .accessibilityLabel("Name this connection")
                    .accessibilityIdentifier("label")
                if let note = StartingRuleText.onPairing(model.startingPolicy) {
                    Text(note)
                        .font(RFont.sans(13.5))
                        .foregroundStyle(Palette.tertiary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.horizontal, 24)
                        .padding(.top, 12)
                        .accessibilityIdentifier("startingRuleNote")
                }
                if let error = vm.error {
                    Banner(error, kind: .error).padding(16).accessibilityIdentifier("pairingError")
                }
            }
        } recap: {
            Text(PairWords.ask(view)).font(RFont.serif(28)).foregroundStyle(Palette.ink)
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
        .accessibilityContainer("pairingSheet")
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
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SignalButtonLook(kind: picked ? .inverse : .outline, height: 56))
                .accessibilityLabel("Code \(label)")
                .accessibilityAddTraits(picked ? .isSelected : [])
                .accessibilityIdentifier("code:\(label)")
            }
        }
    }
}

/// The Reins desktop app's key, as eight digits the computer shows too, on a mono row of its own. They come from the
/// key this phone verified, not from anything the other side named; matching them is what stops the server from
/// slipping in a key of its own.
private struct DesktopKeyRow: View {
    var fingerprint: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Key").font(RFont.sans(15)).foregroundStyle(Palette.ink2)
            Text(fingerprint)
                .font(RFont.mono(34, .semibold))
                .tracking(1.5)
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
                .minimumScaleFactor(0.6)
                .environment(\.layoutDirection, .leftToRight)
            Text("Check that your computer shows the same numbers. If they differ, deny.")
                .textRole(.meta)
                .foregroundStyle(Palette.inkMeta)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("keyFingerprint")
    }
}

#if DEBUG
#Preview("Desktop app") {
    PreviewHost { PairingSheet(pairingId: "pair2") }
}
#endif
