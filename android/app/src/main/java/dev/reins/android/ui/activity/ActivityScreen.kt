package dev.reins.android.ui.activity

import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.KeyboardArrowUp
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.AppContainer
import dev.reins.android.design.ActionKind
import dev.reins.android.design.ActionTile
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.CountdownFrame
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.Hairline
import dev.reins.android.design.LargeTitle
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.ConnectorTags
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowState
import dev.reins.android.design.urgency
import dev.reins.android.platform.NotificationAccess
import dev.reins.android.platform.rememberNotificationState
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.ActivityRow
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.fullTitle
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.main.SettingsButton
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The first tab: what waits for you, then everything your AIs did, newest first. */
@Composable
fun ActivityScreen(
    container: AppContainer,
    notice: String?,
    onDismissNotice: () -> Unit,
    onOpenPending: (PendingItem) -> Unit,
    onOpenEntry: (Long) -> Unit,
    onIntegrations: () -> Unit,
    onSettings: () -> Unit = {},
    /** "Approve all" (true) or "Deny all" (false) for a burst of requests from one AI. */
    onAnswerBurst: (Burst, Boolean) -> Unit = { _, _ -> },
    /** With nothing connected yet: pair a computer, or get the address for Claude.ai / ChatGPT. */
    onConnectComputer: () -> Unit = {},
    onConnectAi: () -> Unit = {},
) {
    val c = LocalColors.current
    val state: AppState = container.state
    val entries by state.activity.collectAsStateWithLifecycle()
    val pending by state.pending.collectAsStateWithLifecycle()
    val seen by state.seenActivityId.collectAsStateWithLifecycle()
    val replaced by state.deviceReplaced.collectAsStateWithLifecycle()
    val registrationError by state.registrationError.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    // "Automatic": only what Autopilot, a bypass or Lockdown decided.
    var automaticOnly by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
    val automaticCount = remember(entries) { entries.count { it.decidedBy.isNotEmpty() } }
    val showFilters = automaticCount > 0 || automaticOnly
    val burstList = remember(pending) { bursts(pending) }
    val shown = remember(entries, automaticOnly) { if (automaticOnly) entries.filter { it.decidedBy.isNotEmpty() } else entries }
    // The "new above" marker sits over the oldest entry you have not seen.
    val newMarkerAt = remember(shown, seen) { shown.indexOfLast { it.id > seen } }
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val context = androidx.compose.ui.platform.LocalContext.current
    val notifications = rememberNotificationState()
    // People who never went through the setup (signed in before it existed) are asked once here.
    LaunchedEffect(Unit) {
        val ask = withContext(Dispatchers.Default) { !NotificationAccess.enabled(context) && !NotificationAccess.asked(context) }
        if (ask) notifications.request()
    }

    // Header rows above the entries: notices and the waiting section.
    val headerCount = 1 + (if (pending.isEmpty()) 0 else 1 + pending.size + burstList.size) + (if (notice != null) 1 else 0) +
        (if (notifications.enabled) 0 else 1) +
        (if (replaced) 1 else 0) + (if (registrationError != null) 1 else 0) + (if (showFilters) 1 else 0)

    // Opening the tab lands where you stopped reading: at the oldest entry you have not seen, unless something is waiting.
    var positioned by remember { mutableStateOf(false) }
    LaunchedEffect(entries.isNotEmpty()) {
        if (positioned || entries.isEmpty()) return@LaunchedEffect
        positioned = true
        val oldestUnseen = entries.indexOfLast { it.id > seen }
        if (oldestUnseen > 2 && pending.isEmpty()) {
            listState.scrollToItem((headerCount + oldestUnseen - 1).coerceAtLeast(0))
        }
    }
    // What is on screen counts as seen: scroll up to the newest and the badge clears. Recorded once the list comes to
    // rest, not on every frame of a fling.
    LaunchedEffect(shown, headerCount) {
        snapshotFlow { listState.layoutInfo.visibleItemsInfo.map { it.index } }.collectLatest { visible ->
            delay(SEEN_SETTLE_MS)
            val firstEntry = visible.filter { it >= headerCount }.minOrNull()?.minus(headerCount)
            val newest = when {
                firstEntry != null -> shown.getOrNull(firstEntry)?.id
                visible.any { it in 0 until headerCount } -> shown.firstOrNull()?.id.takeIf { listState.firstVisibleItemIndex == 0 }
                else -> null
            }
            if (newest != null) container.markActivitySeen(newest)
        }
    }
    val away by remember { derivedStateOf { listState.firstVisibleItemIndex > 3 } }

    Box(Modifier.fillMaxSize().background(c.background)) {
        LazyColumn(
            state = listState,
            modifier = Modifier.fillMaxSize().testTag("activityList"),
            contentPadding = PaddingValues(bottom = 130.dp),
        ) {
            item(key = "title") {
                LargeTitle("Activity") {
                    CapsuleButton("Integrations", Modifier.testTag("integrations"), compact = true, glyph = Glyph.Apps, onClick = onIntegrations)
                    SettingsButton(onSettings)
                }
            }
            if (!notifications.enabled) {
                item(key = "notifications") { NotificationsOffCard(Modifier.padding(horizontal = 16.dp, vertical = 6.dp), notifications::request) }
            }
            notice?.let {
                item(key = "notice") {
                    Column(Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) {
                        Banner(it)
                        CapsuleButton("Dismiss", Modifier.padding(top = 8.dp), style = ButtonStyle.Ghost, compact = true, onClick = onDismissNotice)
                    }
                }
            }
            if (replaced) {
                item(key = "replaced") {
                    Banner(
                        "Another phone is your approval device now. Use this phone again from Settings.",
                        Modifier.padding(16.dp),
                        BannerKind.Warning,
                    )
                }
            }
            registrationError?.let {
                item(key = "registration") {
                    Banner("This phone is not registered as your approval device yet: $it", Modifier.padding(16.dp), BannerKind.Error)
                }
            }
            if (pending.isNotEmpty()) {
                item(key = "waiting-label") { SectionTitle("Waiting for you") }
                burstList.forEach { burst ->
                    item(key = "burst:${burst.connectionId}") {
                        BurstBar(burst, Modifier.padding(horizontal = 16.dp, vertical = 5.dp), onAnswerBurst)
                    }
                }
                itemsIndexed(pending, key = { _, item -> "p:${item.id}" }, contentType = { _, _ -> "pending" }) { _, item ->
                    PendingCard(item, Modifier.padding(horizontal = 16.dp, vertical = 5.dp)) { onOpenPending(item) }
                }
            }
            if (showFilters) {
                item(key = "filters") {
                    Row(Modifier.padding(start = 16.dp, end = 16.dp, top = 14.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        dev.reins.android.design.SelectChip("All", !automaticOnly, Modifier.testTag("filter:all")) { automaticOnly = false }
                        dev.reins.android.design.SelectChip("Automatic · $automaticCount", automaticOnly, Modifier.testTag("filter:automatic")) { automaticOnly = true }
                    }
                }
            }
            if (shown.isEmpty()) {
                item(key = "empty") {
                    if (automaticOnly) {
                        EmptyState(
                            Glyph.Sparkle,
                            "Nothing automatic yet",
                            "What Autopilot, a bypass or Lockdown decides for you shows up here.",
                            tag = "noAutomatic",
                        )
                    } else if (connections.isEmpty()) {
                        // A new account: nothing can ask yet, so say how to connect something.
                        EmptyState(
                            Glyph.List,
                            "Nothing yet",
                            "Connect your computer or an AI app. What they ask for shows up here, and waits for you.",
                            tag = "noActivity",
                        ) {
                            CapsuleButton("Connect a computer", Modifier.testTag("emptyConnectComputer"), glyph = Glyph.Laptop, onClick = onConnectComputer)
                            CapsuleButton(
                                "Connect Claude.ai or ChatGPT",
                                Modifier.testTag("emptyConnectAi"),
                                style = ButtonStyle.Secondary,
                                glyph = Glyph.Link,
                                onClick = onConnectAi,
                            )
                        }
                    } else {
                        EmptyState(
                            Glyph.List,
                            "Nothing yet",
                            "When an AI searches, reads or sends on your behalf, it shows up here.",
                            tag = "noActivity",
                        )
                    }
                }
            } else {
                itemsIndexed(shown, key = { _, e -> "e:${e.id}" }, contentType = { _, _ -> "entry" }) { index, entry ->
                    if (index == 0) SectionTitle(if (automaticOnly) "Decided for you" else "Latest")
                    if (index == newMarkerAt) NewMarker()
                    Box(Modifier.testTag("entry:${entry.id}")) {
                        ActivityRow(entry) { onOpenEntry(entry.id) }
                    }
                    if (index < shown.lastIndex) Hairline(inset = 74.dp)
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

/** Approvals cannot reach a phone that shows no notifications: said first, until they are on. */
@Composable
private fun NotificationsOffCard(modifier: Modifier, onTurnOn: () -> Unit) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .background(c.warning.copy(alpha = 0.12f), RoundedCornerShape(18.dp))
            .padding(14.dp)
            .testTag("notificationsOff"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).background(c.warning.copy(alpha = 0.18f), CircleShape), contentAlignment = Alignment.Center) {
            dev.reins.android.design.GlyphIcon(Glyph.Bell, c.warning, size = 20.dp)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            RText("Notifications are off", RType.sans(15.5f, FontWeight.SemiBold), c.text)
            RText(
                "Requests from your AIs can't reach you while Reins is closed, so they wait and time out.",
                RType.sans(13f, lineHeight = 18f),
                c.secondary,
                Modifier.padding(top = 2.dp),
            )
        }
        Spacer(Modifier.width(10.dp))
        CapsuleButton("Turn on", Modifier.testTag("turnOnNotifications"), style = ButtonStyle.Accent, compact = true, onClick = onTurnOn)
    }
}

@Composable
private fun SectionTitle(text: String) {
    RText(
        text.uppercase(),
        RType.sans(12.5f, FontWeight.Medium).copy(letterSpacing = 0.6.sp),
        LocalColors.current.secondary,
        Modifier.padding(start = 20.dp, top = 18.dp, bottom = 6.dp),
    )
}

/** Where the entries you have not seen begin. */
@Composable
private fun NewMarker() {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp).testTag("newMarker"), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f).height(1.dp).background(c.accent.copy(alpha = 0.4f)))
        RText("NEW ABOVE", RType.sans(11.5f, FontWeight.SemiBold).copy(letterSpacing = 0.8.sp), c.accent, Modifier.padding(horizontal = 10.dp))
        Box(Modifier.weight(1f).height(1.dp).background(c.accent.copy(alpha = 0.4f)))
    }
}

/**
 * Several requests from one AI at once: approve its routine ones together (one screen lock or biometric check), or deny
 * everything it asked. What is asked every time stays in the list below and is opened one by one.
 */
@Composable
private fun BurstBar(burst: Burst, modifier: Modifier, onAnswer: (Burst, Boolean) -> Unit) {
    val c = LocalColors.current
    val label = untrusted(burst.label)
    val held = burst.all.size - burst.quick.size
    Column(
        modifier
            .fillMaxWidth()
            .background(c.accent.copy(alpha = 0.08f), RoundedCornerShape(18.dp))
            .padding(14.dp)
            .testTag("burst:${burst.connectionId}"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            ConnectionIcon(burst.connectionId, burst.label, size = 22.dp)
            Spacer(Modifier.width(8.dp))
            RText("$label asked ${burst.all.size} times", RType.sans(15.5f, FontWeight.SemiBold), c.text, Modifier.weight(1f), maxLines = 1)
        }
        if (held > 0) {
            RText(
                if (held == 1) "1 of them needs a closer look and stays in the list." else "$held of them need a closer look and stay in the list.",
                RType.sans(13f, lineHeight = 18f),
                c.secondary,
                Modifier.padding(top = 4.dp),
            )
        }
        Row(Modifier.fillMaxWidth().padding(top = 10.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            CapsuleButton("Deny all", Modifier.weight(1f).testTag("denyAll"), style = ButtonStyle.Secondary, compact = true) { onAnswer(burst, false) }
            CapsuleButton("Approve ${burst.quick.size}", Modifier.weight(1f).testTag("approveAll"), style = ButtonStyle.Accent, compact = true) { onAnswer(burst, true) }
        }
    }
}

/** A request or connection that waits for you; its border shows how long the AI keeps waiting. */
@Composable
private fun PendingCard(item: PendingItem, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val action = ActionKind.of(item.action)
    // An upload waits until the server deletes it (an hour or so), not for an AI that is holding on: no countdown.
    val waitUntil = if (item.kind == PendingKind.BLOB) null else item.waitUntil
    // One clock for the border and the seconds left; it stops once the AI has stopped waiting.
    val now = rememberNowState(untilMillis = waitUntil?.let { it * 1000 })
    val pairing = item.kind == PendingKind.PAIRING
    val join = item.kind == PendingKind.JOIN
    // Neither comes from an AI connection: no connection icon, no integration tags.
    val fromAi = !pairing && !join
    CountdownFrame(item.createdAt, waitUntil, modifier.testTag("pending:${item.id}"), now = now) {
        Row(
            Modifier
                .fillMaxWidth()
                .background(c.elevated, RoundedCornerShape(18.dp))
                .pressable(highlight = c.controlFill, shape = RoundedCornerShape(18.dp), onClick = onClick)
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ActionTile(action, item.count.toInt(), size = 46.dp)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (fromAi) {
                        ConnectionIcon(item.connectionId, item.connectionLabel, size = 20.dp)
                        Spacer(Modifier.width(8.dp))
                    }
                    RText(
                        when {
                            pairing -> "${untrusted(item.connectionLabel)}: wants to connect"
                            join -> "Add ${untrusted(item.connectionLabel)}?"
                            else -> fullTitle(item.connectionLabel, item.action, item.count.toInt(), item.service, item.opTitle, item.op)
                        },
                        RType.sans(16f, FontWeight.SemiBold),
                        c.text,
                        Modifier.weight(1f),
                        maxLines = 2,
                    )
                }
                if (item.kind == PendingKind.BLOB && item.subtitle.isNotBlank()) {
                    RText(untrusted(item.subtitle), RType.sans(13.5f), c.secondary, maxLines = 2)
                }
                if (join) RText("Another phone signed in to your account", RType.sans(13.5f), c.secondary, maxLines = 2)
                if (fromAi) ConnectorTags(item.service, item.account)
                item.suggestion?.let { line ->
                    Row(Modifier.testTag("pendingSuggestion:${item.id}"), verticalAlignment = Alignment.CenterVertically) {
                        dev.reins.android.design.GlyphIcon(Glyph.Sparkle, c.accent, size = 13.dp, weight = 1.9f)
                        Spacer(Modifier.width(5.dp))
                        RText(line, RType.sans(12.5f, FontWeight.Medium), c.accent, maxLines = 1)
                    }
                }
                WaitLine(item.createdAt, waitUntil, now)
            }
            Spacer(Modifier.width(8.dp))
            CapsuleButton("Review", style = ButtonStyle.Accent, compact = true, onClick = onClick)
        }
    }
}

/** How long the AI keeps waiting, in its own scope: the clock's ticks recompose this line and nothing else on the card. */
@Composable
private fun WaitLine(createdAt: Long, waitUntil: Long?, now: State<Long>) {
    val c = LocalColors.current
    // The clock ticks four times a second; the line changes once a second.
    val line by remember(createdAt, waitUntil, now, c) {
        derivedStateOf {
            val u = urgency(createdAt, waitUntil, now.value)
            when {
                u == null -> relativeTime(createdAt) to c.tertiary
                u.stale -> "Stopped waiting · you can still approve" to c.warning
                u.urgent -> "${u.remainingSeconds} s left" to c.danger
                else -> "${u.remainingSeconds} s · waiting for you" to c.secondary
            }
        }
    }
    RText(line.first, RType.sans(13f, FontWeight.Medium), line.second)
}

/** How long the list must stay still before what it shows counts as seen. */
private const val SEEN_SETTLE_MS = 300L
