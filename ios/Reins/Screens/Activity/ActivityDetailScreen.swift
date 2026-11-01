import SwiftUI

/// One operation, opened (`design/next/iphone`, Next-Activity): decided means paper, so the entry's own header (its kind tile, the line
/// in the display face, the verdict) stands where the field was. An entry that kept its approval then shows the approval's
/// facts and pages, read-only, what was asked in full below and "This was wrong" where the decision dock was. An older entry shows what it kept: who asked, what exactly, what was released or sent and to whom, who
/// decided, and the grant that covered it.
struct ActivityDetailScreen: View {
    var entryId: Int64
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    /// The approval as it was shown, read once; nil for older entries.
    @State private var recorded: ApprovalModel?
    /// A push's changes as this iPhone has them, and those loaded from GitHub; nil for anything but a push.
    @State private var diffs: PushDiffs?
    /// Until the core has answered nothing is drawn, so an entry with its approval never flashes the older page first.
    @State private var looked = false
    /// Set once the core has said it has no such entry (or could not be read); until then a missing entry is read.
    @State private var missing: EntryMissing?

    var body: some View {
        let entry = model.entry(entryId)
        Group {
            if let entry, let vm = recorded, let view = vm.view {
                EntryApprovalPage(entry: entry, vm: vm, view: view, diffs: diffs) {
                    AskedSections(entry: entry, includeCommand: false)
                    extras(entry)
                    // A project's environment released to a computer: its values can be dropped everywhere.
                    if let release = view.envRelease {
                        RevokeEverywhereSection(project: release.project, env: release.env).padding(.top, 14)
                    }
                }
            } else if let entry {
                if looked {
                    // No action bar: a past event needs none ("This was wrong" is the last of Autopilot's reasons).
                    ScrollView { content(entry).padding(.bottom, 24) }
                } else {
                    Color.clear
                }
            } else if case let .failed(message) = missing {
                EmptyState(symbol: "list.bullet", title: "Can't open it", message: message)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("entryError")
            } else if missing == .gone {
                EmptyState(symbol: "list.bullet", title: "Not found")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("entryGone")
            } else {
                // Older than the history read at once: read from the core by its id.
                Color.clear
            }
        }
        // Read again once the entry is there (a link can open it before the history has loaded).
        .task(id: "\(entryId):\(entry != nil)") { await lookUp() }
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
                                .foregroundStyle(Palette.ink)
                                .lineLimit(1)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    ShareLink(item: summary(entry)) { GlyphIcon(.share, size: 20) }
                        .accessibilityLabel("Share")
                }
            }
        }
    }

    /// Reads the approval the entry kept. Without one (an older entry, or the core could not say) the entry shows what
    /// it kept itself.
    private func lookUp() async {
        if model.entry(entryId) == nil {
            // Once read it is kept, so this task runs again with the entry (its id changes).
            missing = await EntryMissing.read(entryId, model: model)
            return
        }
        let shown = try? await model.core.activityApproval(entryId: entryId)
        if Task.isCancelled { return }
        if let shown, let entry = model.entry(entryId) {
            // What was shared is ticked: the emails' ids and the addresses the entry released.
            let released = Set(entry.info.messages.map(\.id).filter { !$0.isEmpty } + entry.info.accounts)
            recorded = ApprovalModel(recorded: shown, released: released)
            // A push decided on another device may have come without its changes line by line.
            if shown.git != nil, let push = try? await model.core.activityPush(entryId: entryId), !Task.isCancelled {
                diffs = PushDiffs(entryId: entryId, notKept: push.diffsNotKept)
            }
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
                    if entry.grantId != nil { Hairline(inset: 48) }
                    AutopilotWhyRow(note: note, title: EntryOutcome.whyTitle(entry), entry: entry)
                    if let matrix = note.matrix {
                        Hairline(inset: 48)
                        MatrixLine(matrix: matrix)
                    }
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

            // The facts: when (said once, under "What it asked" when the request was kept), the account, the service,
            // the grant that covered it, and Autopilot's reasons.
            let when = entry.request == nil
            let account = entry.account.flatMap { $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 }
            let service = !entry.service.isEmpty
            if when || account != nil || service || entry.grantId != nil || entry.autopilot != nil {
                GroupCard {
                    if when {
                        GlyphFact(glyph: .timer, value: AskedText.dayTime(entry.at))
                    }
                    if let account {
                        if when { Hairline(inset: 48) }
                        GlyphFact(glyph: .person, value: account, mono: true)
                    }
                    if service {
                        if when || account != nil { Hairline(inset: 48) }
                        GlyphFact(glyph: ActivitySearchText.glyph(service: entry.service), value: serviceName(entry.service))
                    }
                    if let id = entry.grantId {
                        if when || account != nil || service { Hairline(inset: 48) }
                        coveredBy(id)
                    }
                    if let note = entry.autopilot {
                        if when || account != nil || service || entry.grantId != nil { Hairline(inset: 48) }
                        AutopilotWhyRow(note: note, title: EntryOutcome.whyTitle(entry), entry: entry)
                        if let matrix = note.matrix {
                            Hairline(inset: 48)
                            MatrixLine(matrix: matrix)
                        }
                    }
                }
                .padding(.top, 20)
            }

            if let query = info.query {
                GroupCard(header: "Search", glyph: .search) { ListRow(title: untrusted(query)).accessibilityIdentifier("detailQuery") }
            }
            if let email = info.email {
                GroupCard(header: "Email", glyph: .mail) {
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
        GroupCard(header: entry.outcome == "denied" ? "\(noun) that were not shared" : ActivityText.done(entry) ? "\(noun) shared (\(list.count))" : "\(noun) requested (\(list.count))") {
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
        GroupCard(header: entry.outcome == "denied" ? "Accounts that were not shared" : ActivityText.done(entry) ? "Accounts shared (\(list.count))" : "Accounts requested (\(list.count))") {
            ForEach(Array(list.enumerated()), id: \.offset) { i, address in
                if i > 0 { Hairline(inset: 68) }
                HStack(spacing: 14) {
                    BlobAvatar(seed: address, size: 36)
                    Text(address)
                        .font(RFont.sans(15.5, .medium))
                        .foregroundStyle(Palette.ink)
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
        GlyphFact(glyph: .key, value: grant.map { untrusted($0.summary) } ?? "Grant ended", glyphTint: Palette.violet, action: grant == nil ? nil : {
            feedback.play(.tap)
            model.push(.grantDetail(id))
        })
        .accessibilityIdentifier("openGrantFromEntry")
    }

    /// The entry in words, for sharing.
    private func summary(_ entry: ActivityEntry) -> String {
        [entry.headline, untrusted(entry.detail), "\(HoldText.outcome(entry.outcome, hold: entry.hold) ?? OutcomeTag.word(entry.outcome)) · \(TimeText.full(entry.at))"]
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
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                    .environment(\.layoutDirection, .leftToRight)
                if !message.subject.isEmpty {
                    Text(untrusted(message.subject)).font(RFont.sans(14.5)).foregroundStyle(Palette.ink).lineLimit(2)
                }
                // Another integration keeps what was shared, so it can be read here without asking anyone.
                if !message.text.isEmpty {
                    Text(untrusted(message.text)).font(RFont.sans(14)).foregroundStyle(Palette.ink2)
                }
                if message.date != 0 {
                    Text(TimeText.dateTime(message.date)).font(RFont.sans(12)).foregroundStyle(Palette.inkMeta).padding(.top, 1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if openable {
                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.inkMeta)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .contentShape(Rectangle())
    }
}

/// Why an entry being opened is not shown: the core has no such entry, or it could not be read.
enum EntryMissing: Equatable {
    case gone
    case failed(String)

    /// Reads entry `id` from the core when the app does not have it (`AppModel.loadEntry`): nil once it is there (or
    /// when the read was cancelled), else why it is not.
    @MainActor
    static func read(_ id: Int64, model: AppModel) async -> EntryMissing? {
        do {
            let entry = try await model.loadEntry(id)
            if Task.isCancelled { return nil }
            return entry == nil ? .gone : nil
        } catch {
            return Task.isCancelled || error is CancellationError ? nil : .failed(error.userMessage)
        }
    }
}

#if DEBUG
#Preview {
    PreviewHost { NavigationStack { ActivityDetailScreen(entryId: 14) } }
}
#endif
