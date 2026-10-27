package dev.reins.android.ui.activity

import dev.reins.android.design.fieldSurface
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.ui.focus.focusRequester
import dev.reins.android.design.pane
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.AppContainer
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Calm
import dev.reins.android.design.Field
import dev.reins.android.design.FieldHeader
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.LocalReduceMotion
import dev.reins.android.design.Motion
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.Timers
import dev.reins.android.design.askString
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.pressable
import dev.reins.android.platform.NotificationAccess
import dev.reins.android.platform.rememberNotificationState
import dev.reins.android.platform.rememberScreenLockState
import dev.reins.android.state.AppState
import dev.reins.android.ui.main.MoodStyle
import dev.reins.android.ui.main.RootHeader
import dev.reins.android.ui.main.mood
import dev.reins.core.PendingItem
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Stable
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.IntOffset
import dev.reins.android.state.ActivitySearch
import dev.reins.core.ActivityCursor
import dev.reins.core.ActivityDecider
import dev.reins.core.ActivityEntry
import dev.reins.core.ActivityFacets
import dev.reins.core.ActivityHit
import dev.reins.core.ActivityQuery
import dev.reins.core.ActivitySnippet
import dev.reins.core.ReinsCoreInterface
import java.time.ZoneId
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.draw.drawBehind

/** One row of the river as the list lays it out. */
private sealed interface RiverItem {
    val key: String

    /** A day that opens and closes (browsing). */
    data class Day(val day: DayCount, val label: String, val open: Boolean) : RiverItem {
        override val key get() = "d:${day.day}"
    }

    /** A day's label over search results; the first carries how many were found. */
    data class Label(val day: Long, val label: String, val total: Int?) : RiverItem {
        override val key get() = "l:$day"
    }

    /** An entry; [first]: the first of its day's (tagged "dayEntries:<day>"); [index]: its place among those shown. */
    data class Entry(val entry: ActivityEntry, val index: Int, val day: Long, val first: Boolean, val snippet: ActivitySnippet? = null, val atTop: Boolean = false) : RiverItem {
        override val key get() = "e:${entry.id}"
    }

    /** The end of what is loaded: drawing it reads the next page ([day] null: the search's). */
    data class More(val day: Long?) : RiverItem {
        override val key get() = "m:${day ?: "search"}"
    }
}

/** Something broken that a warn field names: what it is, in a few words, and the one thing to do. */
private data class Warning(val tag: String, val label: String, val ask: String, val target: String?, val action: String?, val actionTag: String?, val onAction: () -> Unit) {
    val glyph: Glyph get() = when (tag) {
        "notificationsOff" -> Glyph.Bell
        "screenLockOff" -> Glyph.Lock
        else -> Glyph.Phone
    }
}

/**
 * The first tab (Reins Signal, iOS-Activity / And-Core 01): what waits, as a stack of cards in their field colours;
 * when nothing does, the pond and a mood line; then the river of what was decided, by day, newest first: Today open,
 * the days before it closed until tapped. Words in the search field or a filter turn the river into what matches,
 * searched by the core on this phone. When Reins is broken (notifications off, no screen lock, another phone
 * approves), a warn field runs into the top.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ActivityScreen(
    container: AppContainer,
    notice: String?,
    onDismissNotice: () -> Unit,
    onOpenPending: (PendingItem) -> Unit,
    onOpenEntry: (Long) -> Unit,
    /** The mode chip and its menu, beside the title. */
    modeChip: @Composable () -> Unit = {},
    /** Answers a card (or a burst): Allow after the owner check (true) or Deny (false). Returns when done. */
    onAnswerBurst: suspend (Burst, Boolean) -> Unit = { _, _ -> },
    /** With nothing connected yet: pair a computer, or get the address for Claude.ai / ChatGPT. */
    onConnectComputer: () -> Unit = {},
    onConnectAi: () -> Unit = {},
    /** "Another phone approves": where to switch back. */
    onOpenSettings: () -> Unit = {},
    /** "Auto needs information": an answer (the owner check first unless Outside), or a dismiss. */
    onAnswerQuestion: (dev.reins.core.AutoQuestion, dev.reins.core.ZoneChoice) -> Unit = { _, _ -> },
    onDismissQuestion: (dev.reins.core.AutoQuestion) -> Unit = {},
) {
    val c = LocalColors.current
    val reduce = LocalReduceMotion.current
    val state: AppState = container.state
    val entries by state.activity.collectAsStateWithLifecycle()
    val pending by state.pending.collectAsStateWithLifecycle()
    val questions by state.autoQuestions.collectAsStateWithLifecycle()
    val loaded by state.loaded.collectAsStateWithLifecycle()
    val seen by state.seenActivityId.collectAsStateWithLifecycle()
    val replaced by state.deviceReplaced.collectAsStateWithLifecycle()
    val registrationError by state.registrationError.collectAsStateWithLifecycle()
    val syncNotice by state.syncNotice.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    val search by state.activitySearch.collectAsStateWithLifecycle()
    val opened by state.activityDays.collectAsStateWithLifecycle()
    var filtersShown by rememberSaveable { mutableStateOf(false) }
    // Search is a button beside the title; open, the field takes the header (Next, Activity 04).
    var searchOpen by rememberSaveable { mutableStateOf(state.activitySearch.value.text.isNotBlank()) }
    // The words as typed (the app state follows them; the search waits for a pause).
    var text by remember { mutableStateOf(state.activitySearch.value.text) }
    val filtersOpen = filtersShown || search.filtered
    val searching = search.searching
    val details = rememberPendingDetails(container, pending, entries)
    val cards = remember(pending) { waitCards(pending) }
    // Requests that became tickets wait in their own section above Today (Decisions v2).
    val tickets = remember(pending) { pending.filter { it.ticket != null } }
    val nowSeconds = (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000
    val zone = ZoneId.systemDefault()
    val today = localDay(nowSeconds, zone)
    val river = rememberRiver(container, entries, search, zone)
    // Who decided is Autopilot alone: each row shows Autopilot's share.
    val showShare = search.deciders == setOf(ActivityDecider.AUTOPILOT)

    // Browsing: the days (the core's stamps; until they are read, the entries at hand) and each open day's entries.
    val byDay = remember(entries, zone) { entries.groupBy { localDay(it.at, zone) } }
    val entryDays = remember(entries, zone) { daysOfEntries(entries, zone) }
    val stamped = river.days
    // A day that just began shows before the stamps are read again.
    val days = remember(stamped, entryDays) {
        stamped?.let { s -> (s + entryDays.filter { d -> s.none { it.day == d.day } }).sortedByDescending { it.day } } ?: entryDays
    }
    val pages = river.pages.toMap()
    val results = river.results
    val rows: List<RiverItem> = remember(searching, results, days, today, opened, byDay, pages, zone) {
        if (searching) searchRows(results, today, zone) else browseRows(days, today, opened, byDay, pages, zone)
    }
    val shown = remember(rows) { rows.mapNotNull { (it as? RiverItem.Entry)?.entry } }
    val newMarkerAt = if (searching) -1 else shown.indexOfLast { it.id > seen }
    // Open days read their entries (again when the activity changes).
    val openDays = if (searching) emptyList() else days.filter { dayOpen(it.day, today, opened) }.map { it.day }
    LaunchedEffect(openDays, river.version) { openDays.forEach { river.loadDay(it, zone) } }

    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val context = androidx.compose.ui.platform.LocalContext.current
    val notifications = rememberNotificationState()
    val screenLock = rememberScreenLockState()
    var refreshing by remember { mutableStateOf(false) }
    // People who never went through the setup (signed in before it existed) are asked once here.
    LaunchedEffect(Unit) {
        val ask = withContext(Dispatchers.Default) { !NotificationAccess.enabled(context) && !NotificationAccess.asked(context) }
        if (ask) notifications.request()
    }

    // The fold lands where the newest entry appears right after an answer: that row glows for 4 s.
    var answeredAt by remember { mutableLongStateOf(0L) }
    var landedId by remember { mutableStateOf<Long?>(null) }
    var newestSeen by remember { mutableStateOf(entries.firstOrNull()?.id ?: 0L) }
    var waitingBefore by remember { mutableStateOf(pending.size) }
    LaunchedEffect(pending.size) {
        if (pending.size < waitingBefore) answeredAt = System.currentTimeMillis()
        waitingBefore = pending.size
    }
    LaunchedEffect(entries.firstOrNull()?.id) {
        val newest = entries.firstOrNull()?.id ?: return@LaunchedEffect
        if (newest > newestSeen && System.currentTimeMillis() - answeredAt < LAND_WINDOW_MS) {
            landedId = newest
            delay(GLOW_MS)
            landedId = null
        }
        newestSeen = maxOf(newestSeen, newest)
    }

    val warnings = buildList {
        if (replaced) add(Warning("replaced", "Reins · Approvals", "Another phone approves", "approves", "Switch", "switchBack", onOpenSettings))
        registrationError?.let { add(Warning("registration", "Reins · Approvals", "Not your approval phone yet", null, null, null) {}) }
        if (!notifications.enabled) add(Warning("notificationsOff", "Reins · Notifications", "Notifications off", "off", "Turn on", "turnOnNotifications", notifications::request))
        if (!screenLock.set) add(Warning("screenLockOff", "Reins · Screen lock", "No screen lock", "lock", "Set one", "setScreenLock", screenLock::openSettings))
    }
    // The items above the river, in the order the list lays them out.
    val headerCount = (if (warnings.isEmpty()) 0 else 1) + 2 + (if (notice != null) 1 else 0) + (if (syncNotice != null) 1 else 0) +
        (if (filtersOpen) 1 else 0) + (if (searching) 0 else 1) + (if (searching || tickets.isEmpty()) 0 else 1)

    // Opening the tab lands where you stopped reading: at the oldest entry you have not seen, unless something is waiting.
    var positioned by remember { mutableStateOf(false) }
    LaunchedEffect(entries.isNotEmpty()) {
        if (positioned || entries.isEmpty()) return@LaunchedEffect
        positioned = true
        if (searching) return@LaunchedEffect
        val oldestUnseen = shown.indexOfLast { it.id > seen }
        if (oldestUnseen > 2 && pending.isEmpty()) {
            val at = rows.indexOfFirst { it is RiverItem.Entry && it.index == oldestUnseen }
            if (at >= 0) listState.scrollToItem((headerCount + at - 1).coerceAtLeast(0))
        }
    }
    // What is on screen counts as seen: scroll up to the newest and the badge clears. Recorded once the list comes to
    // rest, not on every frame of a fling. Search results are not the river: looking at them marks nothing.
    LaunchedEffect(shown, searching) {
        if (searching) return@LaunchedEffect
        snapshotFlow { listState.layoutInfo.visibleItemsInfo.map { it.key } }.collectLatest { keys ->
            delay(SEEN_SETTLE_MS)
            val visible = keys.mapNotNull { (it as? String)?.takeIf { k -> k.startsWith("e:") }?.removePrefix("e:")?.toLongOrNull() }
            val newest = visible.maxOrNull() ?: shown.firstOrNull()?.id.takeIf { listState.firstVisibleItemIndex == 0 && keys.isNotEmpty() }
            if (newest != null) container.markActivitySeen(newest)
        }
    }
    val away by remember { derivedStateOf { listState.firstVisibleItemIndex > 3 } }

    val appearance by dev.reins.android.design.Appearance.state(context)
    // The art shows while browsing; a search is plain paper (Next, Activity 04).
    val art = if (searchOpen) null else appearance.wallpaper(c.dark)
    val computers = connections.count { it.keyFingerprint != null }
    val ais = connections.size - computers
    // Which pane each item belongs to, and whether it holds the pane's top or bottom edge.
    val edges = remember(rows) { paneEdges(rows) }

    dev.reins.android.design.WallpaperPage(art, Modifier.fillMaxSize()) {
    PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = {
            refreshing = true
            scope.launch {
                container.refreshPending()
                refreshing = false
            }
        },
        modifier = Modifier.fillMaxSize(),
    ) {
        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize().testTag("activityList"),
            contentPadding = PaddingValues(bottom = 32.dp),
        ) {
            if (warnings.isNotEmpty()) item(key = "warn") { WarnField(warnings, registrationError) }
            // The header stays pinned under the status bar while the list scrolls; a scrim fades in behind it once rows
            // pass under, so the title and buttons stay readable over them.
            stickyHeader(key = "title") {
                val scrim = if (c.dark) Color.Black.copy(alpha = 0.82f) else c.paper.copy(alpha = 0.9f)
                val shown by androidx.compose.animation.core.animateFloatAsState(
                    if (listState.canScrollBackward) 1f else 0f,
                    label = "headerScrim",
                )
                Box(
                    Modifier.drawBehind {
                        if (shown > 0f) {
                            val fade = 28.dp.toPx()
                            drawRect(
                                Brush.verticalGradient(
                                    0f to scrim.copy(alpha = scrim.alpha * shown),
                                    size.height / (size.height + fade) to scrim.copy(alpha = scrim.alpha * shown),
                                    1f to Color.Transparent,
                                    endY = size.height + fade,
                                ),
                                size = size.copy(height = size.height + fade),
                            )
                        }
                    },
                ) {
                if (searchOpen) {
                    SearchHeader(
                        text,
                        underField = warnings.isNotEmpty(),
                        onChange = {
                            text = it
                            state.setActivitySearch(state.activitySearch.value.copy(text = it))
                        },
                        onCancel = {
                            text = ""
                            state.setActivitySearch(state.activitySearch.value.copy(text = ""))
                            searchOpen = false
                        },
                        filter = { FilterButton(filtersOpen) { filtersShown = !filtersOpen } },
                    )
                } else {
                    RootHeader("Activity", underField = warnings.isNotEmpty()) {
                        modeChip()
                        HeaderButton(Glyph.Search, "Search", "openSearch") { searchOpen = true }
                        FilterButton(filtersOpen) { filtersShown = !filtersOpen }
                    }
                }
            }
            }
            notice?.let { item(key = "notice") { NoticeRow(it, onDismissNotice) } }
            // What the core says about syncing with the account's other phone (#101): a soft row with the sync glyph and
            // OK, which dismisses it in the core. Sync never keeps anyone out; this row is all it shows.
            syncNotice?.let {
                item(key = "syncNotice") {
                    NoticeRow(it, { scope.launch { container.dismissSyncNotice() } }, glyph = Glyph.Refresh, tag = "syncNotice", okTag = "dismissSyncNotice", maxLines = 3)
                }
            }
            if (filtersOpen) {
                item(key = "filters") {
                    FilterRow(search, river.facets, connections, Modifier.padding(top = 12.dp)) { state.setActivitySearch(it) }
                }
            }
            if (!searching) {
                item(key = "stack") {
                    // The art stays open above what waits (Next: 168 dp with nothing waiting, 70 over the cards).
                    val open = when {
                        art == null || filtersOpen -> 14.dp
                        pending.isEmpty() && (ais > 0 && computers > 0) -> 168.dp
                        else -> 70.dp
                    }
                    Column(Modifier.padding(top = open)) {
                        WaitingStack(
                            cards,
                            details,
                            onReview = onOpenPending,
                            onAnswer = { burst, approve ->
                                answeredAt = System.currentTimeMillis()
                                onAnswerBurst(burst, approve)
                            },
                            onLater = { item -> container.keepWaitingInBackground(item.id, "Not moved") { container.core.decideLater(item.id) } },
                        )
                        if (cards.isNotEmpty()) Spacer(Modifier.height(if (cards.size > 1) 30.dp else 12.dp))
                        // After what waits: what Auto needs to know, in the same shell (never notified).
                        if (questions.isNotEmpty()) {
                            QuestionCards(questions, onAnswerQuestion, onDismissQuestion)
                            Spacer(Modifier.height(12.dp))
                        }
                        AnimatedVisibility(
                            pending.isEmpty() && questions.isEmpty() && loaded && !filtersOpen,
                            enter = fadeIn(if (reduce) Motion.reduced() else Motion.fold(false)),
                            exit = fadeOut(Motion.reduced()),
                        ) {
                            Quiet(ais, computers, entries.isEmpty(), onConnectComputer, onConnectAi)
                        }
                    }
                }
                if (tickets.isNotEmpty()) {
                    item(key = "waiting") {
                        WaitingInbox(
                            tickets,
                            details,
                            onOpen = onOpenPending,
                            onBatch = { burst, approve ->
                                scope.launch {
                                    answeredAt = System.currentTimeMillis()
                                    onAnswerBurst(burst, approve)
                                }
                            },
                            modifier = Modifier.padding(bottom = 10.dp),
                        )
                    }
                }
            }
            if (searching && results != null && !results.loading && results.hits.isEmpty()) {
                item(key = "empty") {
                    val tag = when {
                        search.text.isNotBlank() -> "noResults"
                        search.deciders == setOf(ActivityDecider.AUTOPILOT) -> "noAutomatic"
                        else -> "noFiltered"
                    }
                    NothingFound(Modifier.testTag(tag))
                }
            }
            val terms = results?.terms.orEmpty()
            rows.forEachIndexed { at, row ->
                val (top, bottom) = edges[at]
                val gap = top && at > 0
                when (row) {
                    is RiverItem.Day -> item(key = row.key, contentType = "day") {
                        Box(Modifier.animateItem(fadeInSpec = null, fadeOutSpec = null, placementSpec = placement(reduce)).riverPiece(top, bottom, gap)) {
                            DayHeader(row.day, row.label, row.open) {
                                state.setActivityDayOpen(row.day.day, !row.open)
                            }
                        }
                    }
                    is RiverItem.Label -> item(key = row.key, contentType = "label") {
                        Box(Modifier.riverPiece(top, bottom, gap)) {
                            DayLabel(
                                row.label,
                                row.total?.let { if (it == 1) "1 found" else "$it found" },
                                if (row.total != null) Modifier.testTag("searchResults") else Modifier,
                            )
                        }
                    }
                    is RiverItem.Entry -> item(key = row.key, contentType = "entry") {
                        Column(
                            Modifier
                                .animateItem(fadeInSpec = fade(reduce), placementSpec = placement(reduce), fadeOutSpec = fade(reduce))
                                .riverPiece(top, bottom, gap)
                                .then(if (row.first && !searching) Modifier.testTag("dayEntries:${row.day}") else Modifier),
                        ) {
                            if (row.index == newMarkerAt) NewMarker()
                            Box(Modifier.testTag("entry:${row.entry.id}")) {
                                RiverRow(
                                    row.entry,
                                    nowSeconds,
                                    landed = row.entry.id == landedId,
                                    showShare = showShare,
                                    terms = if (searching) terms else emptyList(),
                                    snippet = row.snippet,
                                    divider = !top && rows.getOrNull(at - 1) is RiverItem.Entry && row.index != newMarkerAt,
                                ) { onOpenEntry(row.entry.id) }
                            }
                        }
                    }
                    is RiverItem.More -> item(key = row.key, contentType = "more") {
                        LaunchedEffect(row) { if (row.day == null) river.loadMoreResults() else river.loadMoreDay(row.day) }
                        Box(Modifier.riverPiece(top, bottom, gap).fillMaxWidth().height(10.dp))
                    }
                }
            }
        }
        // Back to the newest.
        if (away) {
            Row(
                Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 20.dp)
                    .clip(CircleShape)
                    .background(c.inverse)
                    .pressable(shape = CircleShape, label = "Latest") { scope.launch { listState.animateScrollToItem(0) } }
                    .padding(horizontal = 16.dp, vertical = 10.dp)
                    .testTag("scrollToLatest"),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                GlyphIcon(Glyph.ChevronUp, c.onInverse, size = 16.dp)
                RText("Latest", RType.sans(14f, FontWeight.SemiBold), c.onInverse, maxLines = 1)
            }
        }
    }
    }
}

/** A piece of a day's pane: each day is its own pane, 16 in from the edges; panes stand 10 apart ([gap]). */
@Composable
private fun Modifier.riverPiece(top: Boolean, bottom: Boolean, gap: Boolean): Modifier =
    padding(start = 16.dp, end = 16.dp, top = if (gap) 10.dp else 0.dp).pane(top = top, bottom = bottom)

/**
 * For each river item, whether it holds the top and the bottom edge of its pane: a day (or a search's day label) starts
 * a pane, which runs to the item before the next day.
 */
private fun paneEdges(rows: List<RiverItem>): List<Pair<Boolean, Boolean>> = rows.indices.map { i ->
    val starts = { r: RiverItem? -> r is RiverItem.Day || r is RiverItem.Label }
    val top = i == 0 || starts(rows[i])
    val bottom = i == rows.lastIndex || starts(rows[i + 1])
    top to bottom
}

private fun fade(reduce: Boolean): FiniteAnimationSpec<Float> = if (reduce) Motion.reduced() else tween(Motion.FADE_MS * 2)

private fun placement(reduce: Boolean): FiniteAnimationSpec<IntOffset>? = if (reduce) null else Motion.fold(false)

/** The river while browsing: each day's header, then, when it is open, its entries and where the next page starts. */
private fun browseRows(
    days: List<DayCount>,
    today: Long,
    opened: Map<Long, Boolean>,
    byDay: Map<Long, List<ActivityEntry>>,
    pages: Map<Long, DayPage>,
    zone: ZoneId,
): List<RiverItem> = buildList {
    var index = 0
    for (d in days) {
        val open = dayOpen(d.day, today, opened)
        add(RiverItem.Day(d, dayTitle(d.day, today), open))
        if (!open) continue
        val page = pages[d.day]
        // What the core read for the day, with what the app holds already (the newest arrive there first).
        val dayEntries = (byDay[d.day].orEmpty() + page?.entries.orEmpty())
            .distinctBy { it.id }
            .filter { localDay(it.at, zone) == d.day }
            .sortedWith(NEWEST_FIRST)
        dayEntries.forEachIndexed { i, e -> add(RiverItem.Entry(e, index++, d.day, first = i == 0)) }
        if (page?.next != null) add(RiverItem.More(d.day))
    }
}

/** The search's results under their days' labels, the first with how many were found; then where the next page starts. */
private fun searchRows(results: Results?, today: Long, zone: ZoneId): List<RiverItem> = buildList {
    if (results == null) return@buildList
    var day: Long? = null
    results.hits.forEachIndexed { i, hit ->
        val d = localDay(hit.entry.at, zone)
        if (d != day) {
            add(RiverItem.Label(d, dayTitle(d, today), if (day == null) results.total else null))
            day = d
        }
        add(RiverItem.Entry(hit.entry, i, d, first = false, snippet = hit.snippet))
    }
    if (results.next != null) add(RiverItem.More(null))
}

private val NEWEST_FIRST = compareByDescending<ActivityEntry> { it.at }.thenByDescending { it.id }

/** A day's entries as read so far, and where the next page starts (null: all read). */
internal data class DayPage(val entries: List<ActivityEntry>, val next: ActivityCursor?, val version: Int)

/** A search's results as read so far, for [search]; [loading] until its first page is in. */
internal data class Results(
    val search: ActivitySearch,
    val hits: List<ActivityHit>,
    val next: ActivityCursor?,
    val total: Int,
    val terms: List<String>,
    val loading: Boolean = false,
)

/**
 * What the river reads from the core: the days and their counts, each open day's entries page by page, the search's
 * results and the filters' choices. Read again whenever the activity changes ([version]).
 */
@Stable
internal class River(private val core: ReinsCoreInterface, private val scope: CoroutineScope) {
    var version by mutableIntStateOf(0)
    var days by mutableStateOf<List<DayCount>?>(null)
    val pages = mutableStateMapOf<Long, DayPage>()
    var results by mutableStateOf<Results?>(null)
    var facets by mutableStateOf<ActivityFacets?>(null)
    /** The version each day is being read for. */
    private val reading = mutableMapOf<Long, Int>()
    private val readingMore = mutableSetOf<Long>()
    private var loadingMore = false

    suspend fun loadDays(zone: ZoneId) {
        quietly { core.activityStamps(ActivityQuery()) }?.let { days = daysOf(it, zone) }
        quietly { core.activityFacets() }?.let { facets = it }
    }

    /** Reads [day]'s entries unless they are read for this [version]: as many as were read before, at least a page. */
    suspend fun loadDay(day: Long, zone: ZoneId) {
        val had = pages[day]
        val v = version
        if (had?.version == v || reading[day] == v) return
        reading[day] = v
        try {
            val limit = (had?.entries?.size ?: 0).coerceIn(DAY_PAGE, PAGE_MAX)
            val page = quietly { core.searchActivity(dayQuery(day, zone), null, limit.toUInt()) } ?: return
            if (version == v) pages[day] = DayPage(page.hits.map { it.entry }, page.next, v)
        } finally {
            if (reading[day] == v) reading.remove(day)
        }
    }

    fun loadMoreDay(day: Long) {
        val had = pages[day] ?: return
        val after = had.next ?: return
        if (!readingMore.add(day)) return
        scope.launch {
            try {
                val zone = ZoneId.systemDefault()
                val page = quietly { core.searchActivity(dayQuery(day, zone), after, DAY_PAGE.toUInt()) } ?: return@launch
                val now = pages[day] ?: return@launch
                if (now.next != after) return@launch
                pages[day] = now.copy(entries = (now.entries + page.hits.map { it.entry }).distinctBy { it.id }, next = page.next)
            } finally {
                readingMore.remove(day)
            }
        }
    }

    /** The first page of [search]'s results (as many as were read before when only the activity changed). */
    suspend fun search(search: ActivitySearch, nowSeconds: Long, zone: ZoneId) {
        val had = results?.takeIf { it.search == search }
        // Until the new results are in, the ones before stay (marked as reading, so "Nothing found" waits too).
        if (had == null) results = results?.copy(loading = true) ?: Results(search, emptyList(), null, 0, emptyList(), loading = true)
        val limit = (had?.hits?.size ?: 0).coerceIn(SEARCH_PAGE, PAGE_MAX)
        val page = quietly { core.searchActivity(search.query(nowSeconds, zone), null, limit.toUInt()) } ?: return
        results = Results(search, page.hits, page.next, page.total.toInt(), page.terms)
    }

    fun loadMoreResults() {
        val had = results ?: return
        val after = had.next ?: return
        if (loadingMore) return
        loadingMore = true
        scope.launch {
            try {
                val nowSeconds = (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000
                val page = quietly { core.searchActivity(had.search.query(nowSeconds, ZoneId.systemDefault()), after, SEARCH_PAGE.toUInt()) } ?: return@launch
                val now = results ?: return@launch
                if (now.search != had.search || now.next != after) return@launch
                results = now.copy(hits = (now.hits + page.hits).distinctBy { it.entry.id }, next = page.next)
            } finally {
                loadingMore = false
            }
        }
    }

    private companion object {
        const val DAY_PAGE = 50
        const val SEARCH_PAGE = 50
        const val PAGE_MAX = 200
    }
}

/**
 * The river's reads, kept up to date: the days when the activity changes, and the search a moment after the words
 * stop changing (a newer search cancels the one before).
 */
@Composable
private fun rememberRiver(container: AppContainer, entries: List<ActivityEntry>, search: ActivitySearch, zone: ZoneId): River {
    val scope = rememberCoroutineScope()
    val river = remember { River(container.core, scope) }
    LaunchedEffect(entries, zone) {
        river.version++
        river.loadDays(zone)
    }
    var lastText by remember { mutableStateOf(search.text) }
    LaunchedEffect(search, river.version) {
        if (!search.searching) {
            river.results = null
            lastText = search.text
            return@LaunchedEffect
        }
        if (search.text != lastText) delay(SEARCH_DEBOUNCE_MS)
        lastText = search.text
        river.search(search, (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000, zone)
    }
    return river
}

/**
 * The header while searching (Next, Activity 04): the field (a sunken well with the search glyph and a clear button
 * once something is typed) and Cancel in violet, which clears the words and closes the search.
 */
@Composable
private fun SearchHeader(text: String, underField: Boolean, onChange: (String) -> Unit, onCancel: () -> Unit, filter: @Composable () -> Unit = {}) {
    val c = LocalColors.current
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    val style = RType.sans(16f)
    Row(
        Modifier
            .fillMaxWidth()
            .then(if (underField) Modifier.padding(top = 18.dp) else Modifier.windowInsetsPadding(dev.reins.android.ui.main.HeaderInsets).padding(top = 8.dp))
            .padding(horizontal = 20.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier.weight(1f).heightIn(min = 44.dp).clip(RoundedCornerShape(14.dp)).background(c.paperSunken).padding(start = 12.dp, end = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GlyphIcon(Glyph.Search, c.inkMeta, size = 18.dp)
            Box(Modifier.weight(1f).padding(vertical = 11.dp)) {
                if (text.isEmpty()) RText("Search", style, c.inkMeta, maxLines = 1)
                BasicTextField(
                    text,
                    onChange,
                    Modifier.fillMaxWidth().focusRequester(focus).testTag("activitySearch").semantics { contentDescription = "Search Activity" },
                    textStyle = style.copy(color = c.ink),
                    cursorBrush = SolidColor(c.violet),
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = { keyboard?.hide() }),
                )
            }
            if (text.isNotEmpty()) {
                Box(
                    Modifier
                        .size(32.dp)
                        .clip(CircleShape)
                        .pressable(shape = CircleShape, label = "Clear search") { onChange("") }
                        .semantics { contentDescription = "Clear search" }
                        .testTag("activitySearchClear"),
                    contentAlignment = Alignment.Center,
                ) { GlyphIcon(Glyph.CircleClose, c.inkMeta, size = 18.dp) }
            }
        }
        // The filters narrow a search too (the board draws none here; they stay one tap away).
        filter()
        RText(
            "Cancel",
            RType.sans(16f, FontWeight.SemiBold),
            c.violet,
            Modifier.pressable(shape = RoundedCornerShape(8.dp), label = "Cancel search", onClick = onCancel).padding(vertical = 8.dp).testTag("searchCancel"),
            maxLines = 1,
        )
    }
    // Opened by the button (not restored with words in it): the keyboard comes up.
    LaunchedEffect(Unit) { if (text.isEmpty()) runCatching { focus.requestFocus() } }
}

/** A search or filters that match nothing. */
@Composable
private fun NothingFound(modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Column(
        modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 36.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        GlyphIcon(Glyph.Search, c.inkFaint, size = 28.dp)
        RText("Nothing found", RType.sans(16f, FontWeight.Medium), c.ink2)
    }
}

/** How long the words must stay still before they are searched. */
private const val SEARCH_DEBOUNCE_MS = 200L

/** A round 40 dp header button (`.ib`): raised with a hairline. */
@Composable
private fun HeaderButton(glyph: Glyph, label: String, tag: String, on: Boolean = false, dot: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier
            .size(40.dp)
            .clip(CircleShape)
            .background(if (on) c.inverse else c.paperRaised)
            .then(if (on) Modifier else Modifier.border(1.dp, c.rule, CircleShape))
            .pressable(shape = CircleShape, label = label, onClick = onClick)
            .semantics { contentDescription = label }
            .testTag(tag),
        contentAlignment = Alignment.Center,
    ) { GlyphIcon(glyph, if (on) c.onInverse else c.ink, size = 20.dp) }
}

/** The round filter button beside the mode chip: ink when the filters are open. */
@Composable
private fun FilterButton(open: Boolean, onClick: () -> Unit) = HeaderButton(Glyph.Filter, "Filter", "filters", on = open, onClick = onClick)

/**
 * Reins is broken and needs you (Next, Activity 02): an amber bar under the status bar, its bottom corners round, each
 * problem a line with its glyph and the one thing to do on a white button.
 */
@Composable
private fun WarnField(warnings: List<Warning>, detail: String?) {
    val c = LocalColors.current
    Column(
        Modifier
            .fillMaxWidth()
            .fieldSurface(Field.Warn, RoundedCornerShape(bottomStart = 26.dp, bottomEnd = 26.dp))
            .windowInsetsPadding(dev.reins.android.ui.main.HeaderInsets)
            .padding(start = 20.dp, end = 20.dp, top = 8.dp, bottom = 14.dp)
            .testTag("warnField"),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        warnings.forEach { w ->
            Row(Modifier.fillMaxWidth().testTag(w.tag), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                GlyphIcon(w.glyph, c.onField, size = 20.dp)
                Column(Modifier.weight(1f)) {
                    RText(w.ask, RType.sans(16f, FontWeight.SemiBold), c.onField, maxLines = 2)
                    if (w.tag == "registration" && detail != null) RText(detail, RType.sans(13f), c.onField2, Modifier.padding(top = 2.dp), maxLines = 3)
                }
                if (w.action != null) {
                    SignalButton(w.action, if (w.actionTag != null) Modifier.testTag(w.actionTag) else Modifier, style = ButtonStyle.OnFieldWhite, size = ButtonSize.Small, onClick = w.onAction)
                }
            }
        }
    }
}

/** A line about something that just happened, on paper, with OK. */
@Composable
private fun NoticeRow(
    text: String,
    onDismiss: () -> Unit,
    glyph: Glyph = Glyph.Info,
    tag: String = "notice",
    okTag: String = "dismissNotice",
    maxLines: Int = 4,
) {
    val c = LocalColors.current
    // A raised card like the waiting and connect cards, solid so it reads over the wallpaper: the glyph in its own
    // tile, the line, and OK.
    val shape = RoundedCornerShape(26.dp)
    Row(
        Modifier
            .padding(horizontal = 16.dp)
            .padding(top = 12.dp)
            .fillMaxWidth()
            .clip(shape)
            .background(c.paperRaised)
            .then(if (!c.dark) Modifier.border(1.dp, c.rule, shape) else Modifier)
            .padding(horizontal = 16.dp, vertical = 14.dp)
            .testTag(tag),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier.size(34.dp).clip(RoundedCornerShape(10.dp)).background(c.violet.copy(alpha = 0.14f)),
            contentAlignment = Alignment.Center,
        ) {
            GlyphIcon(glyph, c.violet, size = 18.dp)
        }
        RText(text, RType.sans(15f), c.ink, Modifier.weight(1f), maxLines = maxLines)
        SignalButton("OK", Modifier.testTag(okTag), size = ButtonSize.Compact, onClick = onDismiss)
    }
}

/**
 * Nothing waits. With an AI app and a computer both connected the art stays open and this is empty; with one of them
 * missing, the missing half takes the action slot (Next, "Activity with a wallpaper" 05 and 05b): a raised card that
 * connects an AI app, or an ink card that connects a computer.
 */
@Composable
private fun Quiet(ais: Int, computers: Int, noHistory: Boolean, onConnectComputer: () -> Unit, onConnectAi: () -> Unit) {
    val c = LocalColors.current
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = if (ais == 0 || computers == 0) 12.dp else 0.dp).testTag(if (noHistory) "noActivity" else "allQuiet"),
    ) {
        val shape = RoundedCornerShape(26.dp)
        if (ais == 0) {
            Column(
                Modifier.fillMaxWidth().clip(shape).background(c.paperRaised).then(if (!c.dark) Modifier.border(1.dp, c.rule, shape) else Modifier).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy((-8).dp)) {
                    dev.reins.android.design.ProviderAvatar(dev.reins.android.design.Providers.byKey("claude")!!, size = 34.dp)
                    dev.reins.android.design.ProviderAvatar(dev.reins.android.design.Providers.byKey("openai")!!, size = 34.dp)
                }
                RText("Connect an AI app", RType.askSmall, c.ink, maxLines = 2)
                SignalButton("Connect", Modifier.fillMaxWidth().testTag("emptyConnectAi"), size = ButtonSize.Small, glyph = Glyph.Plus, onClick = onConnectAi)
            }
        }
        if (ais == 0 && computers == 0) Spacer(Modifier.height(10.dp))
        if (computers == 0) {
            // An ink card: the computer's own command in mono, then the two ways to pair.
            Column(
                Modifier.fillMaxWidth().clip(shape).background(c.inverse).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(34.dp).clip(RoundedCornerShape(10.dp)).background(c.onInverse), contentAlignment = Alignment.Center) {
                        GlyphIcon(Glyph.Laptop, c.inverse, size = 18.dp)
                    }
                    Spacer(Modifier.weight(1f))
                    RText("reins login", RType.mono(13f), c.onInverse.copy(alpha = 0.7f), maxLines = 1, ltr = true)
                }
                RText("Connect a computer", RType.askSmall, c.onInverse, maxLines = 2)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SignalButton("Scan", Modifier.weight(1f).testTag("emptyConnectComputer"), style = ButtonStyle.OnFieldWhite, size = ButtonSize.Small, glyph = Glyph.QrCode, onClick = onConnectComputer)
                    SignalButton(
                        "Type code", Modifier.weight(1f).testTag("emptyTypeCode"),
                        style = if (c.dark) ButtonStyle.Soft else ButtonStyle.OnFieldWash, size = ButtonSize.Small, glyph = Glyph.Pencil, onClick = onConnectComputer,
                    )
                }
            }
        }
    }
}

/** Where the entries you have not seen begin. */
@Composable
private fun NewMarker() {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 4.dp).testTag("newMarker"), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f).height(1.dp).background(c.violet.copy(alpha = 0.4f)))
        RText("New above", RType.sans(11.5f, FontWeight.SemiBold), c.violet, Modifier.padding(horizontal = 10.dp))
        Box(Modifier.weight(1f).height(1.dp).background(c.violet.copy(alpha = 0.4f)))
    }
}

/** How long the list must stay still before what it shows counts as seen. */
private const val SEEN_SETTLE_MS = 300L

/** A new entry this soon after an answer is where the fold landed. */
private const val LAND_WINDOW_MS = 5_000L

/** How long the landed row glows. */
private const val GLOW_MS = 4_000L
