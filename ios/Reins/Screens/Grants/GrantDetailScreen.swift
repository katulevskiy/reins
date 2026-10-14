import SwiftUI

/// One grant: its time left as a big ring, what it allows and never includes, its uses and end, what it was used for,
/// and, pinned at the bottom, Extend and End now (running) or Resume and Delete (ended).
struct GrantDetailScreen: View {
    var grantId: String
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var confirmingRevoke = false
    @State private var confirmingDelete = false
    @State private var resuming: GrantPick?
    @State private var extending: GrantPick?
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
                EmptyState(symbol: "key.horizontal", title: "This grant is gone")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .pageBackground()
                    .accessibilityIdentifier("grantGone")
            }
        }
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func content(_ grant: GrantView) -> some View {
        let used = model.activity.filter { $0.grantId == grant.id }
        let never = GrantText.never(grant)
        return ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                header(grant)

                if let error {
                    FormBanner(text: error).padding(.horizontal, 16).padding(.top, 14)
                }

                GroupCard(header: "It may") {
                    if grant.lines.isEmpty {
                        allowLine("Nothing specific", allowed: true)
                    }
                    ForEach(Array(grant.lines.enumerated()), id: \.offset) { i, line in
                        if i > 0 { Hairline(inset: 44) }
                        allowLine(untrusted(line), allowed: true)
                    }
                    ForEach(Array(never.enumerated()), id: \.offset) { _, line in
                        Hairline(inset: 44)
                        allowLine(line, allowed: false)
                    }
                }

                GroupCard {
                    ValueRow(label: "Uses", value: GrantText.usesValue(grant))
                    Hairline()
                    TimelineView(.periodic(from: .now, by: 30)) { _ in
                        ValueRow(label: grant.active ? "Ends" : GrantText.inactiveWord(grant), value: ends(grant))
                    }
                    if let account = grant.account, !account.trimmingCharacters(in: .whitespaces).isEmpty {
                        Hairline()
                        ValueRow(label: "Account", value: account, mono: true)
                    }
                    Hairline()
                    ValueRow(label: "Came from", value: GrantText.origin(grant.origin, label: grant.connectionLabel))
                }
                .padding(.top, 12)

                GroupCard(header: used.isEmpty ? nil : "Used for · \(used.count)") {
                    if used.isEmpty {
                        ValueRow(label: "Used for", value: "Nothing yet")
                    }
                    ForEach(Array(used.prefix(10).enumerated()), id: \.element.id) { i, entry in
                        if i > 0 { Hairline(inset: 62) }
                        Button {
                            feedback.play(.tap)
                            model.push(.activityDetail(entry.id))
                        } label: {
                            HistoryRow(entry: entry)
                        }
                        .buttonStyle(RowButtonStyle())
                        .accessibilityIdentifier("entry:\(entry.id)")
                    }
                }
                .padding(.top, used.isEmpty ? 12 : 0)
            }
            .padding(.bottom, 24)
        }
        .pageBackground()
        .pageDock { dock(grant) }
        .confirmationDialog("End this grant now?", isPresented: $confirmingRevoke, titleVisibility: .visible) {
            Button("End now", role: .destructive) { run(leave: true) { await model.revokeGrant(grant.id) } }
        } message: {
            Text(untrusted(grant.summary) + "\n\nYou can resume it later from Expired.")
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
        .sheet(item: $extending) { pick in
            ResumeSheet(grant: pick.grant, extending: true) { seconds, _ in
                run(leave: false) { await model.extendGrant(pick.grant.id, seconds: seconds) }
            }
            .environment(model)
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(extending != nil)
    }

    /// The big ring with the time left, the summary, and who and where.
    private func header(_ grant: GrantView) -> some View {
        VStack(spacing: 0) {
            TimelineView(.periodic(from: .now, by: 1)) { ctx in
                let clock = grantClock(grant, now: nowSeconds(ctx.date))
                let color = grant.active ? clockColor(clock) : Palette.tertiary
                ZStack {
                    ArcRing(fraction: grant.active ? clock.fraction : 0, color: color, track: Palette.dynamic(0xE2E2E6, 0x26262B), lineWidth: 11)
                    VStack(spacing: 1) {
                        Text(grant.active ? GrantText.ringLabel(clock) : GrantText.inactiveWord(grant))
                            .font(RFont.fixedSans(grant.active ? 28 : 17, .semibold))
                            .foregroundStyle(grant.active ? Palette.text : Palette.secondary)
                            .lineLimit(1)
                            .minimumScaleFactor(0.6)
                        if grant.active && clock.remaining != nil {
                            Text("left").font(RFont.fixedSans(12)).foregroundStyle(Palette.tertiary)
                        }
                    }
                    .padding(.horizontal, 16)
                }
                .frame(width: 120, height: 120)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(grant.active ? (clock.remaining.map { "\(compactDuration($0)) left" } ?? "No time limit") : GrantText.inactiveWord(grant))
            }
            Text(untrusted(grant.summary))
                .font(RFont.sans(22, .semibold))
                .foregroundStyle(Palette.text)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 14)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("grantTitle")
            HStack(spacing: 6) {
                ConnectionIcon(connectionId: grant.connectionId, label: grant.connectionLabel, size: 16)
                Text([untrusted(grant.connectionLabel), grant.service.isEmpty ? "" : serviceName(grant.service)].filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
            }
            .padding(.top, 6)
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 20)
        .padding(.top, 6)
    }

    /// One line of what the grant allows (a green check) or never includes (a red ban).
    private func allowLine(_ text: String, allowed: Bool) -> some View {
        HStack(spacing: 10) {
            Image(systemName: allowed ? "checkmark" : "nosign")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(allowed ? Palette.success : Palette.danger)
                .frame(width: 18)
            Text(text)
                .font(RFont.sans(15))
                .foregroundStyle(allowed ? Palette.text : Palette.tertiary)
                .environment(\.layoutDirection, .leftToRight)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(allowed ? text : "Never: \(text)")
    }

    private func ends(_ grant: GrantView) -> String {
        guard let end = grant.expiresAt else { return grant.active ? "Never on its own" : "" }
        return TimeText.dateTime(end)
    }

    @ViewBuilder private func dock(_ grant: GrantView) -> some View {
        HStack(spacing: 10) {
            if grant.active {
                Button {
                    extending = GrantPick(grant)
                } label: {
                    Label("Extend", systemImage: "clock")
                }
                .buttonStyle(DockButtonStyle(kind: .neutral))
                .disabled(busy)
                .accessibilityIdentifier("extend")
                Button(role: .destructive) {
                    confirmingRevoke = true
                } label: {
                    Label("End now", systemImage: "nosign")
                }
                .buttonStyle(DockButtonStyle(kind: .danger))
                .disabled(busy)
                .accessibilityIdentifier("revoke")
            } else {
                Button {
                    resuming = GrantPick(grant)
                } label: {
                    Label("Resume", systemImage: "arrow.clockwise")
                }
                .buttonStyle(DockButtonStyle(kind: .light))
                .disabled(busy)
                .accessibilityIdentifier("resume")
                Button(role: .destructive) {
                    confirmingDelete = true
                } label: {
                    Label("Delete", systemImage: "trash")
                }
                .buttonStyle(DockButtonStyle(kind: .danger))
                .disabled(busy)
                .accessibilityIdentifier("deleteEnded")
            }
        }
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
