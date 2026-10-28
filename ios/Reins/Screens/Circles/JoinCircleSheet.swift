import Observation
import SwiftUI
import UIKit

/// Settings › Circles › Join (04 §3, the helper's side): paste the link someone sent; once their phone sent the terms,
/// who invites, the role, what it covers, how soon and what you'll see, then the six digits both phones show. Accept
/// (checked with Face ID, Touch ID or the passcode) or Decline; it runs once they confirm on their phone.
@Observable
@MainActor
final class JoinCircleModel {
    var link = ""
    private(set) var view: CircleInviteView?
    private(set) var busy = false
    var error: String?
    var pollInterval: Duration = .milliseconds(1500)

    var canOpen: Bool { !busy && !link.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    func open(_ app: AppModel) async {
        guard canOpen else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            view = try await app.core.circleJoinOpen(link: link.trimmingCharacters(in: .whitespacesAndNewlines))
            app.feedback.play(.open)
        } catch {
            app.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    /// Asks again until the terms come (Opened), and until they confirm (Accepted).
    func poll(_ app: AppModel) async {
        guard let id = view?.id else { return }
        while !Task.isCancelled, let stage = view?.stage, stage == .opened || stage == .accepted {
            do {
                let next = try await app.core.circleJoinPoll(id: id)
                guard !Task.isCancelled else { return }
                if next.stage == .compare, view?.stage != .compare { app.feedback.play(.requestArrived) }
                if next.stage == .active, view?.stage != .active { app.feedback.play(.connected) }
                view = next
                error = nil
            } catch {
                if case CoreError.Network = error {} else { self.error = error.userMessage }
            }
            if view?.stage == stage { try? await Task.sleep(for: pollInterval) }
        }
    }

    func accept(_ app: AppModel) async {
        guard let view, view.stage == .compare, !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        let reason = "Join \(CircleLogic.name(view.inviter))'s Circle as \(CircleLogic.roleName(view.role))"
        switch await OwnerPrompt.confirm(app, reason: reason) {
        case .confirmed: break
        case .cancelled: return
        case .unavailable:
            app.feedback.play(.error)
            error = OwnerPrompt.unavailableMessage
            return
        }
        do {
            try await app.core.circleJoinAccept(id: view.id)
            self.view?.stage = .accepted
            app.feedback.play(.approved)
        } catch {
            app.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    func decline(_ app: AppModel) async {
        guard let view, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            try await app.core.circleJoinDecline(id: view.id)
            self.view?.stage = .declined
            app.feedback.play(.denied)
        } catch {
            app.feedback.play(.error)
            self.error = error.userMessage
        }
    }
}

struct JoinCircleSheet: View {
    var onClose: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm = JoinCircleModel()
    @FocusState private var focused: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Join a Circle")
                    .textRole(.quietTitle)
                    .foregroundStyle(Palette.ink)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                RoundHeaderButton(glyph: .close, label: "Close", action: onClose)
                    .accessibilityIdentifier("closeJoinCircle")
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
            .padding(.bottom, 6)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if let view = vm.view {
                        terms(view)
                    } else {
                        paste
                    }
                    if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("joinCircleError") }
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
        .task(id: "\(vm.view?.id ?? ""):\(vm.view.map { "\($0.stage)" } ?? "")") { await vm.poll(model) }
        .accessibilityIdentifier("joinCircleSheet")
    }

    // MARK: The link

    private var paste: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader("Their link", quiet: true)
            FlatGroup(inset: 32) {
                InputRow(glyph: .link) {
                    TextField("reins-circle:…", text: $vm.link)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                        .submitLabel(.go)
                        .focused($focused)
                        .onSubmit { Task { await vm.open(model) } }
                        .accessibilityIdentifier("joinCircleLink")
                } trailing: {
                    Button {
                        if let text = UIPasteboard.general.string { vm.link = text }
                        feedback.play(.tap)
                    } label: {
                        Image(systemName: "doc.on.clipboard").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.ink2)
                            .frame(width: 44, height: 44)
                    }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityLabel("Paste")
                    .accessibilityIdentifier("joinCirclePaste")
                }
                .environment(\.layoutDirection, .leftToRight)
            }
        }
        .padding(.top, 6)
    }

    // MARK: The terms and the code

    @ViewBuilder private func terms(_ view: CircleInviteView) -> some View {
        FlatGroup(inset: 0) {
            CirclePersonLabel(email: view.inviter, size: 36)
                .padding(.vertical, 10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("joinInviter")
        }
        .padding(.top, 12)
        switch view.stage {
        case .opened:
            waiting("Waiting for their phone")
        case .compare, .accepted:
            SectionHeader("You'd be", quiet: true)
            FlatGroup(inset: 0) {
                fact(CircleLogic.roleName(view.role), value: CircleLogic.roleHint(view.role, mine: false), symbol: CircleLogic.roleSymbol(view.role))
                if !untrusted(view.scopeLine).isEmpty {
                    fact(untrusted(view.scopeLine), symbol: "square.stack.3d.up")
                }
                fact(CircleLogic.classLine(deferred: view.deferred, windowSecs: view.windowSecs), symbol: "clock")
                if view.role != .watcher {
                    fact(view.fullCards ? "Full requests" : "A summary", symbol: "eye")
                }
                // A guardianship: the notice the ward chose, before Accept.
                if view.role == .guardian, let days = view.noticeDays {
                    GuardianNoticeFact(days: days)
                }
            }
            .accessibilityIdentifier("joinTerms")
            if let code = view.code {
                CircleCode(code: code)
                    .padding(.top, 26)
                    .padding(.horizontal, 20)
                    .accessibilityIdentifier("joinCode")
                Text("Same code on their phone?")
                    .textRole(.body)
                    .foregroundStyle(Palette.ink2)
                    .frame(maxWidth: .infinity)
                    .padding(.top, 10)
            }
            if view.stage == .accepted { waiting("Waiting for them to confirm") }
        case .active:
            settled("checkmark.circle.fill", "You're in", tint: Palette.success)
        case .declined:
            settled("xmark.circle", "Declined", tint: Palette.inkMeta)
        case .expired:
            settled("clock.badge.xmark", "The invitation expired", tint: Palette.inkMeta)
        }
    }

    private func fact(_ title: String, value: String? = nil, symbol: String) -> some View {
        TileRow(title, value: value, chevron: false) { SymbolTile(symbol: symbol) } trailing: { EmptyView() }
            .flatRow()
    }

    private func waiting(_ text: String) -> some View {
        HStack(spacing: 8) {
            ProgressView()
            Text(text).textRole(.meta).foregroundStyle(Palette.inkMeta)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 18)
        .accessibilityElement(children: .combine)
    }

    private func settled(_ symbol: String, _ text: String, tint: Color) -> some View {
        VStack(spacing: 10) {
            Image(systemName: symbol).font(.system(size: 40)).foregroundStyle(tint)
            Text(text).textRole(.row).foregroundStyle(Palette.ink)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 30)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("joinSettled")
    }

    // MARK: The foot

    @ViewBuilder private var footer: some View {
        if vm.view == nil {
            QuietFooter {
                Button {
                    focused = false
                    Task { await vm.open(model) }
                } label: {
                    HStack(spacing: 8) {
                        if vm.busy { ProgressView().tint(Palette.onInverse) }
                        Text("Open")
                    }
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(!vm.canOpen)
                .accessibilityIdentifier("joinCircleOpen")
            }
        } else if vm.view?.stage == .compare {
            QuietFooter {
                RatioRow(spacing: 10, ratio: 1 / 1.35) {
                    Button("Decline") { Task { await vm.decline(model) } }
                        .buttonStyle(SignalButtonLook(kind: .outline))
                        .disabled(vm.busy)
                        .accessibilityIdentifier("joinCircleDecline")
                    Button {
                        Task { await vm.accept(model) }
                    } label: {
                        HStack(spacing: 8) {
                            if vm.busy { ProgressView().tint(Palette.onInverse) } else { BiometryMark(size: 18) }
                            Text("Accept")
                        }
                    }
                    .buttonStyle(SignalButtonLook(kind: .inverse))
                    .disabled(vm.busy || vm.view?.code == nil)
                    .accessibilityIdentifier("joinCircleAccept")
                }
                .frame(height: 54)
            }
        } else if let stage = vm.view?.stage, stage == .active || stage == .declined || stage == .expired {
            QuietFooter {
                Button("Done", action: onClose)
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .accessibilityIdentifier("joinCircleDone")
            }
        }
    }
}
