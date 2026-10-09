import SwiftUI

/// One grant: what it allows, its clock and uses, where it came from, what it was used for, and the way to end it
/// (running) or to resume or delete it for good (ended).
struct GrantDetailScreen: View {
    var grantId: String
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var confirmingRevoke = false
    @State private var confirmingDelete = false
    @State private var resuming: GrantPick?
    @State private var busy = false
    @State private var error: String?

    init(grantId: String) {
        self.grantId = grantId
    }

    var body: some View {
        Group {
            if let grant = model.grants.first(where: { $0.id == grantId }) {
                content(grant)
            } else {
                EmptyState(symbol: "key.horizontal", title: "This grant is gone", message: "It was deleted or removed.")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .pageBackground()
                    .accessibilityIdentifier("grantGone")
            }
        }
        .navigationTitle("Grant")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func content(_ grant: GrantView) -> some View {
        let used = model.activity.filter { $0.grantId == grant.id }
        return List {
            Section { header(grant) }
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 4, leading: 4, bottom: 0, trailing: 4))

            Section {
                if grant.lines.isEmpty {
                    InfoRow("Nothing specific").cardRow()
                }
                ForEach(Array(grant.lines.enumerated()), id: \.offset) { _, line in
                    Text(untrusted(line))
                        .font(RFont.sans(15.5))
                        .foregroundStyle(Palette.text)
                        .environment(\.layoutDirection, .leftToRight)
                        .padding(.vertical, 3)
                        .cardRow()
                }
            } header: {
                GroupHeader("What it allows")
            }

            Section {
                HStack(spacing: 12) {
                    ConnectionIcon(connectionId: grant.connectionId, label: grant.connectionLabel, size: 30)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("AI").font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
                        Text(untrusted(grant.connectionLabel)).font(RFont.sans(16, .medium)).foregroundStyle(Palette.text)
                    }
                }
                .padding(.vertical, 2)
                .accessibilityElement(children: .combine)
                .cardRow()
                TimelineView(.periodic(from: .now, by: 30)) { ctx in
                    InfoRow(
                        "Expires",
                        subtitle: grant.expiresAt.map { "\(GrantText.expiry($0, now: nowSeconds(ctx.date))) · \(GrantText.full($0))" } ?? "Never on its own"
                    )
                }
                .cardRow()
                VStack(alignment: .leading, spacing: 6) {
                    Text("Uses").font(RFont.sans(16, .medium)).foregroundStyle(Palette.text)
                    UsesMeter(uses: Int(grant.uses), maxUses: grant.maxUses.map(Int.init))
                }
                .padding(.vertical, 3)
                .accessibilityElement(children: .combine)
                .cardRow()
                InfoRow("Last used", subtitle: grant.lastUsedAt.map { "\(GrantText.relative($0)) · \(GrantText.full($0))" } ?? "Not used yet")
                    .cardRow()
                InfoRow("Created", subtitle: GrantText.full(grant.createdAt)).cardRow()
                InfoRow("Came from", subtitle: GrantText.origin(grant.origin, label: grant.connectionLabel)).cardRow()
            } header: {
                GroupHeader("Details")
            }

            Section {
                if used.isEmpty {
                    InfoRow("Nothing yet", subtitle: "Operations this grant covers will be listed here.").cardRow()
                }
                ForEach(used.prefix(10), id: \.id) { entry in
                    Button {
                        feedback.play(.tap)
                        model.push(.activityDetail(entry.id))
                    } label: {
                        GrantUseRow(entry: entry)
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("entry:\(entry.id)")
                    .cardRow()
                }
            } header: {
                GroupHeader("Used for")
            }

            Section {
                VStack(spacing: 10) {
                    if let error {
                        FormBanner(text: error)
                    }
                    if !grant.active {
                        Button {
                            resuming = GrantPick(grant)
                        } label: {
                            Label("Resume", systemImage: "arrow.clockwise")
                        }
                        .buttonStyle(CapsuleButtonStyle(kind: .primary))
                        .disabled(busy)
                        .accessibilityIdentifier("resume")
                    }
                    if grant.active {
                        Button(role: .destructive) {
                            confirmingRevoke = true
                        } label: {
                            Label("Delete grant", systemImage: "trash")
                        }
                        .buttonStyle(CapsuleButtonStyle(kind: .danger))
                        .disabled(busy)
                        .accessibilityIdentifier("revoke")
                    } else {
                        Button(role: .destructive) {
                            confirmingDelete = true
                        } label: {
                            Label("Delete for good", systemImage: "trash")
                        }
                        .buttonStyle(CapsuleButtonStyle(kind: .danger))
                        .disabled(busy)
                        .accessibilityIdentifier("deleteEnded")
                    }
                }
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
            }
        }
        .reinsGrouped()
        .confirmationDialog("Delete this grant?", isPresented: $confirmingRevoke, titleVisibility: .visible) {
            Button("Delete", role: .destructive) { run(leave: true) { await model.revokeGrant(grant.id) } }
        } message: {
            Text(untrusted(grant.summary) + "\n\nYou can resume it later from the Expired list.")
        }
        .presentationFeedback(confirmingRevoke)
        .confirmationDialog("Delete this grant for good?", isPresented: $confirmingDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) { run(leave: true) { await model.deleteGrant(grant.id) } }
        } message: {
            Text(untrusted(grant.summary) + "\n\nIt cannot be resumed afterwards.")
        }
        .presentationFeedback(confirmingDelete)
        .sheet(item: $resuming) { pick in
            ResumeSheet(grant: pick.grant) { seconds, standing in
                run(leave: false) { await model.resumeGrant(pick.grant.id, seconds: seconds, standing: standing) }
            }
            .environment(model)
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(resuming != nil)
    }

    private func header(_ grant: GrantView) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .center, spacing: 14) {
                if grant.active {
                    TimelineView(.periodic(from: .now, by: 1)) { ctx in
                        ExpiryPie(clock: grantClock(grant, now: nowSeconds(ctx.date)), size: 60)
                    }
                } else {
                    ActionTile(kind: ActionKind.of(grant.action), size: 52)
                }
                VStack(alignment: .leading, spacing: 8) {
                    Text(untrusted(grant.summary))
                        .font(RFont.sans(21, .semibold))
                        .foregroundStyle(Palette.text)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                        .accessibilityIdentifier("grantTitle")
                    HStack(spacing: 6) {
                        if grant.active {
                            TintedTag("Active", tint: Palette.success)
                        } else {
                            TintedTag(GrantText.inactiveWord(grant), tint: Palette.tertiary)
                        }
                        TintedTag(GrantText.gerund(grant.action), tint: ActionTile.color(ActionKind.of(grant.action)))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            ConnectorTags(service: grant.service, account: grant.account)
        }
        .padding(.vertical, 6)
    }

    /// Runs one of the grant's actions; on success after a deletion, the page closes (the grant is gone or ended).
    private func run(leave: Bool, _ action: @escaping () async -> String?) {
        busy = true
        error = nil
        // The action sounds once the core answers: the dialog or sheet that asked closes quietly.
        feedback.quietClose()
        Task {
            let failure = await action()
            busy = false
            error = failure
            if failure == nil && leave { model.back() }
        }
    }
}
