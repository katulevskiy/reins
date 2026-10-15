import Observation
import SwiftUI

/// Another phone signed in to this account and asks for its keys ("Add another phone"). Approving (Face ID, Touch ID
/// or the passcode first) seals the account secret to that phone; the server relays it without being able to read it.
/// The code is worked out here from the key the server relayed, so a swapped key shows another code than the phone's.
struct JoinSheet: View {
    var joinId: String
    @Environment(AppModel.self) private var model
    @State private var vm: JoinModel

    init(joinId: String) {
        self.joinId = joinId
        _vm = State(initialValue: JoinModel(joinId: joinId))
    }

    var body: some View {
        Group {
            if let view = vm.view {
                JoinContent(vm: vm, view: view)
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
final class JoinModel {
    let joinId: String
    private(set) var loading = true
    private(set) var view: JoinView?
    private(set) var busy = false
    var error: String?
    private(set) var finished = false

    init(joinId: String) {
        self.joinId = joinId
    }

    init(view: JoinView) {
        joinId = view.id
        self.view = view
        loading = false
    }

    func load(_ app: AppModel) async {
        guard view == nil else { return }
        do {
            view = try await app.core.joinView(id: joinId)
            loading = false
        } catch {
            loading = false
            self.error = decisionErrorMessage(error)
        }
    }

    func approve(_ app: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        let name = untrustedName(view?.deviceName ?? "", limit: 40)
        switch await OwnerPrompt.confirm(app, reason: JoinWords.ask) {
        case .confirmed:
            do {
                try await app.core.answerJoin(id: joinId, approve: true)
                app.feedback.play(.connected)
                app.notice = "\(name.isEmpty ? "The other phone" : name) can open your account now."
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
        guard !busy else { return }
        busy = true
        error = nil
        app.feedback.play(.denied)
        do {
            try await app.core.answerJoin(id: joinId, approve: false)
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

/// A join's words: Reins' own sentence; the phone's name only quoted, on a row of its own.
enum JoinWords {
    static let label = "Account · New phone"
    static let ask = "Add a phone to your account"
}

private struct JoinContent: View {
    var vm: JoinModel
    var view: JoinView
    @Environment(AppModel.self) private var model

    var body: some View {
        DecisionLayout(closeInField: true) {
            VStack(alignment: .leading, spacing: 0) {
                SheetField(kind: .accounts, label: JoinWords.label, ask: JoinWords.ask)
                Text("A phone signed in to your account and asks for its keys. Approve only if it is yours and shows this code.")
                    .textRole(.body)
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 24)
                    .padding(.top, 18)
                    .padding(.bottom, 6)
                FactGroup {
                    FactRow(key: "Calls itself", value: PairWords.quoted(view.deviceName), valueId: "joinPhone")
                    // Worked out here from the key the server relayed: a swapped key shows another code.
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Code").font(RFont.sans(15)).foregroundStyle(Palette.ink2)
                        Text(view.code)
                            .font(RFont.mono(40, .semibold))
                            .foregroundStyle(Palette.ink)
                            .environment(\.layoutDirection, .leftToRight)
                            .accessibilityLabel("Code \(view.code)")
                            .accessibilityIdentifier("joinCode")
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 14)
                }
                Banner("It can then open your vault, and it becomes the phone that approves once it is set up.")
                    .padding(.horizontal, 20)
                    .padding(.top, 14)
                if let error = vm.error {
                    Banner(error, kind: .error).padding(16).accessibilityIdentifier("joinError")
                }
            }
        } recap: {
            Text(JoinWords.ask).font(RFont.serif(28)).foregroundStyle(Palette.ink)
        } decision: {
            DecisionBar(
                approveTitle: "Approve",
                busy: vm.busy,
                approveEnabled: true,
                onDeny: { Task { await vm.deny(model) } },
                onApprove: { Task { await vm.approve(model) } }
            )
        }
        .accessibilityContainer("joinSheet")
    }
}
