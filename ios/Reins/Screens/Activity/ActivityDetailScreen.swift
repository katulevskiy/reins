import SwiftUI

/// One operation, opened: who asked, what exactly, what was released or sent and to whom, who decided, and the
/// grant that covered it.
struct ActivityDetailScreen: View {
    var entryId: Int64
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        Group {
            if let entry = model.activity.first(where: { $0.id == entryId }) {
                ScrollView { content(entry).padding(.bottom, 32) }
            } else {
                EmptyState(symbol: "list.bullet", title: "Not found", message: "That entry is no longer in the history.")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("entryGone")
            }
        }
        .pageBackground()
        .navigationTitle("Details")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let entry = model.activity.first(where: { $0.id == entryId }) {
                ToolbarItem(placement: .topBarTrailing) {
                    ShareLink(item: summary(entry)) { Image(systemName: "square.and.arrow.up") }
                        .accessibilityLabel("Share")
                }
            }
        }
    }

    @ViewBuilder private func content(_ entry: ActivityEntry) -> some View {
        let action = ActionKind.of(entry.action)
        let info = entry.info
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    ConnectionIcon(connectionId: entry.connectionId, label: entry.connectionLabel, size: 40)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(untrusted(entry.connectionLabel)).font(RFont.sans(17, .semibold)).foregroundStyle(Palette.text).lineLimit(1)
                        Text(TimeText.full(entry.at)).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary).lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    OutcomeTag(outcome: entry.outcome)
                }
                HStack(spacing: 14) {
                    ActionTile(kind: action, count: Int(entry.count), size: 48)
                    Text(title(entry))
                        .font(RFont.sans(26, .semibold))
                        .foregroundStyle(Palette.text)
                        .lineLimit(2)
                        .minimumScaleFactor(0.8)
                        .accessibilityAddTraits(.isHeader)
                        .accessibilityIdentifier("detailTitle")
                }
                .padding(.top, 16)
                ConnectorTags(service: entry.service, account: entry.account).padding(.top, 12)
                if !entry.detail.isEmpty {
                    Text(untrusted(entry.detail))
                        .font(RFont.sans(15.5))
                        .foregroundStyle(Palette.secondary)
                        .lineSpacing(2)
                        .padding(.top, 12)
                        .textSelection(.enabled)
                        .accessibilityIdentifier("detailSummary")
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 8)

            EntryAutopilotSection(entry: entry)

            if let query = info.query {
                GroupCard(header: "Search") { ListRow(title: untrusted(query)).accessibilityIdentifier("detailQuery") }
            }
            if let email = info.email {
                GroupCard(header: "The email") {
                    EmailPreview(email: email, framed: false).accessibilityIdentifier("detailEmail")
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
            if let id = entry.grantId { coveredBy(id) }
        }
    }

    private func title(_ entry: ActivityEntry) -> String {
        if ActionKind.of(entry.action) == .grant {
            let full = entryTitle(label: "", action: entry.action, count: 1, service: entry.service, outcome: entry.outcome)
            return full.hasPrefix(": ") ? String(full.dropFirst(2)) : full
        }
        return operationTitle(action: entry.action, count: Int(entry.count), service: entry.service, title: entry.opTitle, op: entry.op)
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
        GroupCard(header: "Covered by") {
            ListRow(
                title: grant != nil ? "A standing grant" : "A grant that no longer exists",
                subtitle: grant.map { untrusted($0.summary) },
                symbol: "key.horizontal",
                chevron: grant != nil,
                action: grant == nil ? nil : {
                    feedback.play(.tap)
                    model.push(.grantDetail(id))
                }
            )
            .accessibilityIdentifier("openGrantFromEntry")
        }
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

#Preview {
    PreviewHost { NavigationStack { ActivityDetailScreen(entryId: 14) } }
}
