import Observation
import SwiftUI
import UIKit

/// Settings › Circles › Add someone (04 §3): the role, what it covers, Instant or Deferred; then the link (shared, or
/// shown as a QR code), the six digits both phones show once they opened it, and, after they accepted, Confirm held and
/// checked with Face ID, Touch ID or the passcode, as loosening Autopilot is. A guardianship's ward (this account)
/// picks the notice to leave (0, 3 or 7 days) before the link, so the guardian sees it before accepting.
@Observable
@MainActor
final class AddCircleModel {
    enum Step: Equatable {
        case role, scope, timing
        /// The link is out; nobody opened it yet.
        case link
        /// They answered: compare the code while they decide.
        case compare(code: String, helper: String)
        /// They accepted: compare once more, then confirm.
        case accepted(code: String, helper: String)
        case done(CirclePerson)
        /// Declined, expired or withdrawn.
        case closed(String)
    }

    var step: Step = .role
    var role: CircleRole = .delegate
    var preset: CircleScopePreset = .everything
    var deferred = true
    /// A guardianship's notice, the ward's pick; none until chosen (the link waits for it).
    var noticeDays: UInt8?
    private(set) var start: CircleInviteStart?
    private(set) var busy = false
    var error: String?
    /// How long to wait between asks while the other phone has not moved.
    var pollInterval: Duration = .milliseconds(1500)

    var draft: CircleDraft { GuardianLogic.draft(role: role, preset: preset, deferred: deferred, noticeDays: noticeDays) }

    var canMakeLink: Bool { !busy && GuardianLogic.canMakeLink(role: role, noticeDays: noticeDays) }

    func back() {
        switch step {
        case .scope: step = .role
        case .timing: step = .scope
        default: break
        }
    }

    func create(_ app: AppModel) async {
        guard canMakeLink, start == nil else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            start = try await app.core.circleInviteCreate(draft: draft)
            step = .link
            app.feedback.play(.open)
        } catch {
            app.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    /// Asks where the invitation stands until it is settled (or the sheet goes).
    func poll(_ app: AppModel) async {
        guard let id = start?.id else { return }
        while !Task.isCancelled {
            switch step {
            case .done, .closed: return
            default: break
            }
            do {
                let progress = try await app.core.circleInvitePoll(id: id)
                guard !Task.isCancelled else { return }
                apply(progress, app)
                error = nil
            } catch {
                if case CoreError.Network = error {} else { self.error = error.userMessage }
            }
            try? await Task.sleep(for: pollInterval)
        }
    }

    private func apply(_ progress: CircleInviteProgress, _ app: AppModel) {
        switch progress {
        case .waiting: break
        case let .compare(code, helper):
            if step != .compare(code: code, helper: helper) {
                step = .compare(code: code, helper: helper)
                app.feedback.play(.requestArrived)
            }
        case let .accepted(code, helper):
            if step != .accepted(code: code, helper: helper) {
                step = .accepted(code: code, helper: helper)
                app.feedback.play(.requestArrived)
            }
        case .active:
            // Confirmed elsewhere (another try of this sheet): it runs.
            if case .accepted = step {} else { step = .closed("Already set up.") }
        case .declined: step = .closed("They declined.")
        case .expired: step = .closed("The invitation expired.")
        case .cancelled: step = .closed("The invitation was withdrawn.")
        }
    }

    /// After the hold: the owner check, then the core activates the arrangement.
    func confirm(_ app: AppModel) async {
        guard let id = start?.id, case let .accepted(_, helper) = step, !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        let reason = "Add \(CircleLogic.name(helper)) as \(CircleLogic.roleName(role))"
        switch await OwnerPrompt.confirm(app, reason: reason) {
        case .confirmed: break
        case .cancelled: return
        case .unavailable:
            app.feedback.play(.error)
            error = OwnerPrompt.unavailableMessage
            return
        }
        do {
            let person = try await app.core.circleInviteConfirm(id: id)
            app.feedback.play(.connected)
            step = .done(person)
        } catch {
            app.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    /// Withdraws a link that is out and not yet active (closing the sheet does).
    func cancel(_ app: AppModel) async {
        guard let id = start?.id else { return }
        switch step {
        case .done, .closed: return
        default: try? await app.core.circleInviteCancel(id: id)
        }
    }
}

struct AddCircleSheet: View {
    var onClose: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm = AddCircleModel()
    @State private var pickingAgent = false

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    content
                    if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("addCircleError") }
                }
                .padding(.bottom, 24)
            }
            .scrollBounceBehavior(.basedOnSize)
            footer
        }
        .pageBackground()
        .presentationDetents([.large])
        .presentationCornerRadius(32)
        .interactiveDismissDisabled(vm.busy)
        .task(id: vm.start?.id) { await vm.poll(model) }
        .onDisappear { Task { await vm.cancel(model) } }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("addCircleSheet")
    }

    private var title: String {
        switch vm.step {
        case .role: "Add someone"
        case .scope: "What it covers"
        case .timing: "How soon"
        case .link: "Send the link"
        case .compare, .accepted: "Compare the code"
        case .done: "Added"
        case .closed: "Not added"
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            if [.scope, .timing].contains(vm.step) {
                RoundHeaderButton(glyph: .chevronLeft, label: "Back") { vm.back() }
            }
            Text(title)
                .textRole(.quietTitle)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
            Spacer()
            RoundHeaderButton(glyph: .close, label: "Close", action: onClose)
                .accessibilityIdentifier("closeAddCircle")
        }
        .padding(.horizontal, 20)
        .padding(.top, 18)
        .padding(.bottom, 6)
    }

    @ViewBuilder private var content: some View {
        switch vm.step {
        case .role: roles
        case .scope: scopes
        case .timing: timing
        case .link: link
        case let .compare(code, helper): compare(code: code, helper: helper, accepted: false)
        case let .accepted(code, helper): compare(code: code, helper: helper, accepted: true)
        case let .done(person): done(person)
        case let .closed(why): closed(why)
        }
    }

    // MARK: 1. Role

    private var roles: some View {
        FlatGroup(inset: 0) {
            ForEach(CircleLogic.roles, id: \.self) { role in
                choiceRow(CircleLogic.roleName(role), hint: CircleLogic.roleHint(role, mine: true), symbol: CircleLogic.roleSymbol(role),
                          id: "circleRole:\(CircleLogic.roleName(role))") {
                    vm.role = role
                    vm.step = .scope
                }
            }
        }
        .padding(.top, 12)
    }

    // MARK: 2. Scope

    private var scopes: some View {
        let agents = model.connections
        return VStack(alignment: .leading, spacing: 0) {
            FlatGroup(inset: 0) {
                ForEach(CircleLogic.presets, id: \.self) { preset in
                    choiceRow(CircleLogic.presetTitle(preset), hint: nil, symbol: CircleLogic.presetSymbol(preset),
                              id: "circleScope:\(CircleLogic.presetTitle(preset))") {
                        vm.preset = preset
                        vm.step = .timing
                    }
                }
                choiceRow("One AI", hint: nil, symbol: "sparkles", id: "circleScope:agent", chevron: !pickingAgent) {
                    withAnimation(.smooth(duration: 0.2)) { pickingAgent.toggle() }
                }
                .disabled(agents.isEmpty)
            }
            if pickingAgent {
                FlatGroup(inset: 0) {
                    ForEach(agents, id: \.id) { c in
                        Button {
                            vm.preset = .agent(connectionId: c.id, label: c.label)
                            vm.step = .timing
                            feedback.play(.selection)
                        } label: {
                            TileRow(untrustedName(c.label, limit: 40)) { ConnectionAvatar(connection: c, size: 32) } trailing: { EmptyView() }
                                .flatRow()
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("circleAgent:\(c.id)")
                    }
                }
                .padding(.top, 10)
                .transition(.opacity)
            }
        }
        .padding(.top, 12)
    }

    // MARK: 3. Instant or Deferred

    private var timing: some View {
        VStack(alignment: .leading, spacing: 0) {
            summary
            SectionHeader("How soon", quiet: true)
            Chooser([
                Chooser.Segment(id: "circleDeferred", title: "Deferred", selected: vm.deferred) { vm.deferred = true },
                Chooser.Segment(id: "circleInstant", title: "Instant", selected: !vm.deferred) { vm.deferred = false },
            ])
            .padding(.horizontal, 20)
            Text(vm.deferred ? "Within 4 h" : "Within 2 min, they're at hand")
                .textRole(.meta)
                .foregroundStyle(Palette.inkMeta)
                .padding(.horizontal, 20)
                .padding(.top, 8)
            if vm.role == .guardian {
                GuardianTermsStep(days: $vm.noticeDays)
            }
        }
        .padding(.top, 12)
    }

    /// The choices so far, one row each.
    private var summary: some View {
        FlatGroup(inset: 0) {
            TileRow(CircleLogic.roleName(vm.role), value: CircleLogic.roleHint(vm.role, mine: true), chevron: false) {
                SymbolTile(symbol: CircleLogic.roleSymbol(vm.role))
            } trailing: { EmptyView() }
            .flatRow()
            TileRow(CircleLogic.presetTitle(vm.preset), chevron: false) {
                SymbolTile(symbol: CircleLogic.presetSymbol(vm.preset))
            } trailing: { EmptyView() }
            .flatRow()
        }
    }

    // MARK: 4. The link

    @ViewBuilder private var link: some View {
        if let start = vm.start {
            VStack(spacing: 16) {
                QRCodeView(text: start.link, side: 200)
                    .accessibilityIdentifier("circleQr")
                Text(verbatim: start.link)
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(2)
                    .truncationMode(.middle)
                    .multilineTextAlignment(.center)
                    .environment(\.layoutDirection, .leftToRight)
                    .padding(.horizontal, 24)
                HStack(spacing: 10) {
                    ShareLink(item: start.link) {
                        Label("Share", systemImage: "square.and.arrow.up")
                    }
                    .buttonStyle(SignalButtonStyle(.inverse, size: .small, fill: false))
                    .accessibilityIdentifier("circleShare")
                    Button {
                        UIPasteboard.general.string = start.link
                        feedback.play(.copied)
                        model.notice = "Link copied."
                    } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                    .buttonStyle(SignalButtonStyle(.outline, size: .small, fill: false))
                    .accessibilityIdentifier("circleCopy")
                }
                waiting("Waiting for them to open it")
            }
            .frame(maxWidth: .infinity)
            .padding(.top, 14)
        }
    }

    private func waiting(_ text: String) -> some View {
        HStack(spacing: 8) {
            ProgressView()
            Text(text).textRole(.meta).foregroundStyle(Palette.inkMeta)
        }
        .padding(.top, 4)
        .accessibilityElement(children: .combine)
    }

    // MARK: 5. The code

    private func compare(code: String, helper: String, accepted: Bool) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            FlatGroup(inset: 0) {
                CirclePersonLabel(email: helper, size: 36)
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityIdentifier("circleHelper")
                if let days = vm.draft.noticeDays { GuardianNoticeFact(days: days) }
            }
            CircleCode(code: code)
                .padding(.top, 26)
                .padding(.horizontal, 20)
                .accessibilityIdentifier("circleCode")
            Text("Same code on their phone?")
                .textRole(.body)
                .foregroundStyle(Palette.ink2)
                .frame(maxWidth: .infinity)
                .padding(.top, 10)
            if !accepted {
                waiting("Waiting for them to accept")
                    .frame(maxWidth: .infinity)
                    .padding(.top, 12)
            }
        }
        .padding(.top, 12)
    }

    // MARK: Settled

    private func done(_ person: CirclePerson) -> some View {
        VStack(spacing: 12) {
            Image(systemName: "checkmark.circle.fill").font(.system(size: 44)).foregroundStyle(Palette.success)
            Text(verbatim: "\(CircleLogic.name(person.peer)) · \(CircleLogic.roleName(person.role))")
                .textRole(.row)
                .foregroundStyle(Palette.ink)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 40)
        .accessibilityIdentifier("circleAdded")
    }

    private func closed(_ why: String) -> some View {
        Text(why)
            .textRole(.body)
            .foregroundStyle(Palette.ink2)
            .frame(maxWidth: .infinity)
            .padding(.top, 40)
            .accessibilityIdentifier("circleClosed")
    }

    // MARK: The foot

    @ViewBuilder private var footer: some View {
        switch vm.step {
        case .timing:
            QuietFooter {
                Button {
                    Task { await vm.create(model) }
                } label: {
                    HStack(spacing: 8) {
                        if vm.busy { ProgressView().tint(Palette.onInverse) } else { Image(systemName: "qrcode") }
                        Text("Make the link")
                    }
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(!vm.canMakeLink)
                .accessibilityIdentifier("circleCreate")
            }
        case .accepted:
            QuietFooter {
                HoldButton(title: "Hold to confirm", tint: Palette.ink, symbol: Biometry.symbol, busy: vm.busy) {
                    Task { await vm.confirm(model) }
                }
                .accessibilityHint(HoldButton.hint)
                .accessibilityIdentifier("circleConfirm")
            }
        case .done, .closed:
            QuietFooter {
                Button("Done", action: onClose)
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .accessibilityIdentifier("circleDone")
            }
        default:
            EmptyView()
        }
    }

    private func choiceRow(
        _ title: String, hint: String?, symbol: String, id: String, chevron: Bool = true, action: @escaping () -> Void
    ) -> some View {
        Button {
            action()
            feedback.play(.selection)
        } label: {
            HStack(spacing: 12) {
                SymbolTile(symbol: symbol)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).textRole(.row).foregroundStyle(Palette.ink)
                    if let hint {
                        Text(hint).font(RFont.sans(13)).foregroundStyle(Palette.ink2)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if chevron { RowChevron() }
            }
            .padding(.vertical, 10)
            .frame(minHeight: 56)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(id)
    }
}
