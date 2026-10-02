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
    @State private var visible: Set<Int64> = []
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
        .onChange(of: visible) { markSeen() }
        .onChange(of: atTop) { markSeen() }
        .pageBackground()
        .toolbar(.hidden, for: .navigationBar)
        .navigationTitle("Activity")
    }

    // MARK: Parts

    private var header: some View {
        PageHeader(title: "Activity") {
            HStack(spacing: 8) {
                if let autopilot = model.autopilot {
                    ActivityModePill(settings: autopilot) {
                        feedback.play(.tap)
                        model.show(.autopilot)
                    }
                }
                GlassPill(symbol: "square.grid.2x2", text: "Integrations") {
                    feedback.play(.tap)
                    model.show(.integrations)
                }
                .accessibilityIdentifier("integrations")
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 12)
        .padding(.bottom, 4)
    }

    @ViewBuilder private var banners: some View {
        if model.deviceReplaced {
            Banner("Another phone is your approval device now. Use this phone again from Settings.", kind: .warning)
                .padding(16)
                .accessibilityIdentifier("replacedBanner")
        }
        if let error = model.registrationError {
            Banner("This phone is not registered as your approval device yet: \(error)", kind: .error)
                .padding(16)
                .accessibilityIdentifier("registrationBanner")
        }
    }

    @ViewBuilder private var waiting: some View {
        if !model.pending.isEmpty {
            SectionHeader("Waiting for you")
                .padding(.leading, 16)
                .padding(.top, 18)
                .padding(.bottom, 6)
            ForEach(model.pending, id: \.id) { item in
                PendingCard(item: item) {
                    feedback.play(.tap)
                    model.openSheet(item.sheetTarget)
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
            EmptyState(symbol: "sparkles", title: "Nothing automatic yet", message: "What Autopilot, a bypass or Lockdown decides for you shows up here.")
                .accessibilityIdentifier("noAutomatic")
        } else {
            EmptyState(symbol: "list.bullet", title: "Nothing yet", message: "When an AI searches, reads or sends on your behalf, it shows up here.")
                .accessibilityIdentifier("noActivity")
        }
    }

    @ViewBuilder private func list(_ entries: [ActivityEntry]) -> some View {
        let seen = model.seenActivityId
        let oldestUnseen = entries.lastIndex { $0.id > seen }
        let selected: Int64? = if case let .activityDetail(id)? = model.path(.activity).first { id } else { nil }
        ForEach(Array(entries.enumerated()), id: \.element.id) { index, entry in
            Button {
                feedback.play(.tap)
                model.show(.activityDetail(entry.id), in: .activity)
            } label: {
                ActivityRow(entry: entry, selected: selected == entry.id)
            }
            .buttonStyle(RowButtonStyle())
            .contextMenu {
                Button("Open", systemImage: "arrow.up.right") { model.show(.activityDetail(entry.id), in: .activity) }
                Button("Copy", systemImage: "doc.on.doc") {
                    UIPasteboard.general.string = [entry.headline, untrusted(entry.detail)].filter { !$0.isEmpty }.joined(separator: "\n")
                    feedback.play(.copied)
                }
            }
            .onScrollVisibilityChange(threshold: 0.6) { isVisible in
                if isVisible { visible.insert(entry.id) } else { visible.remove(entry.id) }
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
        guard positioned else { return }
        if let newest = visible.max() {
            model.markActivitySeen(newest)
        } else if atTop, let first = shown.first {
            model.markActivitySeen(first.id)
        }
    }
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

    var body: some View {
        // An upload waits until the server deletes it (an hour or so), not for an AI that is holding on: no countdown.
        let waitUntil = item.kind == .blob ? nil : item.waitUntil
        let pairing = item.kind == .pairing
        CountdownFrame(createdAt: item.createdAt, waitUntil: waitUntil) { u in
            Button(action: onOpen) {
                HStack(spacing: 14) {
                    ActionTile(kind: pairing ? .pair : (item.kind == .blob ? .upload : ActionKind.of(item.action)), count: Int(item.count), size: 46)
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 8) {
                            if !pairing { ConnectionIcon(connectionId: item.connectionId, label: item.connectionLabel, size: 20) }
                            Text(pairing ? "\(untrusted(item.connectionLabel)): wants to connect" : item.headline)
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
                        Text(timeLine(u))
                            .font(RFont.sans(13, .medium))
                            .foregroundStyle(timeColor(u))
                            .monospacedDigit()
                    }
                    Text("Review")
                        .font(RFont.sans(15, .semibold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 9)
                        .background(Palette.accent, in: Capsule())
                        .accessibilityHidden(true)
                }
                .padding(14)
                .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            }
            .buttonStyle(CardPressStyle())
            .accessibilityElement(children: .combine)
            .accessibilityHint("Review")
        }
        .contextMenu {
            Button("Review", systemImage: "checkmark.shield", action: onOpen)
            Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = item.headline }
        }
        .accessibilityIdentifier("pending:\(item.id)")
    }

    private func timeLine(_ u: Urgency?) -> String {
        guard let u else { return TimeText.relative(item.createdAt) }
        if u.stale { return "Stopped waiting · you can still approve" }
        return u.urgent ? "\(u.remainingSeconds) s left" : "\(u.remainingSeconds) s · waiting for you"
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

#Preview {
    PreviewHost { NavigationStack { ActivityScreen() } }
}
