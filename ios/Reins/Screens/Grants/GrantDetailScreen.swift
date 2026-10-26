import SwiftUI

/// One grant (iOS-Trust.png, 02): a violet field, since it is power already handed over: its time left in the serif
/// with a ring that drains, what it allows in a sentence, for whom; then what it may and never does, its uses and end as
/// hairline rows, what it was used for; and Extend with End now (held), or Resume and Delete once it ended.
struct GrantDetailScreen: View {
    var grantId: String
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
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
        return GeometryReader { geo in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    GrantField(grant: grant, top: geo.safeAreaInsets.top)

                    if let error {
                        FormBanner(text: error).padding(.horizontal, 16).padding(.top, 14)
                    }

                    // What it may do (a check) and never includes (a ban), as hairline rows.
                    FactGroup {
                        if grant.lines.isEmpty {
                            allowLine("Nothing specific", allowed: true)
                        }
                        ForEach(Array(grant.lines.enumerated()), id: \.offset) { _, line in
                            allowLine(untrusted(line), allowed: true)
                        }
                        ForEach(Array(never.enumerated()), id: \.offset) { _, line in
                            allowLine(line, allowed: false)
                        }
                    }
                    .padding(.top, 12)

                    FactGroup {
                        FactRow(key: "Uses", value: GrantText.usesValue(grant)) {
                            if let max = grant.maxUses { UsesDashes(uses: Int(grant.uses), max: Int(max)) }
                        }
                        TimelineView(.periodic(from: .now, by: 30)) { _ in
                            FactRow(key: grant.active ? "Ends" : GrantText.inactiveWord(grant), value: ends(grant))
                        }
                        if let account = grant.account, !account.trimmingCharacters(in: .whitespaces).isEmpty {
                            FactRow(key: "Account", value: untrusted(account), mono: true)
                        }
                        FactRow(key: "Came from", value: GrantText.origin(grant.origin, label: grant.connectionLabel))
                    }
                    .padding(.top, 14)

                    // A permission for secrets: its values may still be held on computers, so they can be told to drop them.
                    if grant.action == "secrets", let project = grant.secretsProject {
                        RevokeEverywhereSection(project: project, env: grant.secretsEnv).padding(.top, 12)
                    }

                    Text(used.isEmpty ? "Used for" : "Used for · \(used.count)")
                        .textRole(.section)
                        .foregroundStyle(Palette.inkMeta)
                        .padding(.horizontal, 24)
                        .padding(.top, 26)
                        .padding(.bottom, 4)
                    if used.isEmpty {
                        Text("Nothing yet").textRole(.body).foregroundStyle(Palette.inkMeta).padding(.horizontal, 24).padding(.vertical, 8)
                    }
                    ForEach(used.prefix(10), id: \.id) { entry in
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
                .padding(.bottom, 24)
            }
            .ignoresSafeArea(edges: .top)
            .scrollEdgeEffectHidden(true, for: .top)
        }
        .pageBackground()
        // The bar stays for its back button and the edge swipe, clear over the field, its button white.
        .toolbarBackgroundVisibility(.hidden, for: .navigationBar)
        .toolbarColorScheme(.dark, for: .navigationBar)
        .pageDock { dock(grant) }
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

    /// One line of what the grant allows (a green check) or never includes (a red ban).
    private func allowLine(_ text: String, allowed: Bool) -> some View {
        HStack(alignment: .top, spacing: 10) {
            GlyphIcon(allowed ? .check : .block, size: 16)
                .foregroundStyle(allowed ? Palette.success : Palette.danger)
                .frame(width: 18)
                .padding(.top, 1)
            Text(text)
                .font(RFont.sans(15))
                .foregroundStyle(allowed ? Palette.ink : Palette.inkMeta)
                .environment(\.layoutDirection, .leftToRight)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 18)
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
                    Label("Extend", systemImage: "timer")
                }
                .buttonStyle(SignalButtonLook(kind: .outline))
                .frame(height: 54)
                .disabled(busy)
                .accessibilityIdentifier("extend")
                // The grant's power goes at once: held, never a tap, no dialog.
                HoldButton(title: "End now", busy: busy) {
                    run(leave: true) { await model.revokeGrant(grant.id) }
                }
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

/// The grant's field: "Grant · Gmail", the time left ("5h *left*") with its ring, the core's summary in the serif, and
/// who it is for. An ended grant's field is dimmed.
private struct GrantField: View {
    var grant: GrantView
    /// The status bar's and the bar's height: the field runs under them.
    var top: CGFloat

    var body: some View {
        SignalField(kind: .grant) {
            VStack(alignment: .leading, spacing: 0) {
                Text("Grant · \(grant.service.isEmpty ? "Reins" : serviceName(grant.service))")
                    .textRole(.section)
                    .foregroundStyle(Palette.onField2)
                    .lineLimit(1)
                TimelineView(.periodic(from: .now, by: 1)) { ctx in
                    let clock = grantClock(grant, now: nowSeconds(ctx.date))
                    HStack(alignment: .center) {
                        HStack(alignment: .lastTextBaseline, spacing: 6) {
                            Text(grant.active ? GrantText.ringLabel(clock).replacingOccurrences(of: " ", with: "") : GrantText.inactiveWord(grant))
                                .font(RFont.display(grant.active ? 38 : 30))
                            if grant.active && clock.remaining != nil {
                                Text("left").font(RFont.display(18, target: true))
                            }
                        }
                        .lineLimit(1)
                        .minimumScaleFactor(0.6)
                        Spacer(minLength: 8)
                        if grant.active {
                            ArcRing(fraction: clock.fraction, color: Palette.onField, track: Palette.onField.opacity(0.25), lineWidth: 3)
                                .frame(width: 30, height: 30)
                        }
                    }
                    .foregroundStyle(Palette.onField)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(grant.active ? (clock.remaining.map { "\(compactDuration($0)) left" } ?? "No time limit") : GrantText.inactiveWord(grant))
                    .accessibilityIdentifier("grantClock")
                }
                .padding(.top, 18)
                Text(verbatim: untrusted(grant.summary))
                    .font(RFont.display(26.5))
                    .lineHeight(.multiple(factor: 38 / 36))
                    .foregroundStyle(Palette.onField)
                    .lineLimit(4)
                    .minimumScaleFactor(0.78)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 6)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("grantTitle")
                HStack(spacing: 6) {
                    ConnectionIcon(connectionId: grant.connectionId, label: grant.connectionLabel, size: 18)
                        .environment(\.colorScheme, .light)
                    Text(verbatim: [untrustedName(grant.connectionLabel), grant.account.map { untrusted($0) } ?? ""].filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(RFont.sans(13.5))
                        .foregroundStyle(Palette.onField2)
                        .lineLimit(1)
                }
                .padding(.top, 12)
            }
            .padding(.horizontal, 24)
            .padding(.top, top + 8)
            .padding(.bottom, 24)
        }
        .opacity(grant.active ? 1 : 0.62)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("grantField")
    }
}

/// How much of a grant's uses are spent, one dash per use (ten at most drawn).
private struct UsesDashes: View {
    var uses: Int
    var max: Int

    var body: some View {
        let shown = min(max, 10)
        let filled = max == 0 ? 0 : Int((Double(min(uses, max)) / Double(max) * Double(shown)).rounded(.up))
        HStack(spacing: 3) {
            ForEach(0..<shown, id: \.self) { i in
                Capsule().fill(i < filled ? Palette.violet : Palette.rule).frame(width: 10, height: 3)
            }
        }
        .accessibilityHidden(true)
    }
}
