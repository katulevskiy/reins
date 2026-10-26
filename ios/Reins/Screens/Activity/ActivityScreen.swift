import SwiftUI

/// Activity (Next-Activity, Next-Wallpaper): when something is broken, a short amber field at the very top; the title
/// with the mode chip (it opens the Mode sheet), search and the filter button; the filter chips when shown or on.
/// Browsing, once something is connected, over the wallpaper: the stack of what waits (or the action slot that proposes
/// the missing half, or the open art), then the river by day, newest first, in raised groups (frosted panes on the
/// art, the day's header inside its group): today open, earlier days folded until opened. An answer folds into the
/// river. Searching (the search bar, or a filter on): the matches by day, their words set apart, on plain paper. A new
/// account: the wallpaper and the two ways to connect something. On a regular width this is the content column and an
/// entry opens in the detail column.
struct ActivityScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// What Autopilot made of each waiting request (Manual and Auto), by request id.
    @State private var suggestions: [String: SuggestionView] = [:]
    /// The list was placed where the user stopped reading; until then nothing is marked seen.
    @State private var positioned = false
    /// Which rows are on screen; not observed, so scrolling does not draw the whole list again.
    @State private var seen = SeenRows()
    @State private var atTop = true
    @State private var away = false
    /// The filter chips, shown by the filter button (they also show whenever a filter is on).
    @State private var filtersShown = false
    /// The header is the search bar.
    @State private var searchOpen = false
    /// How tall the warn fields above the header are (0 with none).
    @State private var warningsHeight: CGFloat = 0
    /// The river's days, from the core's stamps of every entry; nil until read (the days of what is loaded until then).
    @State private var days: [ActivityDay]?
    /// The entries read for each opened day, by day.
    @State private var pages: [String: DayPage] = [:]
    /// The integrations, kinds and AIs the history holds, for the filter menus.
    @State private var facets: ActivityFacets?
    /// The search's matches; nil while browsing or before the first answer.
    @State private var results: SearchResults?

    /// A day's entries read so far and where the next page starts.
    private struct DayPage {
        var entries: [ActivityEntry] = []
        var next: ActivityCursor?
        var started = false
        var loading = false
        var done: Bool { started && next == nil }
    }

    /// What is searched: the words (trimmed) and the filters.
    private struct SearchKey: Equatable {
        var text: String
        var filters: ActivityFilters
    }

    /// A screen task's key with whether the app is in the background: leaving the app cancels the task (the store
    /// work it started finishes under its own background task, and no new work starts), and coming back runs it again.
    private struct InFront<Key: Equatable>: Equatable {
        var key: Key
        var background: Bool
    }

    private struct SearchResults {
        var key: SearchKey
        /// The query of the first page (its `since` fixed then), for the pages after it.
        var query: ActivityQuery
        var hits: [ActivityHit]
        var next: ActivityCursor?
        var total: UInt32
        var terms: [String]
        var loadingMore = false
    }

    private var searchKey: SearchKey {
        SearchKey(text: model.activitySearchText.trimmingCharacters(in: .whitespacesAndNewlines), filters: model.activityFilters)
    }

    private var searching: Bool { ActivityFilters.searching(text: model.activitySearchText, filters: model.activityFilters) }

    private var textBinding: Binding<String> {
        Binding(get: { model.activitySearchText }, set: { model.activitySearchText = $0 })
    }

    private var filtersBinding: Binding<ActivityFilters> {
        Binding(get: { model.activityFilters }, set: { model.activityFilters = $0 })
    }

    private var filtersShownBinding: Binding<Bool> {
        Binding(get: { filtersShown }, set: { shown in
            withAnimation(Motion.of(.smooth(duration: 0.2), reduce: reduceMotion)) { filtersShown = shown }
        })
    }

    var body: some View {
        let searching = self.searching
        let wallpaper = wallpaperShown
        ScrollViewReader { proxy in
            ScrollView {
                // The warnings are often empty: outside the lazy stack, whose first rows must have a size.
                VStack(alignment: .leading, spacing: 0) {
                    VStack(spacing: 0) { warnings }
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { warningsHeight = $0 }
                    LazyVStack(alignment: .leading, spacing: 0) {
                        header.id("top")
                        if !searchOpen && (filtersShown || !model.activityFilters.isEmpty) {
                            ActivityFilterBar(filters: filtersBinding, facets: facets)
                                .padding(.top, 14)
                                .transition(.opacity)
                        }
                        if let notice = model.syncNotice { SyncNoticeRow(text: notice) }
                        if searching {
                            found
                        } else {
                            browse(wallpaper: wallpaper)
                        }
                    }
                }
                .padding(.bottom, 32)
            }
            .scrollDismissesKeyboard(.immediately)
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
        .environment(\.onWallpaper, wallpaper)
        .task(id: model.pending.filter { $0.kind == .request }.map(\.id)) { await loadSuggestions() }
        .task(id: model.fold.request?.at) { await model.fold.run(model) }
        // New entries: the days' counts and the open days again.
        .task(id: InFront(key: model.activity.map(\.id), background: model.inBackground)) { await loadRiver() }
        .task(id: InFront(key: searchKey, background: model.inBackground)) { await search(searchKey) }
        .onChange(of: atTop) { markSeenSoon() }
        .animation(Motion.of(.smooth(duration: 0.2), reduce: reduceMotion), value: searching)
        .animation(Motion.of(.smooth(duration: 0.2), reduce: reduceMotion), value: searchOpen)
        .background {
            ZStack {
                Palette.paper.ignoresSafeArea()
                if wallpaper { WallpaperLayer().transition(.opacity) }
            }
        }
        .statusBarBackdrop(shown: !wallpaper || !atTop)
        .foldOverlay(model.fold)
        .signalMenuHost()
        .rootNavigationBar()
        .navigationTitle("Activity")
    }

    // MARK: Parts

    /// The art behind the page while browsing (Next-Wallpaper): searching and filtering are on plain paper.
    private var wallpaperShown: Bool {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-noWallpaper") { return false }
        #endif
        return !searchOpen && model.activityFilters.isEmpty
    }

    /// Something broken shows its amber field above the header.
    private var bannerShown: Bool { warningsHeight > 1 }

    /// The title with the mode chip, search and the filter button beside it, or under it when the column is too narrow
    /// for them (an iPad's content column); the search bar while searching.
    @ViewBuilder private var header: some View {
        Group {
            if searchOpen {
                ActivitySearchBar(text: textBinding) { closeSearch() }
            } else {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 12) {
                        title
                        controls
                    }
                    VStack(alignment: .leading, spacing: 10) {
                        title
                        controls
                    }
                }
            }
        }
        .padding(.horizontal, 20)
        .frame(minHeight: 44)
        .padding(.top, bannerShown ? 14 : 0)
    }

    /// "Activity" in the title role; at accessibility sizes it grows less, so the chip and search stay beside it.
    private var title: some View {
        Text("Activity")
            .textRole(.title)
            .foregroundStyle(Palette.ink)
            .lineLimit(1)
            .minimumScaleFactor(0.8)
            .dynamicTypeSize(...DynamicTypeSize.xLarge)
            .accessibilityAddTraits(.isHeader)
            .accessibilityIdentifier("activityMood")
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var controls: some View {
        HStack(spacing: 8) {
            if let autopilot = model.autopilot {
                ModeChipButton(settings: autopilot)
                    .dynamicTypeSize(...DynamicTypeSize.xxLarge)
                    .padding(.trailing, 4)
            }
            RoundHeaderButton(glyph: .search, label: "Search") { openSearch() }
                .accessibilityIdentifier("activitySearch")
            ActivityFilterToggle(shown: filtersShownBinding, active: !model.activityFilters.isEmpty)
        }
        .fixedSize()
    }

    private func openSearch() {
        withAnimation(Motion.of(.smooth(duration: 0.2), reduce: reduceMotion)) {
            searchOpen = true
            filtersShown = false
        }
        feedback.play(.tap)
    }

    private func closeSearch() {
        model.activitySearchText = ""
        withAnimation(Motion.of(.smooth(duration: 0.2), reduce: reduceMotion)) { searchOpen = false }
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
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

    /// Browsing: what waits (or what to connect, or the open art), then the river.
    @ViewBuilder private func browse(wallpaper: Bool) -> some View {
        if model.connections.isEmpty && model.pending.isEmpty && shownDays.isEmpty {
            // A new account: nothing can ask yet, so offer the ways to connect something, low over the art.
            NewAccountActivity(connectComputer: { model.openSheet(.connectComputer) }, connectAi: copyAddress)
                .padding(.top, wallpaper ? 337 : 120)
        } else {
            if !model.pending.isEmpty {
                WaitStack(groups: WaitGroup.of(model.pending), suggestions: suggestions)
                    .padding(.top, wallpaper ? 70 : 14)
                    .padding(.bottom, wallpaper ? 10 : 0)
                    .animation(Motion.of(Motion.cardSpring(0.32), reduce: reduceMotion), value: model.pending.map(\.id))
            } else if let missing = missingHalf {
                ActionSlotCard(missing: missing, connect: copyAddress, scan: { model.openSheet(.connectComputer) }, typeCode: { model.openSheet(.connectComputer) })
                    .padding(.top, 70)
                    .padding(.bottom, 18)
            } else if wallpaper {
                // All quiet: the art stays open under the title.
                Color.clear.frame(height: 168).accessibilityIdentifier("pond")
            } else {
                Illustration(.pond)
                    .frame(width: 280, height: 148)
                    .frame(maxWidth: .infinity)
                    .padding(.top, 6)
                    .accessibilityIdentifier("pond")
            }
            river(wallpaper: wallpaper)
        }
    }

    /// The half of a setup that is missing: only computers are connected, or only AI apps.
    private var missingHalf: ActionSlotCard.Missing? {
        let computers = model.connections.filter { $0.keyFingerprint != nil }.count
        let apps = model.connections.count - computers
        if computers > 0 && apps == 0 { return .aiApp }
        if apps > 0 && computers == 0 { return .computer }
        return nil
    }

    private func copyAddress() {
        let address = model.mcpAddress
        UIPasteboard.general.string = address
        model.notice = "Copied \(address). In Claude.ai or ChatGPT: Settings, Connectors, add a custom connector and paste it."
        feedback.play(.copied)
    }

    // MARK: The river by day

    /// The days to show: the core's, with any day of what is loaded the stamps do not have yet.
    private var shownDays: [ActivityDay] {
        ActivityDays.merged(days, recent: model.activity, calendar: .current)
    }

    private func todayKey(_ now: Int64) -> String {
        ActivityDays.key(Date(timeIntervalSince1970: TimeInterval(now)), calendar: .current)
    }

    /// The open days' rows, by day.
    private func openRows(_ all: [ActivityDay], today: String) -> [String: [ActivityEntry]] {
        var out: [String: [ActivityEntry]] = [:]
        for day in all where model.activityDayOpen(day.key, today: today) {
            out[day.key] = ActivityDays.rows(day, pages: pages[day.key]?.entries ?? [], recent: model.activity)
        }
        return out
    }

    /// The rows on screen while browsing, newest first.
    private func visibleRows() -> [ActivityEntry] {
        let all = shownDays
        let open = openRows(all, today: todayKey(Timers.nowSeconds()))
        return all.flatMap { open[$0.key] ?? [] }
    }

    /// The days, each a group: on paper the header over its rows, on the art the header as the group's first row.
    @ViewBuilder private func river(wallpaper: Bool) -> some View {
        let now = Timers.nowSeconds()
        let today = Date(timeIntervalSince1970: TimeInterval(now))
        let todayKey = todayKey(now)
        let all = shownDays
        if all.isEmpty {
            if model.pending.isEmpty && missingHalf == nil {
                Color.clear.frame(height: 1).accessibilityIdentifier("noActivity")
            }
        } else {
            let open = openRows(all, today: todayKey)
            let oldestUnseen = all.flatMap { open[$0.key] ?? [] }.last { $0.id > model.seenActivityId }?.id
            let selected: Int64? = if case let .activityDetail(id)? = model.path(.activity).first { id } else { nil }
            ForEach(Array(all.enumerated()), id: \.element.id) { index, day in
                let rows = open[day.key] ?? []
                let isOpen = open[day.key] != nil
                let header = DayHeader(title: EntryText.day(day.start, now: today), day: day, open: isOpen, inGroup: wallpaper) {
                    toggle(day, today: todayKey)
                }
                if wallpaper {
                    header
                        .groupSlice(first: true, last: rows.isEmpty, hairline: false)
                        .padding(.top, index == 0 ? 0 : 10)
                } else {
                    header
                }
                ForEach(Array(rows.enumerated()), id: \.element.id) { i, entry in
                    let last = i == rows.count - 1
                    row(entry, selected: selected == entry.id, now: now)
                        // Where the entries you have not seen end: everything above the violet line is new.
                        .overlay(alignment: .top) {
                            if i > 0 && rows[i - 1].id == oldestUnseen { NewMarker() }
                        }
                        .groupSlice(first: i == 0 && !wallpaper, last: last, hairline: i > 0 && rows[i - 1].id != oldestUnseen)
                        .onAppear {
                            if last { Task { await loadDay(day, reset: false, upTo: rows.count + Self.pageSize) } }
                        }
                }
                if pages[day.key]?.loading == true && rows.count < day.count {
                    ProgressView().frame(maxWidth: .infinity).padding(.vertical, 14)
                }
            }
        }
    }

    private func toggle(_ day: ActivityDay, today: String) {
        let open = !model.activityDayOpen(day.key, today: today)
        withAnimation(Motion.of(.smooth(duration: 0.28), reduce: reduceMotion)) { model.setActivityDay(day.key, open: open) }
        feedback.play(.expand(open))
        if open && pages[day.key] == nil {
            let shown = ActivityDays.rows(day, pages: [], recent: model.activity).count
            Task { await loadDay(day, reset: true, upTo: max(Self.pageSize, shown)) }
        }
    }

    static let pageSize = 50

    /// The days and their counts from the core, the filter menus' choices, then the open days' entries again.
    private func loadRiver() async {
        guard model.screensMayReadStore else { return }
        let core = model.core
        do {
            async let stampsRead = core.activityStamps(query: ActivityQuery())
            async let facetsRead = core.activityFacets()
            let stamps = try await stampsRead
            let read = try await facetsRead
            if Task.isCancelled { return }
            days = ActivityDays.group(stamps, calendar: .current)
            facets = read
        } catch {
            // The days of what is loaded stay; the next change tries again.
            if Task.isCancelled { return }
        }
        let today = todayKey(Timers.nowSeconds())
        for day in shownDays where model.activityDayOpen(day.key, today: today) {
            let have = max(pages[day.key]?.entries.count ?? 0, ActivityDays.rows(day, pages: [], recent: model.activity).count)
            await loadDay(day, reset: true, upTo: max(Self.pageSize, have))
            if Task.isCancelled { return }
        }
    }

    /// Reads `day`'s entries until `upTo` are in (or the day ends), from its start (`reset`) or after what was read.
    private func loadDay(_ day: ActivityDay, reset: Bool, upTo: Int) async {
        guard model.screensMayReadStore else { return }
        if pages[day.key]?.loading == true { return }
        var page = reset ? DayPage() : (pages[day.key] ?? DayPage())
        if page.done || page.entries.count >= upTo { return }
        pages[day.key, default: DayPage()].loading = true
        let query = ActivityQuery(since: day.since, until: day.until)
        do {
            while page.entries.count < upTo && !page.done && model.screensMayReadStore {
                let limit = UInt32(min(200, max(Self.pageSize, upTo - page.entries.count)))
                let read = try await model.core.searchActivity(query: query, after: page.next, limit: limit)
                let entries = read.hits.map(\.entry)
                model.remember(entries)
                let have = Set(page.entries.map(\.id))
                page.entries += entries.filter { !have.contains($0.id) }
                page.next = read.next
                page.started = true
                if entries.isEmpty { break }
            }
        } catch {
            // What was read stays; scrolling to the end tries again.
        }
        page.loading = false
        pages[day.key] = page
    }

    // MARK: Search

    /// The matches by day, newest first, a page at a time, each day a group with its count (Next-Activity, 04).
    @ViewBuilder private var found: some View {
        if let r = results {
            if r.hits.isEmpty {
                VStack(spacing: 10) {
                    GlyphIcon(.search, size: 30).foregroundStyle(Palette.inkMeta)
                    Text("Nothing found").font(RFont.sans(17, .semibold)).foregroundStyle(Palette.ink)
                }
                .frame(maxWidth: .infinity)
                .padding(32)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier(model.activityFilters.isEmpty ? "nothingFound" : "noAutomatic")
            } else {
                let now = Timers.nowSeconds()
                let today = Date(timeIntervalSince1970: TimeInterval(now))
                let snippets = Dictionary(r.hits.map { ($0.entry.id, $0.snippet) }, uniquingKeysWith: { first, _ in first })
                let groups = EntryText.days(r.hits.map(\.entry))
                let lastId = r.hits.last?.entry.id
                let selected: Int64? = if case let .activityDetail(id)? = model.path(.activity).first { id } else { nil }
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(groups.enumerated()), id: \.element.day) { index, group in
                        HStack(spacing: 6) {
                            Text(EntryText.day(group.day, now: today))
                                .font(RFont.sans(13, .semibold))
                                .tracking(13 * TextRole.section.trackingEm)
                                .accessibilityAddTraits(.isHeader)
                            Spacer(minLength: 8)
                            Text("\(group.entries.count)")
                                .font(RFont.sans(13, .semibold))
                                .monospacedDigit()
                                .accessibilityLabel(index == 0 ? "\(r.total) found" : "\(group.entries.count) found")
                                .accessibilityIdentifier(index == 0 ? "searchTotal" : "")
                            GlyphIcon(.chevronDown, size: 14).foregroundStyle(Palette.inkMeta).padding(.leading, 2)
                        }
                        .foregroundStyle(Palette.ink2)
                        .padding(.horizontal, 24)
                        .padding(.top, index == 0 ? 14 : 22)
                        .padding(.bottom, 8)
                        ForEach(Array(group.entries.enumerated()), id: \.element.id) { i, entry in
                            row(entry, selected: selected == entry.id, now: now, terms: r.terms, snippet: snippets[entry.id] ?? nil, tracksSeen: false)
                                .groupSlice(first: i == 0, last: i == group.entries.count - 1, hairline: i > 0)
                                .onAppear {
                                    if entry.id == lastId { Task { await loadMoreResults() } }
                                }
                        }
                    }
                    if r.loadingMore {
                        ProgressView().frame(maxWidth: .infinity).padding(.vertical, 14)
                    }
                }
                .accessibilityContainer("searchResults")
            }
        } else {
            ProgressView().frame(maxWidth: .infinity).padding(.top, 32)
        }
    }

    /// The first page for `key`, after a short pause while words are typed (a newer key cancels this one).
    private func search(_ key: SearchKey) async {
        guard ActivityFilters.searching(text: key.text, filters: key.filters) else {
            results = nil
            return
        }
        if results?.key.text != key.text {
            try? await Task.sleep(for: .milliseconds(200))
            if Task.isCancelled { return }
        }
        let query = key.filters.query(text: key.text, now: Date(timeIntervalSince1970: TimeInterval(Timers.nowSeconds())))
        // Left in the background: the search runs again when the app comes back (its task's key changes).
        guard model.screensMayReadStore else { return }
        do {
            let page = try await model.core.searchActivity(query: query, after: nil, limit: UInt32(Self.pageSize))
            if Task.isCancelled { return }
            model.remember(page.hits.map(\.entry))
            results = SearchResults(key: key, query: query, hits: page.hits, next: page.next, total: page.total, terms: page.terms)
        } catch {
            if Task.isCancelled { return }
            results = SearchResults(key: key, query: query, hits: [], next: nil, total: 0, terms: [])
            model.notice = error.userMessage
        }
    }

    /// The next page of matches, when the last one shown appears.
    private func loadMoreResults() async {
        guard var r = results, let next = r.next, !r.loadingMore, model.screensMayReadStore else { return }
        results?.loadingMore = true
        do {
            let page = try await model.core.searchActivity(query: r.query, after: next, limit: UInt32(Self.pageSize))
            guard results?.key == r.key else { return }
            model.remember(page.hits.map(\.entry))
            let have = Set(r.hits.map(\.entry.id))
            r.hits += page.hits.filter { !have.contains($0.entry.id) }
            r.next = page.next
            r.total = page.total
            r.loadingMore = false
            results = r
        } catch {
            if results?.key == r.key { results?.loadingMore = false }
        }
    }

    // MARK: Rows

    private func row(
        _ entry: ActivityEntry, selected: Bool, now: Int64, terms: [String] = [], snippet: ActivitySnippet? = nil, tracksSeen: Bool = true
    ) -> some View {
        // A Button, so its press shows only after the scroll view's short wait and goes when the touch scrolls.
        Button {
            // The finger that swiped the row to "This was wrong" lifts on it: that is not a tap.
            guard !SidewaysPan.swiping else { return }
            model.show(.activityDetail(entry.id), in: .activity)
            feedback.defaultTap()
        } label: {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                HistoryRow(
                    entry: entry, selected: selected, glowing: model.fold.glowing(entry.id, now: context.date), now: now, terms: terms,
                    snippet: snippet
                )
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
            // Search results are not the river: what is seen is read there.
            guard tracksSeen else { return }
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
                guard model.screensMayReadStore, !Task.isCancelled else { return }
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
        let entries = searching ? [] : visibleRows()
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
        } else if atTop, !searching, let first = visibleRows().first {
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
        Rectangle()
            .fill(Palette.violet.opacity(0.5))
            .frame(height: 1)
            .padding(.leading, 62)
            .accessibilityLabel("New above")
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
                .fixedSize()
        }
        .padding(.top, 14)
        .padding(.bottom, 12)
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
        .padding(.horizontal, 24)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("syncNoticeBanner")
    }
}

extension View {
    /// Paper behind the status bar, fading into the page: rows scroll on beneath it and never run under the clock.
    /// For a root page without a navigation bar whose content is a plain scroll view (a list brings its own).
    func statusBarBackdrop(shown: Bool = true) -> some View {
        overlay(alignment: .top) {
            VStack(spacing: 0) {
                Palette.paper
                    .ignoresSafeArea(edges: .top)
                    .frame(height: 0)
                LinearGradient(colors: [Palette.paper, Palette.paper.opacity(0)], startPoint: .top, endPoint: .bottom)
                    .frame(height: 10)
            }
            .opacity(shown ? 1 : 0)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}
