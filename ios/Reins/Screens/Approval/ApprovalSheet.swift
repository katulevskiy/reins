import SwiftUI

/// The approval sheet: what is asked, one tap to decide, and everything else under "More options" (the Android app's
/// ApprovalSheet). Approving asks for Face ID, Touch ID or the passcode first; denying never does.
struct ApprovalSheet: View {
    var requestId: String
    @Environment(AppModel.self) private var model
    @State private var vm: ApprovalModel

    init(requestId: String) {
        self.requestId = requestId
        _vm = State(initialValue: ApprovalModel(requestId: requestId))
    }

    /// A sheet already holding its request (previews).
    init(model: ApprovalModel) {
        requestId = model.requestId
        _vm = State(initialValue: model)
    }

    var body: some View {
        Group {
            if let view = vm.view {
                ApprovalContent(vm: vm, view: view)
            } else {
                DecisionLoading(error: vm.loading ? nil : vm.error)
            }
        }
        .task { await vm.load(model) }
        .onChange(of: vm.finished) { _, done in if done { model.closeSheet() } }
    }
}

private struct ApprovalContent: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(AppModel.self) private var model
    /// The request was in the waiting list while the sheet was open, so its leaving means it was answered elsewhere
    /// or the server dropped it.
    @State private var wasListed = false

    private var gone: Bool { wasListed && !vm.busy && !vm.finished && !model.pending.contains { $0.id == view.requestId } }

    var body: some View {
        ScrollViewReader { proxy in
            DecisionLayout {
                details
            } recap: {
                VStack(alignment: .leading, spacing: 4) {
                    Text(untrusted(view.connectionLabel)).font(RFont.sans(14, .medium)).foregroundStyle(Palette.secondary)
                    Text(view.headline).font(RFont.sans(22, .semibold)).foregroundStyle(Palette.text).lineLimit(3)
                    if let error = vm.error { Banner(error, kind: .error).padding(.top, 8) }
                }
            } decision: {
                decisionBar
            }
            .onChange(of: vm.moreOpen) { _, open in
                // Opening "More options" brings them into view instead of leaving them below the buttons.
                guard open else { return }
                Task {
                    try? await Task.sleep(for: .milliseconds(280))
                    withAnimation(.smooth) { proxy.scrollTo("moreEnd", anchor: .bottom) }
                }
            }
        }
        .onAppear { if model.pending.contains(where: { $0.id == view.requestId }) { wasListed = true } }
        .onChange(of: model.pending.map(\.id)) { _, ids in if ids.contains(view.requestId) { wasListed = true } }
        .accessibilityIdentifier("approvalSheet")
    }

    @ViewBuilder private var details: some View {
        VStack(alignment: .leading, spacing: 0) {
            DecisionHeader(
                connectionId: view.connectionId,
                label: view.connectionLabel,
                subtitle: TimeText.dateTime(view.createdAt),
                kind: view.actionKind,
                count: view.actionKind == .grant ? 1 : view.shownCount,
                title: view.headline
            ) {
                ConnectorTags(service: view.service, account: view.account, name: view.mcp.map { untrusted($0.serverName) })
                    .padding(.top, 12)
                if let query = view.query {
                    Text(untrusted(query))
                        .font(RFont.mono(14))
                        .foregroundStyle(Palette.secondary)
                        .lineLimit(3)
                        .environment(\.layoutDirection, .leftToRight)
                        .padding(.top, 12)
                        .textSelection(.enabled)
                }
                WaitLine(view: view)
            }
            if gone {
                Banner("This request is no longer waiting. It was answered on another device, or the server let it go.", kind: .warning)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 6)
                    .accessibilityIdentifier("noLongerWaiting")
            }
            if let s = vm.suggestion {
                SuggestionCard(suggestion: s).padding(.horizontal, 16).padding(.vertical, 6)
            }
            kindSection
            MoreOptions(vm: vm, view: view)
            if let error = vm.error {
                Banner(error, kind: .error).padding(16).accessibilityIdentifier("approvalError")
            }
            if let choice = vm.preview?.choice {
                let warnings = publicDomainWarnings(choice)
                if !warnings.isEmpty {
                    Banner(
                        "Allowing everyone at \(warnings.joined(separator: ", ")) covers millions of unrelated people. Prefer specific addresses.",
                        kind: .warning
                    )
                    .padding(16)
                    .accessibilityIdentifier("publicDomainWarning")
                }
            }
            Color.clear.frame(height: 1).id("moreEnd")
        }
        .animation(.smooth(duration: 0.3), value: vm.moreOpen)
        .animation(.smooth(duration: 0.25), value: vm.error)
    }

    @ViewBuilder private var kindSection: some View {
        switch view.kind {
        case .grant:
            if let grant = view.grant { GrantRequestCard(grant: grant, label: view.connectionLabel) }
        case .accounts:
            AccountsCard(vm: vm, view: view)
        case .send:
            if let email = view.email { EmailPreview(email: email).padding(.horizontal, 16).padding(.vertical, 8) }
        case .write:
            // What the change is, in its own terms where there are some: a push ref by ref, a call to an MCP server's
            // tool, a question, secrets, an SSH sign-in; else the core's summary lines.
            if let git = view.git {
                GitPushSection(push: git, host: serviceName(view.service))
            } else if let mcp = view.mcp {
                McpCallSection(call: mcp)
            } else if let ask = view.ask {
                AskSection(ask: ask)
            } else if let secrets = view.secrets {
                SecretsSection(secrets: secrets)
            } else if let ssh = view.ssh {
                SshSection(ssh: ssh)
            } else {
                WritePreview(view: view)
            }
            if let blob = view.blob { AttachedFileSection(blob: blob) }
            // A destructive MCP tool says so in its own section.
            if view.noStanding && view.mcp == nil {
                Banner(
                    "This changes something that cannot be undone or reaches far. It is always asked for and can never be allowed in advance, so read the details above before you approve.",
                    kind: .warning
                )
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .accessibilityIdentifier("onceWarning")
            }
        case .search, .read, .fetch:
            MessagesSection(vm: vm, view: view)
        }
    }

    private var decisionBar: some View {
        let allow = view.kind == .grant || view.kind == .accounts
        // A question from the desktop app is answered, not approved.
        return Group {
            if gone {
                Button("Close") { model.closeSheet() }
                    .buttonStyle(.glass)
                    .font(RFont.sans(17, .semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
            } else {
                DecisionBar(
                    denyTitle: view.isQuestion ? "No" : "Deny",
                    approveTitle: view.isQuestion ? "Yes" : (allow ? "Allow" : "Approve"),
                    accent: allow,
                    busy: vm.busy,
                    onDeny: { Task { await vm.deny(model) } },
                    onApprove: { Task { await vm.approve(model) } }
                )
            }
        }
    }
}

/// How long the AI is still waiting, or that it stopped (approving still works).
private struct WaitLine: View {
    var view: ApprovalView

    var body: some View {
        LiveClock { now in
            let label = untrusted(view.connectionLabel)
            if let u = Urgency.of(createdAt: view.createdAt, waitUntil: view.waitUntil, now: now) {
                if u.stale {
                    Banner("\(label) stopped waiting. You can still approve; then ask \(label) to try again and it will go through.", kind: .warning)
                        .padding(.top, 14)
                        .accessibilityIdentifier("lateBanner")
                } else {
                    Text("\(label) is waiting · \(u.remainingSeconds) s left")
                        .font(RFont.sans(13.5, .medium))
                        .foregroundStyle(u.urgent ? Palette.danger : Palette.secondary)
                        .monospacedDigit()
                        .padding(.top, 12)
                        .accessibilityIdentifier("waitLine")
                }
            }
        }
    }
}

#Preview("Search") {
    PreviewHost { ApprovalSheet(requestId: "req1") }
}

#Preview("Push") {
    PreviewHost { ApprovalSheet(requestId: "req20") }
}

#Preview("Permission") {
    PreviewHost { ApprovalSheet(requestId: "req3") }
}
