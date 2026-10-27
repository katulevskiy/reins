import Observation
import SwiftUI

/// A `Join` item: a phone asking to view the account as a second phone (phone roles: an enrollment) or, on an account
/// without phone roles, one asking for its keys (`JoinSheet`). The core says which.
struct JoinRequestSheet: View {
    var joinId: String
    @Environment(AppModel.self) private var model
    @State private var enrollment: Bool?

    var body: some View {
        Group {
            switch enrollment {
            case .none: DecisionLoading(error: nil)
            case .some(true): EnrollRequestSheet(joinId: joinId)
            case .some(false): JoinSheet(joinId: joinId)
            }
        }
        .task {
            guard enrollment == nil else { return }
            // Gone or unreadable: the legacy sheet says so, as it always did.
            enrollment = (try? await model.core.joinIsEnrollment(id: joinId)) ?? false
        }
    }
}

/// On the approving phone: another phone of the account asks to view it as a second phone. `enroll_view` challenges
/// it the first time and shows the code once the other phone revealed; the user compares the two codes, holds Approve
/// and passes the owner check (Face ID, Touch ID or the passcode), then `enroll_confirm` lists it as a viewer. Deny is
/// one tap. Nothing else answers it: Autopilot, a bypass or a grant never decide another phone.
struct EnrollRequestSheet: View {
    var joinId: String
    @Environment(AppModel.self) private var model
    @State private var vm: EnrollRequestModel

    init(joinId: String) {
        self.joinId = joinId
        _vm = State(initialValue: EnrollRequestModel(joinId: joinId))
    }

    var body: some View {
        Group {
            if let view = vm.view {
                EnrollRequestContent(vm: vm, view: view)
            } else {
                DecisionLoading(error: vm.loading ? nil : vm.error)
            }
        }
        .task { await vm.load(model) }
        .onChange(of: vm.finished) { _, done in if done { model.closeSheet() } }
    }
}

@Observable
@MainActor
final class EnrollRequestModel {
    let joinId: String
    private(set) var loading = true
    private(set) var view: EnrollView?
    private(set) var busy = false
    var error: String?
    private(set) var finished = false
    /// How long to wait between asks while the other phone has not shown its code.
    var pollInterval: Duration = .milliseconds(1500)

    /// Approvable only once both phones show the code (the core refuses before), and not while answering.
    var canApprove: Bool { view?.code != nil && !busy && !finished }

    init(joinId: String) {
        self.joinId = joinId
    }

    init(view: EnrollView) {
        joinId = view.id
        self.view = view
        loading = false
    }

    /// Until the other phone shows its code (it reveals after this phone's challenge, sent by the first ask).
    func load(_ app: AppModel) async {
        while !Task.isCancelled, view?.code == nil {
            do {
                let next = try await app.core.enrollView(id: joinId)
                view = next
                loading = false
                error = nil
                if next.code != nil { return }
            } catch {
                loading = false
                self.error = decisionErrorMessage(error)
                if case CoreError.Network = error {} else { return }
            }
            try? await Task.sleep(for: pollInterval)
        }
    }

    func approve(_ app: AppModel) async {
        guard canApprove else { return }
        busy = true
        error = nil
        let name = untrustedName(view?.deviceName ?? "", limit: 40)
        switch await OwnerPrompt.confirm(app, reason: EnrollWords.ask) {
        case .confirmed:
            do {
                try await app.core.enrollConfirm(id: joinId, approve: true)
                app.feedback.play(.connected)
                app.notice = "\(name.isEmpty ? "The other phone" : PairWords.quoted(name)) sees this account now. It decides nothing."
                await app.refreshPending()
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
        guard !busy, !finished else { return }
        busy = true
        error = nil
        app.feedback.play(.denied)
        do {
            try await app.core.enrollConfirm(id: joinId, approve: false)
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

/// A second phone's words: Reins' own sentence; the phone's name only quoted, on a row of its own.
enum EnrollWords {
    static let label = "Account · Second phone"
    /// "Add a *second phone*": Reins' shield, with the phone badge, on the accounts field.
    static let line = AskLine(
        kind: label, lead: "Add a ", target: "second phone",
        icon: AskIcon(mark: .glyph(.shieldCheck), action: .phone, kind: .accounts)
    )
    static let ask = line.text
}

private struct EnrollRequestContent: View {
    var vm: EnrollRequestModel
    var view: EnrollView
    @Environment(AppModel.self) private var model

    var body: some View {
        DecisionLayout(closeInField: true) {
            VStack(alignment: .leading, spacing: 0) {
                ConnectField(kind: .accounts, ask: EnrollWords.line) {
                    if let code = view.code {
                        // From both phones' keys and nonces: a server that swapped a key shows another code.
                        BigCode(code: code, size: 58)
                            .padding(.top, 26)
                            .accessibilityLabel("Code \(code)")
                            .accessibilityIdentifier("enrollCode")
                        Text("Approve only if it shows this code.")
                            .font(RFont.sans(14, .medium))
                            .foregroundStyle(Palette.onField.opacity(0.88))
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.top, 12)
                    } else {
                        HStack(spacing: 10) {
                            ProgressView().tint(Palette.onField)
                            Text("Waiting for its code…")
                                .font(RFont.sans(15, .medium))
                                .foregroundStyle(Palette.onField)
                        }
                        .padding(.top, 26)
                        .accessibilityIdentifier("enrollWaiting")
                    }
                }
                RaisedGroup(inset: 48) {
                    // The name it gave: the code covers its keys, not its name.
                    PairFact(glyph: .phone, value: PairWords.quoted(view.deviceName), valueId: "enrollPhone")
                }
                .padding(.top, 14)
                SectionHeader("It gets")
                RaisedGroup(inset: 48) {
                    PairFact(glyph: .eye, value: "Sees connections and activity")
                    PairFact(glyph: .block, value: "Decides nothing")
                }
                .accessibilityIdentifier("enrollGets")
                if let error = vm.error {
                    FormBanner(text: error).padding(16).accessibilityIdentifier("enrollError")
                }
            }
        } recap: {
            Text(EnrollWords.ask).textRole(.cardAsk).foregroundStyle(Palette.ink)
        } decision: {
            RatioRow(spacing: 10, ratio: 1 / 1.35) {
                Button("Deny") { Task { await vm.deny(model) } }
                    .buttonStyle(SignalButtonLook(kind: .outline))
                    .disabled(vm.busy)
                    .accessibilityIdentifier("deny")
                HoldButton(title: "Approve", tint: Palette.ink, enabled: vm.canApprove, busy: vm.busy) {
                    Task { await vm.approve(model) }
                }
                .accessibilityIdentifier("approve")
            }
            .frame(height: 54)
            .padding(.horizontal, 20)
            .padding(.vertical, 10)
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.hidden)
        .presentationCornerRadius(32)
        .accessibilityContainer("enrollSheet")
    }
}
