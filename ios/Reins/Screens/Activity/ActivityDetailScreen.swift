import SwiftUI

/// One operation, opened: who asked, what exactly, what was released or sent and to whom, who decided, and the
/// grant that covered it.
struct ActivityDetailScreen: View {
    var entryId: Int64
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    /// "This was wrong" stays once shown, so its thanks can replace it after the correction.
    @State private var dockShown = false

    var body: some View {
        let entry = model.activity.first(where: { $0.id == entryId })
        Group {
            if let entry {
                ScrollView { content(entry).padding(.bottom, 24) }
                    .pageDock(when: dockShown) { ThisWasWrongDock(entry: entry) }
            } else {
                EmptyState(symbol: "list.bullet", title: "Not found")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("entryGone")
            }
        }
        .onChange(of: entry.map(ThisWasWrongDock.offered) ?? false, initial: true) { _, offered in
            if offered { dockShown = true }
        }
        .pageBackground()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let entry {
                ToolbarItem(placement: .topBarTrailing) {
                    ShareLink(item: summary(entry)) { Image(systemName: "square.and.arrow.up") }
                        .accessibilityLabel("Share")
                }
            }
        }
    }

    @ViewBuilder private func content(_ entry: ActivityEntry) -> some View {
        let info = entry.info
        VStack(alignment: .leading, spacing: 0) {
            hero(entry)
                .padding(.horizontal, 12)
                .padding(.top, 4)

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
                    AutopilotWhyRow(note: note, title: EntryOutcome.whyTitle(entry))
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

    /// The approval hero, after the fact: the AI, three dots, the service; the sentence; how it ended.
    private func hero(_ entry: ActivityEntry) -> some View {
        let kind = ActionKind.of(entry.action)
        let tint = KindTint.of(kind)
        let shape = RoundedRectangle(cornerRadius: 28, style: .continuous)
        let outcome = EntryOutcome.tint(entry)
        return VStack(spacing: 0) {
            HStack(spacing: 14) {
                ConnectionIcon(connectionId: entry.connectionId, label: entry.connectionLabel, size: 52)
                HStack(spacing: 5) {
                    Circle().fill(Palette.tertiary).frame(width: 5, height: 5)
                    Circle().fill(Palette.tertiary).frame(width: 5, height: 5)
                    Circle().fill(tint).frame(width: 5, height: 5)
                }
                ServiceTile(service: entry.service, symbol: kind.symbol, size: 52)
            }
            .accessibilityHidden(true)
            Text(EntryText.title(entry))
                .font(RFont.sans(23, .semibold))
                .foregroundStyle(Palette.text)
                .multilineTextAlignment(.center)
                .lineLimit(3)
                .minimumScaleFactor(0.8)
                .padding(.top, 16)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("detailTitle")
            if !entry.detail.isEmpty {
                Text(untrusted(entry.detail))
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.secondary)
                    .multilineTextAlignment(.center)
                    .lineLimit(4)
                    .padding(.top, 6)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("detailSummary")
            }
            HStack(spacing: 6) {
                Image(systemName: EntryOutcome.symbol(entry)).font(.system(size: 12, weight: .semibold))
                Text(EntryOutcome.text(entry)).font(RFont.sans(13, .semibold))
            }
            .foregroundStyle(outcome)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(outcome.opacity(0.16), in: Capsule())
            .padding(.top, 14)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("entryOutcome")
            Text(untrusted(entry.connectionLabel))
                .font(RFont.sans(13))
                .foregroundStyle(Palette.tertiary)
                .lineLimit(1)
                .padding(.top, 10)
        }
        .padding(.horizontal, 20)
        .padding(.top, 22)
        .padding(.bottom, 18)
        .frame(maxWidth: .infinity)
        .background(KindTint.wash(tint), in: shape)
        .overlay(shape.strokeBorder(tint.opacity(0.24), lineWidth: 1))
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
