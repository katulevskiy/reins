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

/// Activity (iOS-Activity.png): when something is broken, a short amber field at the very top; the title with the mode
/// chip and the filter; one mood line; the stack of what waits (the pond when nothing does); then the river, everything
/// your AIs did by day, newest first, in hairline rows. An answer folds into the river. On a regular width this is the
/// content column and an entry opens in the detail column.
struct ActivityScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @SceneStorage("activity.filter") private var filterRaw = ActivityFilter.all.rawValue
    /// Any AI, or one connection's entries only ("" for any).
    @SceneStorage("activity.ai") private var aiFilter = ""
    /// What Autopilot made of each waiting request (Manual and Auto), by request id.
    @State private var suggestions: [String: SuggestionView] = [:]
    /// The list was placed where the user stopped reading; until then nothing is marked seen.
    @State private var positioned = false
    /// Which rows are on screen; not observed, so scrolling does not draw the whole list again.
    @State private var seen = SeenRows()
    @State private var atTop = true
    @State private var away = false

    private var filter: ActivityFilter { ActivityFilter(rawValue: filterRaw) ?? .all }

    private var filterBinding: Binding<ActivityFilter> {
        Binding(get: { filter }, set: { f in
            withAnimation(Motion.of(.smooth(duration: 0.2), reduce: reduceMotion)) { filterRaw = f.rawValue }
        })
    }

    private var aiBinding: Binding<String> {
        Binding(get: { aiFilter }, set: { id in
            withAnimation(Motion.of(.smooth(duration: 0.2), reduce: reduceMotion)) { aiFilter = id }
        })
    }

    private var shown: [ActivityEntry] {
        ActivityText.filtered(model.activity, filter: filter, connection: aiFilter)
    }

    var body: some View {
        let entries = shown
        ScrollViewReader { proxy in
            ScrollView {
                // The warnings are often empty: outside the lazy stack, whose first rows must have a size.
                VStack(alignment: .leading, spacing: 0) {
                    warnings
                    LazyVStack(alignment: .leading, spacing: 0) {
                        header.id("top")
                        mood
                        if let notice = model.syncNotice { SyncNoticeRow(text: notice) }
                        waiting
                        if filter != .all || !aiFilter.isEmpty { filters }
                        if entries.isEmpty {
                            empty
                        } else {
                            list(entries)
                        }
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
                        HStack(spacing: 6) {
                            GlyphIcon(.chevronUp, size: 14)
                            Text("Latest")
                        }
                        .font(RFont.sans(14, .semibold))
                        .foregroundStyle(Palette.ink)
                        .padding(.horizontal, 16)
                        .frame(height: 40)
                        .background(Palette.paperRaised, in: Capsule())
                        .overlay(Capsule().strokeBorder(Palette.rule, lineWidth: 1))
                    }
                    .buttonStyle(PressDimsStyle())
                    .padding(.bottom, 16)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                    .accessibilityIdentifier("scrollToLatest")
                }
            }
            .task(id: model.activity.isEmpty) { await position(proxy) }
        }
        .task(id: model.pending.filter { $0.kind == .request }.map(\.id)) { await loadSuggestions() }
        .task(id: model.fold.request?.at) { await model.fold.run(model) }
        .onChange(of: atTop) { markSeenSoon() }
        .pageBackground()
        .foldOverlay(model.fold)
        .signalMenuHost()
        .rootNavigationBar()
        .navigationTitle("Activity")
    }

    // MARK: Parts

    /// The title with the mode chip and the filter beside it, or under it when the column is too narrow for both (an
    /// iPad's content column).
    private var header: some View {
        ViewThatFits(in: .horizontal) {
            PageHeader(title: "Activity") { controls }
            VStack(alignment: .leading, spacing: 10) {
                PageHeader(title: "Activity") { EmptyView() }
                controls
            }
        }
        .padding(.horizontal, 24)
        .padding(.top, 12)
    }

    private var controls: some View {
        HStack(spacing: 8) {
            if let autopilot = model.autopilot {
                ActivityModeMenu(settings: autopilot)
            }
            ActivityFilterMenu(filter: filterBinding, ai: aiBinding, ais: ActivityText.ais(model.activity))
        }
        .fixedSize()
    }

    /// "Two things wait for you.", "All quiet. Three AIs are connected."
    private var mood: some View {
        Text(ActivityText.mood(waiting: model.pending.count, connections: model.connections.count))
            .textRole(.mood)
            .foregroundStyle(Palette.ink2)
            .padding(.horizontal, 24)
            .padding(.top, 2)
            .accessibilityIdentifier("activityMood")
    }

    /// What is broken, as the warn field at the top of the screen.
    @ViewBuilder private var warnings: some View {
        if model.deviceReplaced {
            CompactBanner(text: "Another phone approves", symbol: "iphone.slash", button: "Settings", label: "Reins · This phone") {
                model.select(.settings)
            }
            .accessibilityIdentifier("replacedBanner")
        }
        if model.registrationError != nil {
            CompactBanner(
                text: "Not your approval phone yet", symbol: "exclamationmark.triangle.fill", tint: Palette.danger, button: "Retry",
                label: "Reins · Approvals"
            ) {
                Task {
                    do {
                        try await model.registerDevice(force: false)
                    } catch {
                        model.registrationError = error.userMessage
                        model.notice = error.userMessage
                    }
                }
            }
            .accessibilityIdentifier("registrationBanner")
        }
        NotificationsOffCard()
        ScreenLockBanner(compact: true)
    }

    @ViewBuilder private var waiting: some View {
        if !model.pending.isEmpty {
            WaitStack(groups: WaitGroup.of(model.pending), suggestions: suggestions)
                .padding(.top, 16)
                .animation(Motion.of(Motion.cardSpring(0.32), reduce: reduceMotion), value: model.pending.map(\.id))
        } else if filter == .all && !model.connections.isEmpty {
            // All quiet: the pond fills the empty stack.
            Illustration(.pond)
                .frame(maxWidth: 300)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 24)
                .padding(.top, 12)
                .accessibilityIdentifier("pond")
        }
    }

    /// The chosen filter, as chips to change or clear it.
    private var filters: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(ActivityFilter.allCases, id: \.self) { f in
                    FilterChip(title: f.title, selected: filter == f) { filterBinding.wrappedValue = f }
                        .accessibilityIdentifier("filter:\(f.rawValue)")
                }
                AnyAIChip(ai: aiBinding, ais: ActivityText.ais(model.activity))
            }
            .padding(.horizontal, 24)
        }
        .padding(.top, 16)
        .transition(.opacity)
    }

    @ViewBuilder private var empty: some View {
        if filter != .all || !aiFilter.isEmpty {
            EmptyState(symbol: "line.3.horizontal.decrease", title: "Nothing here")
                .accessibilityIdentifier("noAutomatic")
        } else if model.connections.isEmpty {
            // A new account: nothing can ask yet, so offer the ways to connect something.
            VStack(spacing: 10) {
                EmptyState.calm("Nothing has asked yet.")
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
        } else if model.pending.isEmpty {
            // The pond already says it.
            Color.clear.frame(height: 1).accessibilityIdentifier("noActivity")
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
        ForEach(Array(EntryText.days(entries).enumerated()), id: \.element.day) { _, group in
            PageSectionHeader(EntryText.day(group.day, now: today)) {
                Text(ActivityText.dayCount(group.entries)).textRole(.meta).foregroundStyle(Palette.inkMeta)
            }
            .padding(.top, 10)
            ForEach(Array(group.entries.enumerated()), id: \.element.id) { i, entry in
                let last = i == group.entries.count - 1
                row(entry, selected: selected == entry.id, now: now)
                // Where the entries you have not seen end: everything above this line is new.
                if entry.id == oldestUnseen && !last {
                    NewMarker()
                } else if !last {
                    Rectangle().fill(Palette.rule).frame(height: 1).padding(.leading, 76).padding(.trailing, 24)
                }
            }
        }
    }

    private func row(_ entry: ActivityEntry, selected: Bool, now: Int64) -> some View {
        Button {
            model.show(.activityDetail(entry.id), in: .activity)
            feedback.defaultTap()
        } label: {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                HistoryRow(entry: entry, selected: selected, glowing: model.fold.glowing(entry.id, now: context.date), now: now)
            }
        }
        .buttonStyle(RowButtonStyle())
        .correctionSwipe(entry)
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
            Rectangle().fill(Palette.violet.opacity(0.4)).frame(height: 1)
            Text("New above").textRole(.label).foregroundStyle(Palette.violet).fixedSize()
            Rectangle().fill(Palette.violet.opacity(0.4)).frame(height: 1)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 6)
        .accessibilityIdentifier("newMarker")
    }
}

#if DEBUG
#Preview {
    PreviewHost { NavigationStack { ActivityScreen() } }
}
#endif

/// What the core says about syncing with the account's other phone (#101): a soft row on paper under a hairline, the
/// sync glyph, the core's words and OK, which dismisses it in the core. Sync never keeps anyone out; this row is all it
/// shows.
struct SyncNoticeRow: View {
    var text: String
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 12) {
            GlyphIcon(.refresh, size: 18).foregroundStyle(Palette.violet)
            Text(text)
                .font(RFont.sans(14))
                .foregroundStyle(Palette.ink)
                .lineLimit(3)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("OK") { Task { await model.dismissSyncNotice() } }
                .buttonStyle(SignalButtonStyle(.soft, size: .small))
        }
        .padding(.top, 14)
        .padding(.bottom, 12)
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
        .padding(.horizontal, 24)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("syncNoticeBanner")
    }
}
