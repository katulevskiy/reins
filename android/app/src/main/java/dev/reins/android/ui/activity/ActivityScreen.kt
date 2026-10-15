package dev.reins.android.ui.activity

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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** One row of the river as the list lays it out: a day's label, or an entry. */
private sealed interface RiverItem {
    data class Day(val label: String, val count: String?) : RiverItem

    data class Entry(val entry: dev.reins.core.ActivityEntry, val index: Int) : RiverItem
}

/** Something broken that a warn field names: what it is, in a few words, and the one thing to do. */
private data class Warning(val tag: String, val label: String, val ask: String, val target: String?, val action: String?, val actionTag: String?, val onAction: () -> Unit)

/**
 * The first tab (Reins Signal, iOS-Activity / And-Core 01): what waits, as a stack of cards in their field colours;
 * when nothing does, the pond and a mood line; then the river of what was decided, by day, newest first. When Reins is
 * broken (notifications off, no screen lock, another phone approves), a warn field runs into the top.
 */
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
) {
    val c = LocalColors.current
    val reduce = LocalReduceMotion.current
    val state: AppState = container.state
    val entries by state.activity.collectAsStateWithLifecycle()
    val pending by state.pending.collectAsStateWithLifecycle()
    val loaded by state.loaded.collectAsStateWithLifecycle()
    val seen by state.seenActivityId.collectAsStateWithLifecycle()
    val replaced by state.deviceReplaced.collectAsStateWithLifecycle()
    val registrationError by state.registrationError.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    var filter by rememberSaveable { mutableStateOf(HistoryFilter.All) }
    var ai by rememberSaveable { mutableStateOf<String?>(null) }
    var filtersOpen by rememberSaveable { mutableStateOf(false) }
    val details = rememberPendingDetails(container, pending, entries)
    val cards = remember(pending) { waitCards(pending) }
    val shown = remember(entries, filter, ai) { entries.filter { filter.matches(it) && (ai == null || it.connectionId == ai) } }
    val nowSeconds = (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000
    // The river, by day; Today carries its count.
    val river = remember(shown, nowSeconds / 3_600, filter) {
        buildList {
            var day: Long? = null
            val today = dayOf(nowSeconds)
            shown.forEachIndexed { i, e ->
                val d = dayOf(e.at)
                if (d != day) {
                    add(RiverItem.Day(dayLabel(e.at, nowSeconds), if (d == today) dayCount(shown.filter { dayOf(it.at) == d }, filter == HistoryFilter.Autopilot) else null))
                    day = d
                }
                add(RiverItem.Entry(e, i))
            }
        }
    }
    val newMarkerAt = remember(shown, seen) { shown.indexOfLast { it.id > seen } }
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
        if (!notifications.enabled) add(Warning("notificationsOff", "Reins · Notifications", "Notifications are off", "off", "Turn on", "turnOnNotifications", notifications::request))
        if (!screenLock.set) add(Warning("screenLockOff", "Reins · Screen lock", "No screen lock", "lock", "Set one", "setScreenLock", screenLock::openSettings))
    }
    val headerCount = (if (warnings.isEmpty()) 0 else 1) + 1 + (if (notice != null) 1 else 0) + (if (filtersOpen) 1 else 0) + 1

    // Opening the tab lands where you stopped reading: at the oldest entry you have not seen, unless something is waiting.
    var positioned by remember { mutableStateOf(false) }
    LaunchedEffect(entries.isNotEmpty()) {
        if (positioned || entries.isEmpty()) return@LaunchedEffect
        positioned = true
        val oldestUnseen = shown.indexOfLast { it.id > seen }
        if (oldestUnseen > 2 && pending.isEmpty()) {
            val at = river.indexOfFirst { it is RiverItem.Entry && it.index == oldestUnseen }
            if (at >= 0) listState.scrollToItem((headerCount + at - 1).coerceAtLeast(0))
        }
    }
    // What is on screen counts as seen: scroll up to the newest and the badge clears. Recorded once the list comes to
    // rest, not on every frame of a fling.
    LaunchedEffect(shown) {
        snapshotFlow { listState.layoutInfo.visibleItemsInfo.map { it.key } }.collectLatest { keys ->
            delay(SEEN_SETTLE_MS)
            val visible = keys.mapNotNull { (it as? String)?.takeIf { k -> k.startsWith("e:") }?.removePrefix("e:")?.toLongOrNull() }
            val newest = visible.maxOrNull() ?: shown.firstOrNull()?.id.takeIf { listState.firstVisibleItemIndex == 0 && keys.isNotEmpty() }
            if (newest != null) container.markActivitySeen(newest)
        }
    }
    val away by remember { derivedStateOf { listState.firstVisibleItemIndex > 3 } }

    PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = {
            refreshing = true
            scope.launch {
                container.refreshPending()
                refreshing = false
            }
        },
        modifier = Modifier.fillMaxSize().background(c.paper),
    ) {
        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize().testTag("activityList"),
            contentPadding = PaddingValues(bottom = 32.dp),
        ) {
            if (warnings.isNotEmpty()) item(key = "warn") { WarnField(warnings, registrationError) }
            item(key = "title") {
                RootHeader(
                    "Activity",
                    mood = if (pending.isNotEmpty()) waitingMood(pending.size).let { (lead, rest) -> mood(lead, rest) } else null,
                    underField = warnings.isNotEmpty(),
                ) {
                    modeChip()
                    FilterButton(filtersOpen) { filtersOpen = !filtersOpen }
                }
            }
            notice?.let { item(key = "notice") { NoticeRow(it, onDismissNotice) } }
            if (filtersOpen) {
                item(key = "filters") {
                    FilterRow(filter, ai, connections, Modifier.padding(top = 14.dp), onSelect = { filter = it }, onAi = { ai = it })
                }
            }
            item(key = "stack") {
                Column(Modifier.padding(top = if (pending.isEmpty()) 0.dp else 18.dp)) {
                    WaitingStack(
                        cards,
                        details,
                        onReview = onOpenPending,
                        onAnswer = { burst, approve ->
                            answeredAt = System.currentTimeMillis()
                            onAnswerBurst(burst, approve)
                        },
                    )
                    AnimatedVisibility(
                        pending.isEmpty() && loaded && filter == HistoryFilter.All && ai == null && !filtersOpen,
                        enter = fadeIn(if (reduce) Motion.reduced() else Motion.fold(false)),
                        exit = fadeOut(Motion.reduced()),
                    ) {
                        Quiet(connections.size, entries.isEmpty(), onConnectComputer, onConnectAi)
                    }
                }
            }
            if (shown.isEmpty() && loaded && (filter != HistoryFilter.All || ai != null)) {
                item(key = "empty") {
                    RText(
                        "Nothing here.",
                        MoodStyle,
                        c.ink2,
                        Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 28.dp).testTag(if (filter == HistoryFilter.Autopilot) "noAutomatic" else "noFiltered"),
                    )
                }
            } else {
                river.forEach { row ->
                    when (row) {
                        is RiverItem.Day -> item(key = "d:${row.label}", contentType = "day") { DayLabel(row.label, row.count) }
                        is RiverItem.Entry -> item(key = "e:${row.entry.id}", contentType = "entry") {
                            Column {
                                if (row.index == newMarkerAt) NewMarker()
                                Box(Modifier.testTag("entry:${row.entry.id}")) {
                                    RiverRow(row.entry, nowSeconds, landed = row.entry.id == landedId, showShare = filter == HistoryFilter.Autopilot) { onOpenEntry(row.entry.id) }
                                }
                            }
                        }
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

/** The round filter button beside the mode chip: ink when the filters are open. */
@Composable
private fun FilterButton(open: Boolean, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier
            .size(40.dp)
            .clip(CircleShape)
            .background(if (open) c.inverse else c.paperRaised)
            .then(if (open) Modifier else Modifier.border(1.dp, c.rule, CircleShape))
            .pressable(shape = CircleShape, label = "Filter", onClick = onClick)
            .semantics { contentDescription = "Filter" }
            .testTag("filters"),
        contentAlignment = Alignment.Center,
    ) { GlyphIcon(Glyph.Filter, if (open) c.onInverse else c.ink, size = 20.dp) }
}

/**
 * The warn field: Reins is broken and needs you, the only colour on the screen when nothing waits. Each problem is
 * its label line, a few words in the serif, and the one thing to do.
 */
@Composable
private fun WarnField(warnings: List<Warning>, detail: String?) {
    val c = LocalColors.current
    FieldHeader(Field.Warn, Modifier.testTag("warnField")) {
        warnings.forEach { w ->
            Row(Modifier.fillMaxWidth().testTag(w.tag), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    LabelText(w.label, c.onField2)
                    BasicText(askString(w.ask, w.target), Modifier.padding(top = 6.dp), style = RType.serif(23f, 26f, -0.01f).copy(color = c.onField))
                    if (w.tag == "registration" && detail != null) RText(detail, RType.sans(13.5f), c.onField2, Modifier.padding(top = 4.dp), maxLines = 3)
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
private fun NoticeRow(text: String, onDismiss: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.padding(horizontal = 24.dp).padding(top = 14.dp).fillMaxWidth().hairlineBelow(c.rule).padding(bottom = 12.dp).testTag("notice"),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(Glyph.Info, c.violet, size = 18.dp)
        RText(text, RType.sans(14f), c.ink, Modifier.weight(1f), maxLines = 4)
        SignalButton("OK", Modifier.testTag("dismissNotice"), style = ButtonStyle.Soft, size = ButtonSize.Compact, onClick = onDismiss)
    }
}

/** Nothing waits: the pond and the mood line; with nothing connected, the two ways to connect something. */
@Composable
private fun Quiet(connections: Int, noHistory: Boolean, onConnectComputer: () -> Unit, onConnectAi: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(top = 10.dp, bottom = 4.dp).testTag(if (noHistory) "noActivity" else "allQuiet"),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Calm(quietMood(connections), moodStyle = MoodStyle, drawingWidth = 300.dp, gap = 4.dp)
        if (connections == 0) {
            Column(Modifier.padding(top = 18.dp), verticalArrangement = Arrangement.spacedBy(10.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                SignalButton("Connect a computer", Modifier.testTag("emptyConnectComputer"), size = ButtonSize.Small, glyph = Glyph.Laptop, onClick = onConnectComputer)
                SignalButton("Connect Claude.ai or ChatGPT", Modifier.testTag("emptyConnectAi"), style = ButtonStyle.Soft, size = ButtonSize.Small, glyph = Glyph.Link, onClick = onConnectAi)
            }
        }
    }
}

/** Where the entries you have not seen begin. */
@Composable
private fun NewMarker() {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 6.dp).testTag("newMarker"), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f).height(1.dp).background(c.violet.copy(alpha = 0.4f)))
        RText("NEW ABOVE", RType.label.copy(fontSize = 10.5.sp), c.violet, Modifier.padding(horizontal = 10.dp))
        Box(Modifier.weight(1f).height(1.dp).background(c.violet.copy(alpha = 0.4f)))
    }
}

/** How long the list must stay still before what it shows counts as seen. */
private const val SEEN_SETTLE_MS = 300L

/** A new entry this soon after an answer is where the fold landed. */
private const val LAND_WINDOW_MS = 5_000L

/** How long the landed row glows. */
private const val GLOW_MS = 4_000L
