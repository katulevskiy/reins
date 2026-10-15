import SwiftUI

/// One operation, opened (iOS-Activity.png, 05): decided means paper, so the entry's own header (its kind tile, the line
/// in the serif, the verdict) stands where the field was. An entry that kept its approval then shows the approval's
/// facts and pages, read-only, what was asked in full below and "This was wrong" where the decision dock was. An older entry shows what it kept: who asked, what exactly, what was released or sent and to whom, who
/// decided, and the grant that covered it.
struct ActivityDetailScreen: View {
    var entryId: Int64
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    /// "This was wrong" stays once shown, so its thanks can replace it after the correction.
    @State private var dockShown = false
    /// The approval as it was shown, read once; nil for older entries.
    @State private var recorded: ApprovalModel?
    /// Until the core has answered nothing is drawn, so an entry with its approval never flashes the older page first.
    @State private var looked = false

    var body: some View {
        let entry = model.entry(entryId)
        Group {
            if let entry, let vm = recorded, let view = vm.view {
                EntryApprovalPage(entry: entry, vm: vm, view: view) {
                    AskedSections(entry: entry)
                    extras(entry)
                    // A project's environment released to a computer: its values can be dropped everywhere.
                    if let release = view.envRelease {
                        RevokeEverywhereSection(project: release.project, env: release.env).padding(.top, 14)
                    }
                }
                .pageDock(when: dockShown) { ThisWasWrongDock(entry: entry) }
            } else if let entry {
                if looked {
                    ScrollView { content(entry).padding(.bottom, 24) }
                        .pageDock(when: dockShown) { ThisWasWrongDock(entry: entry) }
                } else {
                    Color.clear
                }
            } else {
                EmptyState(symbol: "list.bullet", title: "Not found")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("entryGone")
            }
        }
        // Read again once the entry is there (a link can open it before the history has loaded).
        .task(id: "\(entryId):\(entry != nil)") { await lookUp() }
        .onChange(of: entry.map(ThisWasWrongDock.offered) ?? false, initial: true) { _, offered in
            if offered { dockShown = true }
        }
        .pageBackground()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let entry {
                if recorded != nil {
                    // Who asked, as the approval's own bar has it.
                    ToolbarItem(placement: .principal) {
                        HStack(spacing: 8) {
                            ConnectionIcon(connectionId: entry.connectionId, label: entry.connectionLabel, size: 22)
                            Text(untrusted(entry.connectionLabel))
                                .font(RFont.sans(15, .semibold))
                                .foregroundStyle(Palette.text)
                                .lineLimit(1)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    ShareLink(item: summary(entry)) { Image(systemName: "square.and.arrow.up") }
                        .accessibilityLabel("Share")
                }
            }
        }
    }

    /// Reads the approval the entry kept. Without one (an older entry, or the core could not say) the entry shows what
    /// it kept itself.
    private func lookUp() async {
        let shown = try? await model.core.activityApproval(entryId: entryId)
        if Task.isCancelled { return }
        if let shown, let entry = model.entry(entryId) {
            // What was shared is ticked: the emails' ids and the addresses the entry released.
            let released = Set(entry.info.messages.map(\.id).filter { !$0.isEmpty } + entry.info.accounts)
            recorded = ApprovalModel(recorded: shown, released: released)
        }
        looked = true
    }

    /// Under an entry shown with its approval: the grant that covered it and Autopilot's reasons, when there are any.
    @ViewBuilder private func extras(_ entry: ActivityEntry) -> some View {
        if entry.grantId != nil || entry.autopilot != nil {
            GroupCard {
                if let id = entry.grantId {
                    coveredBy(id)
                }
                if let note = entry.autopilot {
                    if entry.grantId != nil { Hairline() }
                    AutopilotWhyRow(note: note, title: EntryOutcome.whyTitle(entry), entry: entry)
                }
            }
            .padding(.top, 14)
        }
    }

    @ViewBuilder private func content(_ entry: ActivityEntry) -> some View {
        let info = entry.info
        VStack(alignment: .leading, spacing: 0) {
            DecidedHeader(entry: entry)

            AskedSections(entry: entry)

            GroupCard {
                ValueRow(label: "When", value: TimeText.dateTime(entry.at))
                if let account = entry.account, !account.trimmingCharacters(in: .whitespaces).isEmpty {
                    Hairline()
                    ValueRow(label: "Account", value: account, mono: true)
                }
                if !entry.service.isEmpty {
                    Hairline()
                    ValueRow(label: "Service", value: serviceName(entry.service))
                }
                if let id = entry.grantId {
                    Hairline()
                    coveredBy(id)
                }
                if let note = entry.autopilot {
                    Hairline()
                    AutopilotWhyRow(note: note, title: EntryOutcome.whyTitle(entry), entry: entry)
                }
            }
            .padding(.top, 14)

            if let query = info.query {
                GroupCard(header: "Search") { ListRow(title: untrusted(query)).accessibilityIdentifier("detailQuery") }
            }
            if let email = info.email {
                GroupCard(header: "The email") {
                    EmailPreview(email: email, framed: false).accessibilityContainer("detailEmail")
                }
            }
            if !info.messages.isEmpty { messages(entry) }
            if !info.accounts.isEmpty { accounts(entry) }
            if let summary = info.grantSummary.map(untrusted) {
                GroupCard(header: "Permission") { ListRow(title: summary) }
            }
            if let note = info.note.map(untrusted) {
                GroupCard(header: "Note") { ListRow(title: note) }
            }
        }
    }

    @ViewBuilder private func messages(_ entry: ActivityEntry) -> some View {
        let list = entry.info.messages
        let other = !entry.op.isEmpty
        let noun = other ? "Items" : "Emails"
        GroupCard(header: entry.outcome == "denied" ? "\(noun) that were not shared" : "\(noun) shared (\(list.count))") {
            ForEach(Array(list.enumerated()), id: \.offset) { i, m in
                if i > 0 { Hairline() }
                // Only the id is kept, never the text: opening an email fetches it from Gmail again.
                let openable = !m.id.isEmpty && !other
                let row = MessageLine(message: m, openable: openable)
                if openable {
                    Button {
                        feedback.play(.tap)
                        model.push(.email(entryId: entry.id, index: i))
                    } label: { row }
                        .buttonStyle(RowButtonStyle())
                        .accessibilityIdentifier("detailMessage:\(i)")
                } else {
                    row.accessibilityIdentifier("detailMessage:\(i)")
                }
            }
        }
    }

    @ViewBuilder private func accounts(_ entry: ActivityEntry) -> some View {
        let list = entry.info.accounts
        GroupCard(header: entry.outcome == "denied" ? "Accounts that were not shared" : "Accounts shared (\(list.count))") {
            ForEach(Array(list.enumerated()), id: \.offset) { i, address in
                if i > 0 { Hairline(inset: 68) }
                HStack(spacing: 14) {
                    BlobAvatar(seed: address, size: 36)
                    Text(address)
                        .font(RFont.sans(15.5, .medium))
                        .foregroundStyle(Palette.text)
                        .lineLimit(1)
                        .environment(\.layoutDirection, .leftToRight)
                        .textSelection(.enabled)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("sharedAccount:\(i)")
            }
        }
    }

    @ViewBuilder private func coveredBy(_ id: String) -> some View {
        let grant = model.grants.first { $0.id == id }
        ValueRow(label: "Grant", value: grant.map { untrusted($0.summary) } ?? "Ended", action: grant == nil ? nil : {
            feedback.play(.tap)
            model.push(.grantDetail(id))
        })
        .accessibilityIdentifier("openGrantFromEntry")
    }

    /// The entry in words, for sharing.
    private func summary(_ entry: ActivityEntry) -> String {
        [entry.headline, untrusted(entry.detail), "\(OutcomeTag.word(entry.outcome)) · \(TimeText.full(entry.at))"]
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }
}

/// One email or item an entry shared: from whom, the subject, the text another integration kept, when.
private struct MessageLine: View {
    var message: ActivityMessage
    var openable: Bool

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                Text(untrusted(message.from))
                    .font(RFont.sans(15, .semibold))
                    .foregroundStyle(Palette.text)
                    .lineLimit(1)
                    .environment(\.layoutDirection, .leftToRight)
                if !message.subject.isEmpty {
                    Text(untrusted(message.subject)).font(RFont.sans(14.5)).foregroundStyle(Palette.text).lineLimit(2)
                }
                // Another integration keeps what was shared, so it can be read here without asking anyone.
                if !message.text.isEmpty {
                    Text(untrusted(message.text)).font(RFont.sans(14)).foregroundStyle(Palette.secondary)
                }
                if message.date != 0 {
                    Text(TimeText.dateTime(message.date)).font(RFont.sans(12)).foregroundStyle(Palette.tertiary).padding(.top, 1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if openable {
                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.tertiary)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .contentShape(Rectangle())
    }
}

#if DEBUG
#Preview {
    PreviewHost { NavigationStack { ActivityDetailScreen(entryId: 14) } }
}
#endif
