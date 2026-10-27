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

    /// "Connect a *computer*", "Connect an *AI app*": a laptop or a globe, with the link badge, on the pair field.
    static func line(_ view: PairingView) -> AskLine {
        let computer = computer(view)
        return AskLine(
            kind: label(view), lead: computer ? "Connect a " : "Connect an ", target: computer ? "computer" : "AI app",
            icon: AskIcon(mark: .glyph(computer ? .laptop : .globe), action: .link, kind: .pair)
        )
    }

    static func ask(_ view: PairingView) -> String { line(view).text }

    /// "Claude is connected already · name this one": a second account of the same AI.
    static func sameClient(_ view: PairingView) -> String {
        let name = untrustedName(view.clientName, limit: 32)
        return "\(name.isEmpty ? "This AI" : name) is connected already · name this one"
    }

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
    nonisolated static let maxLabel = 64
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
    /// The user changed the name field: only then is its name the user's (the core still refuses the client's own).
    private(set) var nameEdited = false

    /// The user typing in the name field.
    func edit(_ name: String) {
        label = name
        nameEdited = true
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
        label = Self.startingName(view)
        loading = false
    }

    /// The name the field starts with: the core's suggestion ("Claude 2" when the same AI is connected already), else
    /// the client's own name.
    nonisolated static func startingName(_ view: PairingView) -> String {
        untrustedName(view.suggestedName.isEmpty ? view.clientName : view.suggestedName, limit: maxLabel)
    }

    /// The same AI is connected already (a second account of it): the user names this one now.
    var sameClient: Bool { (view?.sameClient ?? 0) > 0 }

    /// The three codes to pick from.
    var codes: [UInt8] { view.map { Array($0.choices) } ?? [] }

    func load(_ app: AppModel) async {
        guard view == nil else { return }
        do {
            let v = try await app.core.pairingView(pairingId: pairingId)
            view = v
            label = Self.startingName(v)
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
                try await app.core.answerPairing(
                    pairingId: pairingId, approve: true, chosenCode: code, label: trimmed.isEmpty ? nil : trimmed, nameEdited: nameEdited
                )
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
            try await app.core.answerPairing(pairingId: pairingId, approve: false, chosenCode: nil, label: nil, nameEdited: false)
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
                ConnectField(kind: .pair, ask: PairWords.line(view)) {
                    if let fingerprint = view.keyFingerprint {
                        DesktopKey(fingerprint: fingerprint)
                    }
                    Text(view.keyFingerprint != nil ? "Started it yourself? Same on your computer? If not, deny." : "Only continue if you just started this.")
                        .font(RFont.sans(14, .medium))
                        .foregroundStyle(Palette.onField.opacity(0.88))
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 10)
                        .accessibilityIdentifier("pairWarning")
                }
                // What the other side calls itself: quoted, on rows of their own, never the ask.
                RaisedGroup(inset: 48) {
                    PairFact(glyph: PairWords.computer(view) ? .laptop : .globe, value: PairWords.quoted(view.clientName), valueId: "pairName")
                    let host = asciiHost(untrustedName(view.clientHost, limit: 64))
                    if !host.isEmpty { PairFact(glyph: .globe, value: host, mono: true, valueId: "pairHost") }
                    // Agents pair with one profile: the one this sign-in reached (switch in Settings for the other).
                    if WorkProfiles.hasWork {
                        PairFact(glyph: .briefcase, value: WorkProfiles.active.label, valueId: "pairProfile")
                    }
                }
                .padding(.top, 14)
                SectionHeader("Tap the code it shows")
                codes.padding(.horizontal, 16)
                SectionHeader("Name")
                if vm.sameClient {
                    // Two accounts of one AI look the same to Reins: the name is what tells them apart.
                    Label { Text(PairWords.sameClient(view)) } icon: { GlyphIcon(.person, size: 14) }
                        .font(RFont.sans(13.5))
                        .foregroundStyle(Palette.violet)
                        .padding(.horizontal, 24)
                        .padding(.bottom, 8)
                        .accessibilityIdentifier("pairSameClient")
                }
                RaisedFields {
                    GlyphInputRow(glyph: .pencil) {
                        TextField(untrustedName(view.clientName, limit: 48), text: Binding(get: { vm.label }, set: { vm.edit($0) }))
                            .focused($naming)
                            .submitLabel(.done)
                            .textInputAutocapitalization(.words)
                            .accessibilityLabel("Name this connection")
                            .accessibilityIdentifier("label")
                    }
                }
                if let note = StartingRuleText.onPairing(model.startingPolicy) {
                    Text(note)
                        .textRole(.sub)
                        .foregroundStyle(Palette.inkMeta)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.horizontal, 24)
                        .padding(.top, 10)
                        .accessibilityIdentifier("startingRuleNote")
                }
                if let error = vm.error {
                    FormBanner(text: error).padding(16).accessibilityIdentifier("pairingError")
                }
            }
        } recap: {
            Text(PairWords.ask(view)).textRole(.cardAsk).foregroundStyle(Palette.ink)
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
        .presentationDetents([.large])
        .presentationDragIndicator(.hidden)
        .presentationCornerRadius(32)
        .accessibilityContainer("pairingSheet")
        // A second account of an AI already connected: the cursor waits in the name field, "Claude 2" selected to type over.
        .task(id: vm.sameClient) { if vm.sameClient { naming = true } }
    }

    private var codes: some View {
        HStack(spacing: 10) {
            ForEach(vm.codes, id: \.self) { code in
                let label = String(format: "%02d", code)
                let picked = vm.chosen == code
                Button {
                    vm.chosen = code
                    feedback.play(.selection)
                } label: {
                    Text(label).font(RFont.mono(19, .medium)).frame(maxWidth: .infinity)
                }
                .buttonStyle(SignalButtonLook(kind: picked ? .violet : .outline, height: 48))
                .accessibilityLabel("Code \(label)")
                .accessibilityAddTraits(picked ? .isSelected : [])
                .accessibilityIdentifier("code:\(label)")
            }
        }
    }
}

/// The top of a pairing or joining sheet (the Connect board, 03 and 04): its field running into the sheet's top, the
/// request icon Reins picked and the close button, Reins' own sentence, then what to compare. Reins alone picks the
/// field and the words: a name the other side sent never goes here, only quoted on a row of its own below.
struct ConnectField<Extra: View>: View {
    var kind: FieldKind
    var ask: AskLine
    @ViewBuilder var extra: Extra
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top) {
                RequestIcon(spec: ask.icon, size: 40, ring: kind.fill)
                    .accessibilityIdentifier("requestIcon")
                Spacer()
                RoundHeaderButton(glyph: .close, label: "Close", onField: true) { model.closeSheet() }
                    .accessibilityIdentifier("closeSheet")
            }
            ask.runs.reduce(Text(verbatim: "")) { sentence, run in
                Text("\(sentence)\(Text(verbatim: run.text).font(RFont.display(26.5, target: run.target)))")
            }
                .tracking(RFont.displayTracking(26.5))
                .foregroundStyle(Palette.onField)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 12)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(ask.text)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("what")
            extra
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .padding(.bottom, 22)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(FieldSurface(kind: kind))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("hero")
    }
}

/// A fact with no key (the kit's `fact`): its glyph says what it is, the value beside it. Names from the other side
/// arrive quoted, cleaned and capped; hosts in mono.
struct PairFact: View {
    var glyph: Glyph
    var value: String
    var mono = false
    var valueId: String?

    var body: some View {
        HStack(spacing: 12) {
            GlyphIcon(glyph, size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20)
            Text(verbatim: value)
                .font(mono ? RFont.mono(14) : RFont.sans(15.5, .medium))
                .foregroundStyle(Palette.ink)
                .lineLimit(1)
                .truncationMode(mono ? .middle : .tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                .environment(\.layoutDirection, .leftToRight)
                .accessibilityIdentifier(valueId ?? "")
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 50)
        .accessibilityElement(children: .combine)
    }
}

/// The Reins desktop app's key, as eight digits the computer shows too, big and mono on the field. They come from the
/// key this phone verified, not from anything the other side named; matching them is what stops the server from
/// slipping in a key of its own.
private struct DesktopKey: View {
    var fingerprint: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label { Text("Key") } icon: { GlyphIcon(.fingerprint, size: 15) }
                .labelStyle(SignalLabelStyle())
                .font(RFont.sans(13.5, .semibold))
                .foregroundStyle(Palette.onField.opacity(0.85))
            BigCode(code: fingerprint, size: 46)
        }
        .padding(.top, 22)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Key \(fingerprint)")
        .accessibilityIdentifier("keyFingerprint")
    }
}

#if DEBUG
#Preview("Desktop app") {
    PreviewHost { PairingSheet(pairingId: "pair2") }
}
#endif
