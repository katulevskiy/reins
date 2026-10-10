import SwiftUI

/// The first section: what waits for you, then everything your AIs did, newest first. On a regular width this is the
/// content column and an entry opens in the detail column.
struct ActivityScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    /// "Automatic": only what Autopilot, a bypass or Lockdown decided.
    @SceneStorage("activity.automaticOnly") private var automaticOnly = false
    /// The list was placed where the user stopped reading; until then nothing is marked seen.
    @State private var positioned = false
    /// Which rows are on screen; not observed, so scrolling does not draw the whole list again.
    @State private var seen = SeenRows()
    @State private var atTop = true
    @State private var away = false

    private var shown: [ActivityEntry] {
        automaticOnly ? model.activity.filter { !$0.decidedBy.isEmpty } : model.activity
    }

    var body: some View {
        let entries = shown
        let automaticCount = model.activity.filter { !$0.decidedBy.isEmpty }.count
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    header.id("top")
                    banners
                    waiting
                    if automaticCount > 0 || automaticOnly { filters(automaticCount) }
                    if entries.isEmpty {
                        empty
                    } else {
                        SectionHeader(automaticOnly ? "Decided for you" : "Latest")
                            .padding(.leading, 16)
                            .padding(.top, 18)
                            .padding(.bottom, 6)
                        list(entries)
                    }
                }
                .padding(.bottom, 32)
            }
            .accessibilityContainer("activityList")
            .refreshable {
                await model.refreshPending()
                feedback.play(.refresh)
            }
            .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y + $0.contentInsets.top } action: { _, y in
                atTop = y < 24
                let far = y > 900
                if far != away { withAnimation(.smooth) { away = far } }
            }
            .overlay(alignment: .bottom) {
                if away {
                    Button {
                        withAnimation(.smooth) { proxy.scrollTo("top", anchor: .top) }
                        feedback.play(.tap)
                    } label: {
                        Label("Latest", systemImage: "chevron.up")
                            .font(RFont.sans(14, .semibold))
                            .foregroundStyle(Palette.accent)
                            .padding(.horizontal, 16)
                            .padding(.vertical, 10)
                    }
                    .buttonStyle(.plain)
                    .glassEffect(.regular.interactive(), in: Capsule())
                    .padding(.bottom, 16)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                    .accessibilityIdentifier("scrollToLatest")
                }
            }
            .task(id: model.activity.isEmpty) { await position(proxy) }
        }
        .onChange(of: atTop) { markSeenSoon() }
        .pageBackground()
        .rootNavigationBar()
        .navigationTitle("Activity")
    }

    // MARK: Parts

    /// The title with the pills beside it, or under it when the column is too narrow for both (an iPad's content
    /// column).
    private var header: some View {
        ViewThatFits(in: .horizontal) {
            PageHeader(title: "Activity") { pills }
            VStack(alignment: .leading, spacing: 10) {
                PageHeader(title: "Activity") { EmptyView() }
                pills
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 12)
        .padding(.bottom, 4)
    }

    private var pills: some View {
        HStack(spacing: 8) {
            if let autopilot = model.autopilot {
                AutopilotQuickPill(settings: autopilot)
            }
            // The page's Open cue is the sound (GlassPill plays the default tap).
            GlassPill(symbol: "square.grid.2x2", text: "Integrations") {
                model.show(.integrations)
            }
            .accessibilityIdentifier("integrations")
        }
        .fixedSize()
    }

    @ViewBuilder private var banners: some View {
        if model.deviceReplaced {
            Banner("Another phone approves now. Switch back in Settings.", kind: .warning)
                .padding(16)
                .accessibilityIdentifier("replacedBanner")
        }
        if let error = model.registrationError {
            Banner("Not your approval phone yet: \(error)", kind: .error)
                .padding(16)
                .accessibilityIdentifier("registrationBanner")
        }
        NotificationsOffCard()
        ScreenLockBanner(padding: EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
    }

    @ViewBuilder private var waiting: some View {
        if !model.pending.isEmpty {
            SectionHeader("Waiting for you")
                .padding(.leading, 16)
                .padding(.top, 18)
                .padding(.bottom, 6)
            ForEach(Burst.of(model.pending), id: \.connectionId) { burst in
                BurstBar(burst: burst)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 5)
            }
            ForEach(model.pending, id: \.id) { item in
                PendingCard(item: item) {
                    model.openSheet(item.sheetTarget)
                    feedback.defaultTap()
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 5)
                .id("p:\(item.id)")
            }
        }
    }

    private func filters(_ automaticCount: Int) -> some View {
        HStack(spacing: 8) {
            SelectChip(title: "All", selected: !automaticOnly) { automaticOnly = false }
                .accessibilityIdentifier("filter:all")
            SelectChip(title: "Automatic · \(automaticCount)", selected: automaticOnly) { automaticOnly = true }
                .accessibilityIdentifier("filter:automatic")
        }
        .padding(.horizontal, 16)
        .padding(.top, 14)
    }

    @ViewBuilder private var empty: some View {
        if automaticOnly {
            EmptyState(symbol: "sparkles", title: "Nothing automatic yet")
                .accessibilityIdentifier("noAutomatic")
        } else if model.connections.isEmpty {
            // A new account: nothing can ask yet, so say how to connect something.
            VStack(spacing: 10) {
                EmptyState(symbol: "list.bullet", title: "Nothing yet")
                    .accessibilityIdentifier("noActivity")
                Button {
                    model.openSheet(.connectComputer)
                } label: {
                    Label("Connect a computer", systemImage: "desktopcomputer")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 48))
                .accessibilityIdentifier("emptyConnectComputer")
                Button {
                    let address = model.mcpAddress
                    UIPasteboard.general.string = address
                    model.notice = "Copied \(address). In Claude.ai or ChatGPT: Settings, Connectors, add a custom connector and paste it."
                    feedback.play(.copied)
                } label: {
                    Label("Connect Claude.ai or ChatGPT", systemImage: "link")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 48))
                .accessibilityIdentifier("emptyConnectAi")
            }
            .padding(.horizontal, 32)
        } else {
            EmptyState(symbol: "list.bullet", title: "Nothing yet")
                .accessibilityIdentifier("noActivity")
        }
    }

    @ViewBuilder private func list(_ entries: [ActivityEntry]) -> some View {
        let seenId = model.seenActivityId
        let oldestUnseen = entries.lastIndex { $0.id > seenId }
        let selected: Int64? = if case let .activityDetail(id)? = model.path(.activity).first { id } else { nil }
        ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
            Button {
                model.show(.activityDetail(entry.id), in: .activity)
                feedback.defaultTap()
            } label: {
                ActivityRow(entry: entry, selected: selected == entry.id)
            }
            .buttonStyle(RowButtonStyle())
            .contextMenu {
                Button("Open", systemImage: "arrow.up.right") {
                    model.show(.activityDetail(entry.id), in: .activity)
                    feedback.defaultTap()
                }
                Button("Copy", systemImage: "doc.on.doc") {
                    UIPasteboard.general.string = [entry.headline, untrusted(entry.detail)].filter { !$0.isEmpty }.joined(separator: "\n")
                    feedback.play(.copied)
                }
            }
            .onScrollVisibilityChange(threshold: 0.6) { isVisible in
                if isVisible { seen.visible.insert(entry.id) } else { seen.visible.remove(entry.id) }
                markSeenSoon()
            }
            .id("e:\(entry.id)")
            .accessibilityIdentifier("entry:\(entry.id)")
            // Where the entries you have not seen end: everything above this line is new.
            if index == oldestUnseen {
                NewMarker()
            } else if index < entries.count - 1 {
                Hairline(inset: 74)
            }
        }
    }

    // MARK: Seen

    /// Opening the list lands where you stopped reading: at the oldest entry you have not seen, unless something waits.
    private func position(_ proxy: ScrollViewProxy) async {
        guard !positioned, !model.activity.isEmpty else { return }
        let entries = shown
        if let oldest = entries.lastIndex(where: { $0.id > model.seenActivityId }), oldest > 2, model.pending.isEmpty {
            proxy.scrollTo("e:\(entries[oldest - 1].id)", anchor: .top)
            try? await Task.sleep(for: .milliseconds(300))
        }
        positioned = true
        markSeen()
    }

    /// What is on screen counts as seen: scroll up to the newest and the badge clears.
    private func markSeen() {
        seen.settle?.cancel()
        seen.settle = nil
        guard positioned else { return }
        if let newest = seen.visible.max() {
            model.markActivitySeen(newest)
        } else if atTop, let first = shown.first {
            model.markActivitySeen(first.id)
        }
    }

    /// `markSeen` once the scrolling settles, not for every row that passes.
    private func markSeenSoon() {
        guard positioned else { return }
        seen.settle?.cancel()
        seen.settle = Task {
            try? await Task.sleep(for: .milliseconds(300))
            if !Task.isCancelled { markSeen() }
        }
    }
}

/// The rows on screen and the pending "seen" update. Deliberately not observable: the screen's body never reads it.
@MainActor
private final class SeenRows {
    var visible: Set<Int64> = []
    var settle: Task<Void, Never>?
}

/// Where the entries you have not seen begin.
private struct NewMarker: View {
    var body: some View {
        HStack(spacing: 10) {
            Rectangle().fill(Palette.accent.opacity(0.4)).frame(height: 1)
            Text("NEW ABOVE").font(RFont.sans(11.5, .semibold)).tracking(0.8).foregroundStyle(Palette.accent).fixedSize()
            Rectangle().fill(Palette.accent.opacity(0.4)).frame(height: 1)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 6)
        .accessibilityIdentifier("newMarker")
    }
}

/// A request or connection that waits for you; its border shows how long the AI keeps waiting.
private struct PendingCard: View {
    var item: PendingItem
    var onOpen: () -> Void
    @Environment(\.feedback) private var feedback
    /// In a narrow column (an iPad's content column) the text gets the room and a chevron stands in for "Review".
    @State private var narrow = false

    var body: some View {
        // An upload waits until the server deletes it (an hour or so), not for an AI that is holding on: no countdown.
        let waitUntil = item.kind == .blob ? nil : item.waitUntil
        // A new connection or another phone of the account: no connection of its own yet.
        let pairing = item.kind == .pairing || item.kind == .join
        let joining = item.kind == .join
        CountdownFrame(createdAt: item.createdAt, waitUntil: waitUntil) {
            Button(action: onOpen) {
                HStack(spacing: 14) {
                    ActionTile(kind: joining ? .join : pairing ? .pair : (item.kind == .blob ? .upload : ActionKind.of(item.action)), count: Int(item.count), size: 46)
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 8) {
                            if !pairing { ConnectionIcon(connectionId: item.connectionId, label: item.connectionLabel, size: 20) }
                            Text(joining ? "\(untrusted(item.connectionLabel)) asks to join your account" : pairing ? "\(untrusted(item.connectionLabel)): wants to connect" : item.listTitle)
                                .font(RFont.sans(16, .semibold))
                                .foregroundStyle(Palette.text)
                                .lineLimit(2)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        if item.kind == .blob && !item.subtitle.trimmingCharacters(in: .whitespaces).isEmpty {
                            Text(untrusted(item.subtitle)).font(RFont.sans(13.5)).foregroundStyle(Palette.secondary).lineLimit(2)
                        }
                        if !pairing { ConnectorTags(service: item.service, account: item.account) }
                        if let line = item.suggestion {
                            Label(untrusted(line), systemImage: "sparkles")
                                .font(RFont.sans(12.5, .medium))
                                .foregroundStyle(Palette.accent)
                                .lineLimit(1)
                                .accessibilityIdentifier("pendingSuggestion:\(item.id)")
                        }
                        timeText(waitUntil)
                    }
                    if narrow {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 14, weight: .semibold))
                            .foregroundStyle(Palette.accent)
                            .accessibilityHidden(true)
                    } else {
                        Text("Review")
                            .font(RFont.sans(15, .semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 16)
                            .padding(.vertical, 9)
                            .background(Palette.accent, in: Capsule())
                            .accessibilityHidden(true)
                    }
                }
                .padding(14)
                .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            }
            .buttonStyle(CardPressStyle())
            .accessibilityElement(children: .combine)
            .accessibilityHint("Review")
        }
        .onGeometryChange(for: Bool.self) { $0.size.width < 370 } action: { narrow = $0 }
        .contextMenu {
            Button("Review", systemImage: "checkmark.shield", action: onOpen)
            Button("Copy", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = item.listTitle
                feedback.play(.copied)
            }
        }
        .accessibilityIdentifier("pending:\(item.id)")
    }

    /// The only part of the card that follows the clock: the countdown while the AI waits, else "5 min ago", which
    /// moves slowly.
    @ViewBuilder private func timeText(_ waitUntil: Int64?) -> some View {
        if waitUntil == nil {
            LiveClock(interval: 15) { _ in timeLabel(nil) }
        } else {
            UrgencyClock(createdAt: item.createdAt, waitUntil: waitUntil) { u in timeLabel(u) }
        }
    }

    private func timeLabel(_ u: Urgency?) -> some View {
        Text(timeLine(u))
            .font(RFont.sans(13, .medium))
            .foregroundStyle(timeColor(u))
            .monospacedDigit()
            .animation(.easeInOut(duration: 0.4), value: u?.urgent)
    }

    private func timeLine(_ u: Urgency?) -> String {
        guard let u else { return TimeText.relative(item.createdAt) }
        if u.stale { return "Stopped waiting" }
        return u.urgent ? "\(u.remainingText) left" : "\(u.remainingText) · waiting for you"
    }

    private func timeColor(_ u: Urgency?) -> Color {
        guard let u else { return Palette.tertiary }
        if u.stale { return Palette.warning }
        return u.urgent ? Palette.danger : Palette.secondary
    }
}

/// A card that dims a little while pressed.
private struct CardPressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.8 : 1)
            .scaleEffect(configuration.isPressed ? 0.985 : 1)
            .animation(.spring(duration: 0.2), value: configuration.isPressed)
    }
}

#if DEBUG
#Preview {
    PreviewHost { NavigationStack { ActivityScreen() } }
}
#endif
