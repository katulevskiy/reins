import Observation
import SwiftUI

/// One guardianship as the ward sees it (decisions v2 P7, 04 §4): what it covers, the notice they chose, every
/// decision the guardian made with its note, the weekly summary switch (theirs alone), the guardian's ask to mark the
/// account a minor's, and Leave: after the notice (a countdown while the obligations stay), or, for a minor's account,
/// once the guardian agrees. Cooperative (08 R7): this phone holds to it, and leaving early is shown to the guardian.
@Observable
@MainActor
final class GuardianWardModel {
    let linkId: String
    private(set) var view: WardView?
    private(set) var busy = false
    var error: String?
    /// Asked when the ward accepts a minor's mark; `.unavailable` never blocks it.
    @ObservationIgnored var signals: any AgeSignals

    init(linkId: String, signals: any AgeSignals = DeclaredAgeSignals()) {
        self.linkId = linkId
        self.signals = signals
    }

    var leave: GuardianLogic.Leave? { view.map(GuardianLogic.leave) }

    func load(_ app: AppModel) async {
        let epoch = app.accountEpoch
        do {
            let next = try await app.core.guardianWardView(linkId: linkId)
            guard epoch == app.accountEpoch else { return }
            view = next
            error = nil
        } catch {
            guard epoch == app.accountEpoch else { return }
            self.error = error.userMessage
        }
    }

    /// Asks to end it: the notice starts (or the guardian is asked, for a minor's account).
    func requestLeave(_ app: AppModel) async {
        guard case .available = leave else { return }
        await run(app, cue: .revoked) { try await app.core.circleEnd(linkId: self.linkId) }
    }

    /// The ward's switch; the core refuses it from anyone else.
    func setDigest(_ on: Bool, _ app: AppModel) async {
        guard view?.digest != on else { return }
        view?.digest = on
        await run(app, cue: on ? .toggleOn : .toggleOff) { try await app.core.guardianSetDigest(linkId: self.linkId, on: on) }
    }

    /// Accepts the guardian's ask, with whatever the platform says about age (an adult's is refused by the core; none
    /// at all is the ward's own word).
    func acceptMinor(_ app: AppModel) async {
        guard let view, GuardianLogic.showsMinorCard(view), !busy else { return }
        busy = true
        let signal = await signals.check()
        busy = false
        // The core asks the owner (Face ID, Touch ID or the passcode) before it signs: a cancel marks nothing.
        await run(app, cue: .approved) {
            try await app.core.guardianAcceptMinor(linkId: self.linkId, signal: signal.input, owner: app.ownerCheck)
        }
    }

    private func run(_ app: AppModel, cue: FeedbackEvent, _ body: @escaping () async throws -> Void) async {
        guard !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        var failure: String?
        do {
            try await body()
            app.feedback.play(cue)
        } catch {
            app.feedback.play(.error)
            failure = error.userMessage
        }
        // Read again either way; what went wrong stays shown (the read clears it).
        await load(app)
        if let failure { error = failure }
        await app.circlesChanged()
    }
}

struct GuardianWardScreen: View {
    var person: CirclePerson
    var onClose: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm: GuardianWardModel
    @State private var confirmingLeave = false

    init(person: CirclePerson, signals: any AgeSignals = DeclaredAgeSignals(), onClose: @escaping () -> Void) {
        self.person = person
        self.onClose = onClose
        _vm = State(initialValue: GuardianWardModel(linkId: person.linkId, signals: signals))
    }

    private var guardian: String { vm.view?.guardian ?? person.peer }
    private var guardianName: String { CircleLogic.name(guardian) }

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("wardError") }
                    if let view = vm.view {
                        content(view)
                    } else {
                        ProgressView().frame(maxWidth: .infinity).padding(.vertical, 40)
                    }
                }
                .padding(.bottom, 24)
            }
            .scrollBounceBehavior(.basedOnSize)
            footer
        }
        .pageBackground()
        .presentationDetents([.large])
        .presentationCornerRadius(32)
        .task(id: model.circleTick) { await vm.load(model) }
        .confirmationDialog(
            "Leave \(guardianName)'s guardianship?",
            isPresented: $confirmingLeave,
            titleVisibility: .visible
        ) {
            Button("Leave", role: .destructive) {
                Task { await vm.requestLeave(model) }
            }
            .accessibilityIdentifier("wardConfirmLeave")
            Button("Cancel", role: .cancel) {}
        } message: {
            if let view = vm.view { Text(GuardianLogic.leaveMessage(view)) }
        }
        .presentationFeedback(confirmingLeave)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("guardianWardScreen")
    }

    private var header: some View {
        HStack(spacing: 8) {
            SymbolTile(symbol: "person.badge.shield.checkmark", tint: Palette.violet, side: 32)
            Text("Guardian")
                .textRole(.quietTitle)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
            HelpButton(text: "\(guardianName) must approve what's in scope. It's cooperative: your phone holds to it, and \(guardianName) is told if it ends early.")
                .accessibilityIdentifier("wardHelp")
            Spacer()
            RoundHeaderButton(glyph: .close, label: "Close", action: onClose)
                .accessibilityIdentifier("closeWard")
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .padding(.bottom, 6)
    }

    @ViewBuilder private func content(_ view: WardView) -> some View {
        FlatGroup(inset: 0) {
            CirclePersonLabel(email: view.guardian, size: 36)
                .padding(.vertical, 10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("wardGuardian")
            if !untrusted(view.scopeLine).isEmpty {
                fact(untrusted(view.scopeLine), symbol: "square.stack.3d.up")
            }
            GuardianNoticeFact(days: view.noticeDays)
            if view.minor {
                fact("Minor's account", value: "Leaving needs \(guardianName)'s OK", symbol: "figure.child")
                    .accessibilityIdentifier("wardMinor")
            }
        }
        .padding(.top, 12)

        if GuardianLogic.showsMinorCard(view) { minorCard }

        FlatGroup(inset: 0) {
            Toggle(isOn: Binding(get: { view.digest }, set: { on in Task { await vm.setDigest(on, model) } })) {
                HStack(spacing: 6) {
                    Text(GuardianLogic.digestLabel(view.guardian)).textRole(.row).foregroundStyle(Palette.ink)
                    HelpButton(text: GuardianLogic.digestHelp)
                }
            }
            .toggleStyle(SignalSwitchStyle())
            .disabled(vm.busy)
            .padding(.vertical, 12)
            .accessibilityIdentifier("wardDigest")
        }
        .padding(.top, 18)

        decisions(view.decisions)
    }

    /// The guardian asked to mark this account a minor's: Accept (the platform's age signal goes with it), or leave it.
    private var minorCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "figure.child").font(.system(size: 15, weight: .semibold)).foregroundStyle(Palette.violet)
                Text("\(guardianName) asks to mark this a minor's account")
                    .textRole(.row)
                    .foregroundStyle(Palette.ink)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                HelpButton(text: "Then leaving needs \(guardianName)'s OK. Your phone may confirm your age range if you share it.")
            }
            Button {
                Task { await vm.acceptMinor(model) }
            } label: {
                HStack(spacing: 8) {
                    if vm.busy { ProgressView() }
                    Text("Accept")
                }
            }
            .buttonStyle(SignalButtonStyle(.outline, size: .small, fill: false))
            .disabled(vm.busy)
            .accessibilityIdentifier("wardAcceptMinor")
        }
        .padding(16)
        .background(Palette.violet.opacity(0.08), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("wardMinorCard")
    }

    @ViewBuilder private func decisions(_ list: [GuardianDecisionView]) -> some View {
        SectionHeader("Decisions", quiet: true, trailing: list.isEmpty ? nil : "\(list.count)")
        FlatGroup(inset: 0) {
            if list.isEmpty {
                TileRow("None yet", chevron: false) { SymbolTile(symbol: "checkmark.seal") } trailing: { EmptyView() }
                    .flatRow()
                    .accessibilityIdentifier("wardNoDecisions")
            }
            ForEach(Array(list.sorted { $0.at > $1.at }.enumerated()), id: \.offset) { _, d in
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Text(verbatim: untrusted(d.title))
                            .textRole(.row)
                            .foregroundStyle(Palette.ink)
                            .lineLimit(2)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Pill(d.approved ? "Approved" : "Denied", tone: d.approved ? .success : .danger)
                    }
                    if let note = CircleLogic.note(d.note.map(untrusted)) {
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Image(systemName: "text.bubble").font(.system(size: 12, weight: .semibold))
                            Text(verbatim: note).fixedSize(horizontal: false, vertical: true)
                        }
                        .font(RFont.sans(13.5))
                        .foregroundStyle(Palette.ink2)
                    }
                    Text(TimeText.relative(d.at)).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                }
                .padding(.vertical, 10)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("wardDecision")
            }
        }
    }

    private func fact(_ title: String, value: String? = nil, symbol: String) -> some View {
        TileRow(title, value: value, chevron: false) { SymbolTile(symbol: symbol) } trailing: { EmptyView() }
            .flatRow()
    }

    // MARK: Leave

    @ViewBuilder private var footer: some View {
        if let leave = vm.leave {
            QuietFooter {
                switch leave {
                case .available:
                    Button {
                        confirmingLeave = true
                        feedback.play(.open)
                    } label: {
                        Label("Leave", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                    .buttonStyle(SignalButtonStyle(.outline))
                    .disabled(vm.busy)
                    .accessibilityIdentifier("wardLeave")
                case let .ending(at):
                    TimelineView(.periodic(from: .now, by: 60)) { context in
                        status("clock", GuardianLogic.endingLine(at, now: Int64(context.date.timeIntervalSince1970)))
                    }
                    .accessibilityIdentifier("wardEnding")
                case .waitingForConsent:
                    status("hourglass", GuardianLogic.waitingFor(guardian))
                        .accessibilityIdentifier("wardWaiting")
                }
            }
        }
    }

    private func status(_ symbol: String, _ text: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: symbol).font(.system(size: 15, weight: .semibold))
            Text(text)
        }
        .font(RFont.sans(15, .semibold))
        .foregroundStyle(Palette.ink2)
        .frame(maxWidth: .infinity, minHeight: 54)
        .accessibilityElement(children: .combine)
    }
}
