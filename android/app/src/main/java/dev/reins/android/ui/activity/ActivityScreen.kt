package dev.reins.android.ui.activity

import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.NotificationsOff
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.Sync
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Icon
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.AppContainer
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.LargeTitle
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Timers
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.platform.NotificationAccess
import dev.reins.android.platform.rememberNotificationState
import dev.reins.android.platform.rememberScreenLockState
import dev.reins.android.state.AppState
import dev.reins.android.ui.main.HeaderControlHeight
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.primaryFill
import dev.reins.core.PendingItem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** One row of the history as the list lays it out: a day's header, or an entry with its place in the day's card. */
private sealed interface HistoryItem {
    data class Day(val label: String, val first: Boolean) : HistoryItem

    data class Entry(val entry: dev.reins.core.ActivityEntry, val first: Boolean, val last: Boolean, val index: Int) : HistoryItem
}

/** The first tab: what waits for you, then everything your AIs did, by day, newest first. */
@Composable
fun ActivityScreen(
    container: AppContainer,
    notice: String?,
    onDismissNotice: () -> Unit,
    onOpenPending: (PendingItem) -> Unit,
    onOpenEntry: (Long) -> Unit,
    /** The round + beside the title: the integrations. */
    onIntegrations: () -> Unit,
    /** The mode pill: the quick switcher (or the Autopilot tab). */
    onAutopilot: () -> Unit = {},
    /** Autopilot's one-tap pick: Allow (true, after the owner check) or Deny (false) for one request or a group. */
    onAnswerBurst: (Burst, Boolean) -> Unit = { _, _ -> },
    /** With nothing connected yet: pair a computer, or get the address for Claude.ai / ChatGPT. */
    onConnectComputer: () -> Unit = {},
    onConnectAi: () -> Unit = {},
    /** "Another phone approves": where to switch back. */
    onOpenSettings: () -> Unit = {},
) {
    val c = LocalColors.current
    val state: AppState = container.state
    val entries by state.activity.collectAsStateWithLifecycle()
    val pending by state.pending.collectAsStateWithLifecycle()
    val loaded by state.loaded.collectAsStateWithLifecycle()
    val seen by state.seenActivityId.collectAsStateWithLifecycle()
    val replaced by state.deviceReplaced.collectAsStateWithLifecycle()
    val registrationError by state.registrationError.collectAsStateWithLifecycle()
    val syncNotice by state.syncNotice.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    val apSettings by state.autopilot.collectAsStateWithLifecycle()
    var filter by rememberSaveable { mutableStateOf(HistoryFilter.All) }
    var filtersOpen by rememberSaveable { mutableStateOf(false) }
    val details = rememberPendingDetails(container, pending)
    val cards = remember(pending) { waitCards(pending) }
    val shown = remember(entries, filter) { entries.filter(filter::matches) }
    val nowSeconds = (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000
    // The history, by day; the "new above" marker sits over the oldest entry you have not seen.
    val history = remember(shown, nowSeconds / 3_600) {
        buildList {
            var day: Long? = null
            shown.forEachIndexed { i, e ->
                val d = dayOf(e.at)
                if (d != day) {
                    add(HistoryItem.Day(dayLabel(e.at, nowSeconds), first = day == null))
                    day = d
                }
                val next = shown.getOrNull(i + 1)
                add(HistoryItem.Entry(e, first = (lastOrNull() is HistoryItem.Day), last = next == null || dayOf(next.at) != d, index = i))
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

    // The rows above the history: the title, the banners, the waiting section and the filters.
    val headerCount = 1 + (if (notifications.enabled) 0 else 1) + (if (screenLock.set) 0 else 1) + (if (notice != null) 1 else 0) +
        (if (replaced) 1 else 0) + (if (registrationError != null) 1 else 0) + (if (syncNotice != null) 1 else 0) +
        (if (pending.isEmpty()) 0 else 1 + cards.size) +
        (if (filtersOpen) 1 else 0)

    // Opening the tab lands where you stopped reading: at the oldest entry you have not seen, unless something is waiting.
    var positioned by remember { mutableStateOf(false) }
    LaunchedEffect(entries.isNotEmpty()) {
        if (positioned || entries.isEmpty()) return@LaunchedEffect
        positioned = true
        val oldestUnseen = shown.indexOfLast { it.id > seen }
        if (oldestUnseen > 2 && pending.isEmpty()) {
            val at = history.indexOfFirst { it is HistoryItem.Entry && it.index == oldestUnseen }
            if (at >= 0) listState.scrollToItem((headerCount + at - 1).coerceAtLeast(0))
        }
    }
    // What is on screen counts as seen: scroll up to the newest and the badge clears. Recorded once the list comes to
    // rest, not on every frame of a fling.
    LaunchedEffect(shown) {
        snapshotFlow { listState.layoutInfo.visibleItemsInfo.map { it.key } }.collectLatest { keys ->
            delay(SEEN_SETTLE_MS)
            val visible = keys.mapNotNull { (it as? String)?.takeIf { k -> k.startsWith("e:") }?.removePrefix("e:")?.toLongOrNull() }
            // Only the rows above the history on screen, at the very top: the newest counts as seen.
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
        modifier = Modifier.fillMaxSize().background(c.background),
    ) {
        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize().testTag("activityList"),
            contentPadding = PaddingValues(bottom = 130.dp),
        ) {
            item(key = "title") {
                LargeTitle("Activity") {
                    RoundPlus("integrations", "Integrations", onIntegrations)
                    dev.reins.android.ui.main.ModePill(apSettings, onAutopilot)
                }
            }
            if (!notifications.enabled) {
                item(key = "notifications") {
                    CompactBanner(Icons.Rounded.NotificationsOff, "Notifications are off", c.warning, "Turn on", "notificationsOff", "turnOnNotifications", notifications::request)
                }
            }
            if (!screenLock.set) {
                item(key = "screenLock") {
                    CompactBanner(Icons.Rounded.Lock, "No screen lock", c.warning, "Set one", "screenLockOff", "setScreenLock", screenLock::openSettings)
                }
            }
            notice?.let {
                item(key = "notice") {
                    CompactBanner(null, it, c.accent, "OK", "notice", "dismissNotice", onDismissNotice, maxLines = 4)
                }
            }
            if (replaced) {
                item(key = "replaced") {
                    CompactBanner(Icons.Rounded.PhoneAndroid, "Another phone approves", c.warning, "Switch", "replaced", "switchBack", onOpenSettings)
                }
            }
            syncNotice?.let {
                item(key = "syncNotice") {
                    CompactBanner(Icons.Rounded.Sync, it, c.warning, "OK", "syncNotice", "dismissSyncNotice", { scope.launch { container.dismissSyncNotice() } }, maxLines = 3)
                }
            }
            registrationError?.let {
                item(key = "registration") {
                    CompactBanner(Icons.Rounded.Warning, "Not your approval phone yet: $it", c.danger, null, "registration", null, {}, maxLines = 3)
                }
            }
            if (pending.isNotEmpty()) {
                item(key = "waiting-label") { SectionHeader("Waiting · ${pending.size}", Modifier.testTag("waitingHeader")) }
                cards.forEach { card ->
                    when (card) {
                        is WaitCard.One -> item(key = "p:${card.item.id}", contentType = "pending") {
                            val item = card.item
                            val single = Burst(item.connectionId, item.connectionLabel, listOf(item.id), listOf(item.id))
                            WaitingCard(
                                item,
                                details.suggestions[item.id],
                                details.views[item.id],
                                Modifier.padding(start = 12.dp, end = 12.dp, bottom = 10.dp),
                                onReview = { onOpenPending(item) },
                                onAllow = { onAnswerBurst(single, true) },
                                onDeny = { onAnswerBurst(single, false) },
                            )
                        }
                        is WaitCard.Group -> item(key = "g:${card.burst.connectionId}", contentType = "group") {
                            GroupCard(
                                card,
                                details,
                                Modifier.padding(start = 12.dp, end = 12.dp, bottom = 10.dp),
                                onReview = { onOpenPending(card.items.first()) },
                                onAllow = { onAnswerBurst(card.burst, true) },
                                onDeny = { onAnswerBurst(card.burst, false) },
                            )
                        }
                    }
                }
            }
            if (filtersOpen) {
                item(key = "filters") { FilterChips(filter, Modifier.padding(top = 10.dp)) { filter = it } }
            }
            if (shown.isEmpty() && loaded) {
                item(key = "empty") {
                    Column {
                        if (entries.isNotEmpty()) DayHeader("History", filterOpen = filtersOpen, onFilter = { filtersOpen = !filtersOpen })
                        when {
                            filter != HistoryFilter.All -> EmptyState(Glyph.List, "Nothing here", null, tag = if (filter == HistoryFilter.Autopilot) "noAutomatic" else "noFiltered")
                            connections.isEmpty() -> EmptyState(Glyph.List, "Nothing yet", null, tag = "noActivity") {
                                // A new account: nothing can ask yet, so say how to connect something.
                                CapsuleButton("Connect a computer", Modifier.testTag("emptyConnectComputer"), glyph = Glyph.Laptop, onClick = onConnectComputer)
                                CapsuleButton("Connect Claude.ai or ChatGPT", Modifier.testTag("emptyConnectAi"), style = ButtonStyle.Secondary, glyph = Glyph.Link, onClick = onConnectAi)
                            }
                            else -> EmptyState(Glyph.List, "Nothing yet", null, tag = "noActivity")
                        }
                    }
                }
            } else {
                history.forEach { row ->
                    when (row) {
                        is HistoryItem.Day -> item(key = "d:${row.label}", contentType = "day") {
                            DayHeader(row.label, filterOpen = if (row.first) filtersOpen else null, onFilter = { filtersOpen = !filtersOpen })
                        }
                        is HistoryItem.Entry -> item(key = "e:${row.entry.id}", contentType = "entry") {
                            Column {
                                if (row.index == newMarkerAt) NewMarker()
                                Box(Modifier.testTag("entry:${row.entry.id}")) {
                                    HistoryRow(row.entry, nowSeconds, row.first, row.last) { onOpenEntry(row.entry.id) }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Back to the newest.
        if (away) {
            Box(
                Modifier
                    .align(Alignment.BottomCenter)
                    .padding(bottom = 108.dp)
                    .glass(c, CircleShape, 8.dp)
                    .pressable(shape = CircleShape, label = "Latest") { scope.launch { listState.animateScrollToItem(0) } }
                    .padding(horizontal = 16.dp, vertical = 10.dp)
                    .testTag("scrollToLatest"),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Rounded.KeyboardArrowUp, null, Modifier.height(20.dp), tint = c.accent)
                    Spacer(Modifier.width(4.dp))
                    RText("Latest", RType.sans(14f, FontWeight.SemiBold), c.accent)
                }
            }
        }
    }
}

/** The white round + beside a tab's title. */
@Composable
fun RoundPlus(tag: String, label: String, onClick: () -> Unit) {
    val c = LocalColors.current
    val (bg, fg) = primaryFill(c)
    Box(
        Modifier
            .size(HeaderControlHeight)
            .clip(CircleShape)
            .background(bg)
            .pressable(shape = CircleShape, label = label, onClick = onClick)
            .semantics { contentDescription = label }
            .testTag(tag),
        contentAlignment = Alignment.Center,
    ) { Icon(Icons.Rounded.Add, null, Modifier.size(22.dp), tint = fg) }
}

/** A section's header in the accent colour ("Waiting · 2", "Active · 3"). */
@Composable
fun SectionHeader(text: String, modifier: Modifier = Modifier) {
    RText(
        text,
        RType.sans(14f, FontWeight.Medium),
        LocalColors.current.accent,
        modifier.padding(start = 22.dp, top = 14.dp, bottom = 8.dp),
        maxLines = 1,
    )
}

/** A notice in one line, with the one thing to do about it. */
@Composable
private fun CompactBanner(
    icon: ImageVector?,
    text: String,
    tone: Color,
    action: String?,
    tag: String,
    actionTag: String?,
    onAction: () -> Unit,
    maxLines: Int = 1,
) {
    val c = LocalColors.current
    Row(
        Modifier
            .padding(horizontal = 12.dp, vertical = 4.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(20.dp))
            .background(tone.copy(alpha = 0.12f))
            .padding(start = 14.dp, end = 6.dp, top = 6.dp, bottom = 6.dp)
            .testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, Modifier.size(18.dp), tint = tone)
            Spacer(Modifier.width(10.dp))
        }
        RText(text, RType.sans(14f, FontWeight.Medium), c.text, Modifier.weight(1f).padding(vertical = 8.dp), maxLines = maxLines)
        if (action != null) {
            Spacer(Modifier.width(8.dp))
            PillButton(action, if (actionTag != null) Modifier.testTag(actionTag) else Modifier, background = tone.copy(alpha = 0.2f), foreground = if (tone == c.accent) c.accent else c.text, height = 34.dp, textSize = 13.5f, onClick = onAction)
        }
    }
}

/** Where the entries you have not seen begin. */
@Composable
private fun NewMarker() {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 6.dp).testTag("newMarker"), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f).height(1.dp).background(c.accent.copy(alpha = 0.4f)))
        RText("NEW ABOVE", RType.sans(11.5f, FontWeight.SemiBold).copy(letterSpacing = 0.8.sp), c.accent, Modifier.padding(horizontal = 10.dp))
        Box(Modifier.weight(1f).height(1.dp).background(c.accent.copy(alpha = 0.4f)))
    }
}

/** How long the list must stay still before what it shows counts as seen. */
private const val SEEN_SETTLE_MS = 300L
