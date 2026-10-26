import SwiftUI

/// One grant (the Trust board, 02): a violet field, since it is power already handed over: who it is for beside the
/// back button, the time left with a ring that drains and the service's icon, what it allows in the display face;
/// then what it never does, its uses, end, account and where it came from, what it was used for; and Extend with Hold
/// to end, or Resume and Delete once it ended.
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

                    // What it allows beyond its sentence, when the core spells it out.
                    if grant.lines.count > 1 {
                        SectionHeader("Can", glyph: .check)
                        RaisedGroup(inset: 46) {
                            ForEach(Array(grant.lines.enumerated()), id: \.offset) { _, line in
                                GrantFact(glyph: .check, text: untrusted(line))
                            }
                        }
                    }

                    if !never.isEmpty {
                        SectionHeader("Never", glyph: .close)
                        RaisedGroup(inset: 46) {
                            ForEach(Array(never.enumerated()), id: \.offset) { _, line in
                                GrantFact(glyph: GrantFact.glyph(never: line), text: line)
                                    .accessibilityLabel("Never: \(line)")
                            }
                        }
                    }

                    RaisedGroup(inset: 46) {
                        GrantFact(glyph: .card, text: GrantFactText.uses(grant))
                        TimelineView(.periodic(from: .now, by: 30)) { _ in
                            GrantFact(glyph: .timer, text: GrantFactText.ends(grant))
                        }
                        if let account = grant.account, !account.trimmingCharacters(in: .whitespaces).isEmpty {
                            GrantFact(glyph: .person, text: untrusted(account), mono: true)
                        }
                        GrantFact(glyph: .circleCheck, text: GrantText.origin(grant.origin, label: grant.connectionLabel))
                    }
                    .padding(.top, 12)

                    // A permission for secrets: its values may still be held on computers, so they can be told to drop them.
                    if grant.action == "secrets", let project = grant.secretsProject {
                        RevokeEverywhereSection(project: project, env: grant.secretsEnv).padding(.top, 12)
                    }

                    SectionHeader("Used for", trailing: used.isEmpty ? nil : "\(used.count)")
                    if used.isEmpty {
                        Text("Nothing yet").textRole(.body).foregroundStyle(Palette.inkMeta).padding(.horizontal, 24).padding(.vertical, 4)
                    } else {
                        RaisedGroup(inset: 62) {
                            ForEach(used.prefix(10), id: \.id) { entry in
                                Button {
                                    feedback.play(.tap)
                                    model.push(.activityDetail(entry.id))
                                } label: {
                                    RunningEntryRow(entry: entry, outcome: .grant)
                                }
                                .buttonStyle(PressDimsStyle())
                                .accessibilityIdentifier("entry:\(entry.id)")
                            }
                        }
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

    @ViewBuilder private func dock(_ grant: GrantView) -> some View {
        HStack(spacing: 10) {
            if grant.active {
                Button {
                    extending = GrantPick(grant)
                } label: {
                    Label { Text("Extend") } icon: { GlyphIcon(.timer, size: 18) }
                }
                .buttonStyle(SignalButtonLook(kind: .outline))
                .frame(height: 54)
                .disabled(busy)
                .accessibilityIdentifier("extend")
                // The grant's power goes at once: held, never a tap, no dialog.
                HoldButton(title: "Hold to end", busy: busy) {
                    run(leave: true) { await model.revokeGrant(grant.id) }
                }
                .layoutPriority(1)
                .accessibilityIdentifier("revoke")
            } else {
                Button {
                    resuming = GrantPick(grant)
                } label: {
                    Label { Text("Resume") } icon: { GlyphIcon(.refresh, size: 18) }
                }
                .buttonStyle(SignalButtonLook(kind: .soft))
                .frame(height: 54)
                .disabled(busy)
                .accessibilityIdentifier("resume")
                Button(role: .destructive) {
                    confirmingDelete = true
                } label: {
                    Label { Text("Delete") } icon: { GlyphIcon(.trash, size: 18) }
                }
                .buttonStyle(SignalButtonLook(kind: .danger))
                .frame(height: 54)
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

/// What a grant's fact rows say.
enum GrantFactText {
    /// "12 uses", "3 of 5 uses", "Not used yet".
    static func uses(_ g: GrantView) -> String {
        if let cap = g.maxUses { return "\(g.uses) of \(cap) uses" }
        switch g.uses {
        case 0: return "Not used yet"
        case 1: return "1 use"
        default: return "\(g.uses) uses"
        }
    }

    /// "Ends Oct 25, 3:50 PM", "Never ends on its own", "Expired Oct 25, 3:50 PM", "Used up".
    static func ends(_ g: GrantView) -> String {
        guard let end = g.expiresAt else { return g.active ? "Never ends on its own" : GrantText.inactiveWord(g) }
        return "\(g.active ? "Ends" : GrantText.inactiveWord(g)) \(TimeText.dateTime(end))"
    }
}

/// A fact on a grant's page (the boards' `fact`): a glyph and a line.
struct GrantFact: View {
    var glyph: Glyph
    var text: String
    var mono = false

    /// The glyph of what a grant never does: sending, codes and passwords, reading, else a ban.
    static func glyph(never line: String) -> Glyph {
        let l = line.lowercased()
        if l.contains("code") || l.contains("password") { return .vault }
        if l.contains("send") { return .send }
        if l.contains("read") || l.contains("open") { return .eye }
        return .block
    }

    var body: some View {
        HStack(spacing: 14) {
            GlyphIcon(glyph, size: 18).foregroundStyle(Palette.ink2)
            Text(verbatim: text)
                .font(mono ? RFont.mono(14.5) : TextRole.row.font(target: false))
                .foregroundStyle(Palette.ink)
                .environment(\.layoutDirection, .leftToRight)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(minHeight: 50)
        .accessibilityElement(children: .combine)
    }
}

/// The grant's field: who it is for (beside the back button), the time left with its ring and the service's icon, and
/// the core's summary in the display face. An ended grant's field is dimmed.
private struct GrantField: View {
    var grant: GrantView
    /// The status bar's and the bar's height: the field runs under them.
    var top: CGFloat

    var body: some View {
        SignalField(kind: .grant) {
            VStack(alignment: .leading, spacing: 0) {
                // Who: the agent's avatar and name, one short clean line, in the bar's row after the back button.
                HStack(spacing: 10) {
                    ConnectionIcon(connectionId: grant.connectionId, label: grant.connectionLabel, size: 28)
                    Text(verbatim: untrustedName(grant.connectionLabel))
                        .font(RFont.sans(15, .semibold))
                        .lineLimit(1)
                }
                .frame(height: 44)
                .padding(.leading, 48)
                .padding(.top, max(top - 50, 0))
                .accessibilityElement(children: .combine)
                .accessibilityLabel("For \(untrustedName(grant.connectionLabel))")
                TimelineView(.periodic(from: .now, by: 1)) { ctx in
                    let clock = grantClock(grant, now: nowSeconds(ctx.date))
                    HStack(alignment: .center, spacing: 14) {
                        Text(grant.active ? (clock.remaining.map(compactDuration) ?? "\u{221E}") : GrantText.inactiveWord(grant))
                            .font(RFont.display(grant.active ? 64 : 40))
                            .tracking(RFont.displayTracking(64))
                            .monospacedDigit()
                            .lineLimit(1)
                            .minimumScaleFactor(0.6)
                            .accessibilityLabel(grant.active ? (clock.remaining.map { "\(compactDuration($0)) left" } ?? "No time limit") : GrantText.inactiveWord(grant))
                            .accessibilityIdentifier("grantClock")
                        if grant.active {
                            ArcRing(fraction: clock.fraction, color: Palette.onField, track: Palette.onField.opacity(0.25), lineWidth: 3.2)
                                .frame(width: 30, height: 30)
                                .accessibilityHidden(true)
                        }
                        Spacer(minLength: 8)
                        RequestIcon(spec: AskIcon(mark: .service(grant.service.isEmpty ? "gmail" : grant.service), action: .key, kind: .grant), size: 38, ring: Palette.fieldGrant)
                    }
                    .foregroundStyle(Palette.onField)
                }
                .padding(.top, 18)
                AskText(GrantWords.runs(grant.summary), role: .ask, lineLimit: 4)
                    .foregroundStyle(Palette.onField)
                    .padding(.top, 8)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(untrusted(grant.summary))
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("grantTitle")
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 24)
        }
        .opacity(grant.active ? 1 : 0.62)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("grantField")
    }
}
