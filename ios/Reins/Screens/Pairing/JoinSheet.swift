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
    /// "Add a phone to *your account*": Reins' shield, with the phone badge, on the accounts field.
    static let line = AskLine(
        kind: label, lead: "Add a phone to ", target: "your account",
        icon: AskIcon(mark: .glyph(.shieldCheck), action: .phone, kind: .accounts)
    )
    static let ask = line.text
}

private struct JoinContent: View {
    var vm: JoinModel
    var view: JoinView
    @Environment(AppModel.self) private var model

    var body: some View {
        DecisionLayout(closeInField: true) {
            VStack(alignment: .leading, spacing: 0) {
                ConnectField(kind: .accounts, ask: JoinWords.line) {
                    // Worked out here from the key the server relayed: a swapped key shows another code.
                    BigCode(code: view.code, size: 58)
                        .padding(.top, 26)
                        .accessibilityLabel("Code \(view.code)")
                        .accessibilityIdentifier("joinCode")
                    Text("Approve only if it's yours and shows this code.")
                        .font(RFont.sans(14, .medium))
                        .foregroundStyle(Palette.onField.opacity(0.88))
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 12)
                }
                RaisedGroup(inset: 48) {
                    GlyphFact(glyph: .phone, value: PairWords.quoted(view.deviceName), valueId: "joinPhone")
                }
                .padding(.top, 14)
                SectionHeader("It gets")
                RaisedGroup(inset: 48) {
                    GlyphFact(glyph: .vault, value: "Your vault")
                    GlyphFact(glyph: .shieldCheck, value: "Approves requests")
                }
                .accessibilityIdentifier("joinGets")
                if let error = vm.error {
                    FormBanner(text: error).padding(16).accessibilityIdentifier("joinError")
                }
            }
        } recap: {
            Text(JoinWords.ask).textRole(.cardAsk).foregroundStyle(Palette.ink)
        } decision: {
            DecisionBar(
                approveTitle: "Approve",
                busy: vm.busy,
                approveEnabled: true,
                onDeny: { Task { await vm.deny(model) } },
                onApprove: { Task { await vm.approve(model) } }
            )
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.hidden)
        .presentationCornerRadius(32)
        .accessibilityContainer("joinSheet")
    }
}
