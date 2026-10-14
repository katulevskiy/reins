import SwiftUI

/// History filters: everything, what Autopilot (or a bypass, or Lockdown) decided, the denials, what a grant covered.
enum ActivityFilter: String, CaseIterable {
    case all, autopilot, denied, grants

    var title: String {
        switch self {
        case .all: "All"
        case .autopilot: "Autopilot"
        case .denied: "Denied"
        case .grants: "Grants"
        }
    }

    func keeps(_ entry: ActivityEntry) -> Bool {
        switch self {
        case .all: true
        case .autopilot: !entry.decidedBy.isEmpty
        case .denied: entry.outcome == "denied"
        case .grants: entry.grantId != nil
        }
    }
}

/// The first section: what waits for you, then everything your AIs did, by day, newest first. On a regular width this
/// is the content column and an entry opens in the detail column.
struct ActivityScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @SceneStorage("activity.filter") private var filterRaw = ActivityFilter.all.rawValue
    @State private var filtersShown = false
    /// What Autopilot made of each waiting request (Manual and Auto), by request id.
    @State private var suggestions: [String: SuggestionView] = [:]
    /// The list was placed where the user stopped reading; until then nothing is marked seen.
    @State private var positioned = false
    /// Which rows are on screen; not observed, so scrolling does not draw the whole list again.
    @State private var seen = SeenRows()
    @State private var atTop = true
    @State private var away = false
    /// The mode pill's quick menu is open.
    @State private var quickOpen = false

    private var filter: ActivityFilter { ActivityFilter(rawValue: filterRaw) ?? .all }

    private var shown: [ActivityEntry] {
        let f = filter
        return f == .all ? model.activity : model.activity.filter(f.keeps)
    }

    var body: some View {
        let entries = shown
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    header.id("top")
                    banners
                    waiting
                    if filtersShown || filter != .all { filters }
                    if entries.isEmpty {
                        empty
                    } else {
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
        .task(id: model.pending.filter { $0.kind == .request }.map(\.id)) { await loadSuggestions() }
        .onChange(of: atTop) { markSeenSoon() }
        .pageBackground()
        .autopilotQuickMenu(isPresented: $quickOpen)
        .rootNavigationBar()
        .navigationTitle("Activity")
    }

    // MARK: Parts

    /// The title with the + and the mode pill beside it, or under it when the column is too narrow for both (an iPad's
    /// content column).
    private var header: some View {
        ViewThatFits(in: .horizontal) {
            PageHeader(title: "Activity") { controls }
            VStack(alignment: .leading, spacing: 10) {
                PageHeader(title: "Activity") { EmptyView() }
                controls
            }
        }
        .padding(.leading, 20)
        .padding(.trailing, 16)
        .padding(.top, 12)
        .padding(.bottom, 4)
    }

    private var controls: some View {
        HStack(spacing: 8) {
            // The page's Open cue is the sound (the button plays the default tap).
            RoundPlusButton(label: "Integrations") { model.show(.integrations) }
                .accessibilityIdentifier("integrations")
            if let autopilot = model.autopilot {
                AutopilotQuickPill(settings: autopilot, menuOpen: $quickOpen)
            }
        }
        .fixedSize()
    }

    @ViewBuilder private var banners: some View {
        if model.deviceReplaced {
            CompactBanner(text: "Another phone approves", symbol: "iphone.slash", button: "Settings") {
                model.select(.settings)
            }
            .padding(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
            .accessibilityIdentifier("replacedBanner")
        }
        if model.registrationError != nil {
            CompactBanner(text: "Not your approval phone yet", symbol: "exclamationmark.triangle.fill", tint: Palette.danger, button: "Retry") {
                Task {
                    do {
                        try await model.registerDevice(force: false)
                    } catch {
                        model.registrationError = error.userMessage
                        model.notice = error.userMessage
                    }
                }
            }
            .padding(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
            .accessibilityIdentifier("registrationBanner")
        }
        NotificationsOffCard()
        ScreenLockBanner(padding: EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16), compact: true)
    }

    @ViewBuilder private var waiting: some View {
        if !model.pending.isEmpty {
            PageSectionHeader("Waiting · \(model.pending.count)")
            ForEach(WaitGroup.of(model.pending)) { group in
                WaitCard(
                    group: group,
                    advice: WaitAdvice.of(group.items, suggestions),
                    danger: group.items.contains { suggestions[$0.id]?.floor == true }
                )
                .padding(.horizontal, 12)
                .padding(.vertical, 5)
                .id("p:\(group.id)")
            }
        }
    }

    /// The shared choice chips, one chosen; Autopilot's carries its bolt. They scroll sideways when the chosen one's
    /// check leaves no room.
    private var filters: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(ActivityFilter.allCases, id: \.self) { f in
                    let on = filter == f
                    let pick = {
                        guard !on else { return }
                        withAnimation(.smooth(duration: 0.2)) { filterRaw = f.rawValue }
                    }
                    Group {
                        if f == .autopilot {
                            SelectChip(title: f.title, selected: on, leading: {
                                Image(systemName: "bolt.fill")
                                    .font(.system(size: 12, weight: .semibold))
                                    .foregroundStyle(on ? Palette.accent : Palette.secondary)
                            }, action: pick)
                        } else {
                            SelectChip(title: f.title, selected: on, action: pick)
                        }
                    }
                    .accessibilityIdentifier("filter:\(f.rawValue)")
                }
            }
            .padding(.horizontal, 16)
        }
        .padding(.top, 14)
        .transition(.opacity.combined(with: .move(edge: .top)))
    }

    private var filterButton: some View {
        Button {
            // Closing the chips also shows everything again.
            let open = !(filtersShown || filter != .all)
            withAnimation(.smooth(duration: 0.25)) {
                filtersShown = open
                if !open { filterRaw = ActivityFilter.all.rawValue }
            }
            feedback.play(.expand(open))
        } label: {
            Image(systemName: filter == .all ? "line.3.horizontal.decrease" : "line.3.horizontal.decrease.circle.fill")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(Palette.accent)
                .frame(width: 32, height: 28)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Filter")
        .accessibilityIdentifier("filterToggle")
    }

    @ViewBuilder private var empty: some View {
        if filter != .all {
            EmptyState(symbol: "line.3.horizontal.decrease", title: "Nothing here")
                .accessibilityIdentifier("noAutomatic")
        } else if model.connections.isEmpty {
            // A new account: nothing can ask yet, so offer the ways to connect something.
            VStack(spacing: 10) {
                EmptyState(symbol: "waveform.path.ecg", title: "Nothing yet")
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
            EmptyState(symbol: "waveform.path.ecg", title: "Nothing yet")
                .accessibilityIdentifier("noActivity")
        }
    }

    @ViewBuilder private func list(_ entries: [ActivityEntry]) -> some View {
        let seenId = model.seenActivityId
        let oldestUnseen = entries.lastIndex { $0.id > seenId }.map { entries[$0].id }
        let selected: Int64? = if case let .activityDetail(id)? = model.path(.activity).first { id } else { nil }
        let now = Timers.nowSeconds()
        let today = Date(timeIntervalSince1970: TimeInterval(now))
        ForEach(Array(EntryText.days(entries).enumerated()), id: \.element.day) { index, group in
            if index == 0 {
                PageSectionHeader(EntryText.day(group.day, now: today)) { filterButton }
            } else {
                PageSectionHeader(EntryText.day(group.day, now: today))
            }
            ForEach(Array(group.entries.enumerated()), id: \.element.id) { i, entry in
                let first = i == 0
                let last = i == group.entries.count - 1
                row(entry, selected: selected == entry.id, first: first, last: last, now: now)
                // Where the entries you have not seen end: everything above this line is new.
                if entry.id == oldestUnseen && !last {
                    NewMarker().background(Palette.elevated).padding(.horizontal, 16)
                } else if !last {
                    Hairline(inset: HistoryRow.hairlineInset(entry)).background(Palette.elevated).padding(.horizontal, 16)
                }
            }
        }
    }

    private func row(_ entry: ActivityEntry, selected: Bool, first: Bool, last: Bool, now: Int64) -> some View {
        let shape = UnevenRoundedRectangle(
            topLeadingRadius: first ? 20 : 0, bottomLeadingRadius: last ? 20 : 0,
            bottomTrailingRadius: last ? 20 : 0, topTrailingRadius: first ? 20 : 0, style: .continuous
        )
        return Button {
            model.show(.activityDetail(entry.id), in: .activity)
            feedback.defaultTap()
        } label: {
            HistoryRow(entry: entry, selected: selected, now: now)
        }
        .buttonStyle(RowButtonStyle())
        .background(Palette.elevated)
        .clipShape(shape)
        .contentShape(.contextMenuPreview, shape)
        .contextMenu {
            Button("Open", systemImage: "arrow.up.right") {
                model.show(.activityDetail(entry.id), in: .activity)
                feedback.defaultTap()
            }
            Button("Copy", systemImage: "doc.on.doc") {
                UIPasteboard.general.string = [entry.label.map(RequestLabelLook.plain) ?? entry.headline, untrusted(entry.detail)]
                    .filter { !$0.isEmpty }
                    .joined(separator: "\n")
                feedback.play(.copied)
            }
        }
        .padding(.horizontal, 16)
        .onScrollVisibilityChange(threshold: 0.6) { isVisible in
            if isVisible { seen.visible.insert(entry.id) } else { seen.visible.remove(entry.id) }
            markSeenSoon()
        }
        .id("e:\(entry.id)")
        .accessibilityIdentifier("entry:\(entry.id)")
    }

    // MARK: Autopilot

    /// Reads what Autopilot made of each waiting request. It judges as requests arrive, so the ones it has not judged
    /// yet are asked again a few times.
    private func loadSuggestions() async {
        let core = model.core
        var tries = 0
        while !Task.isCancelled {
            let ids = model.pending.filter { $0.kind == .request }.map(\.id)
            var next: [String: SuggestionView] = [:]
            for id in ids {
                if let s = try? await core.autopilotSuggestion(requestId: id) { next[id] = s }
            }
            if Task.isCancelled { return }
            if next != suggestions { suggestions = next }
            tries += 1
            guard next.count < ids.count, tries < 6, model.autopilot != nil, Timers.live else { return }
            try? await Task.sleep(for: .seconds(2))
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

#if DEBUG
#Preview {
    PreviewHost { NavigationStack { ActivityScreen() } }
}
#endif
