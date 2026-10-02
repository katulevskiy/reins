package dev.rewarden.android.ui.activity

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
import dev.rewarden.android.AppContainer
import dev.rewarden.android.design.ActionKind
import dev.rewarden.android.design.ActionTile
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.CountdownFrame
import dev.rewarden.android.design.EmptyState
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.Hairline
import dev.rewarden.android.design.LargeTitle
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.ConnectorTags
import dev.rewarden.android.design.glass
import dev.rewarden.android.design.pressable
import dev.rewarden.android.design.rememberNowMillis
import dev.rewarden.android.design.urgency
import dev.rewarden.android.state.AppState
import dev.rewarden.android.ui.common.ActivityRow
import dev.rewarden.android.ui.common.ConnectionIcon
import dev.rewarden.android.ui.common.fullTitle
import dev.rewarden.android.ui.common.relativeTime
import dev.rewarden.android.ui.common.untrusted
import dev.rewarden.core.PendingItem
import dev.rewarden.core.PendingKind
import kotlinx.coroutines.launch

/** The first tab: what waits for you, then everything your AIs did, newest first. */
@Composable
fun ActivityScreen(
    container: AppContainer,
    notice: String?,
    onDismissNotice: () -> Unit,
    onOpenPending: (PendingItem) -> Unit,
    onOpenEntry: (Long) -> Unit,
    onIntegrations: () -> Unit,
    onAutopilot: () -> Unit = {},
) {
    val c = LocalColors.current
    val state: AppState = container.state
    val entries by state.activity.collectAsStateWithLifecycle()
    val pending by state.pending.collectAsStateWithLifecycle()
    val seen by state.seenActivityId.collectAsStateWithLifecycle()
    val replaced by state.deviceReplaced.collectAsStateWithLifecycle()
    val registrationError by state.registrationError.collectAsStateWithLifecycle()
    val autopilot by state.autopilot.collectAsStateWithLifecycle()
    // "Automatic": only what Autopilot, a bypass or Lockdown decided.
    var automaticOnly by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
    val automaticCount = entries.count { it.decidedBy.isNotEmpty() }
    val showFilters = automaticCount > 0 || automaticOnly
    val shown = if (automaticOnly) entries.filter { it.decidedBy.isNotEmpty() } else entries
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()

    // Header rows above the entries: notices and the waiting section.
    val headerCount = 1 + (if (pending.isEmpty()) 0 else 1 + pending.size) + (if (notice != null) 1 else 0) +
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
    // What is on screen counts as seen: scroll up to the newest and the badge clears.
    LaunchedEffect(shown, headerCount) {
        snapshotFlow { listState.layoutInfo.visibleItemsInfo.map { it.index } }.collect { visible ->
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
                    if (autopilot != null) dev.rewarden.android.ui.autopilot.ModePill(autopilot, Modifier.testTag("modePill"), onClick = onAutopilot)
                    CapsuleButton("Integrations", Modifier.testTag("integrations"), compact = true, glyph = Glyph.Apps, onClick = onIntegrations)
                }
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
                itemsIndexed(pending, key = { _, item -> "p:${item.id}" }) { _, item ->
                    PendingCard(item, Modifier.padding(horizontal = 16.dp, vertical = 5.dp)) { onOpenPending(item) }
                }
            }
            if (showFilters) {
                item(key = "filters") {
                    Row(Modifier.padding(start = 16.dp, end = 16.dp, top = 14.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        dev.rewarden.android.design.SelectChip("All", !automaticOnly, Modifier.testTag("filter:all")) { automaticOnly = false }
                        dev.rewarden.android.design.SelectChip("Automatic · $automaticCount", automaticOnly, Modifier.testTag("filter:automatic")) { automaticOnly = true }
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
                itemsIndexed(shown, key = { _, e -> "e:${e.id}" }) { index, entry ->
                    if (index == 0) SectionTitle(if (automaticOnly) "Decided for you" else "Latest")
                    if (entry.id > seen && index == shown.indexOfLast { it.id > seen }) NewMarker()
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

/** A request or connection that waits for you; its border shows how long the AI keeps waiting. */
@Composable
private fun PendingCard(item: PendingItem, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val action = ActionKind.of(item.action)
    val now = rememberNowMillis()
    // An upload waits until the server deletes it (an hour or so), not for an AI that is holding on: no countdown.
    val waitUntil = if (item.kind == PendingKind.BLOB) null else item.waitUntil
    val u = urgency(item.createdAt, waitUntil, now)
    val pairing = item.kind == PendingKind.PAIRING
    val join = item.kind == PendingKind.JOIN
    // Neither comes from an AI connection: no connection icon, no integration tags.
    val fromAi = !pairing && !join
    CountdownFrame(item.createdAt, waitUntil, modifier.testTag("pending:${item.id}")) {
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
                        dev.rewarden.android.design.GlyphIcon(Glyph.Sparkle, c.accent, size = 13.dp, weight = 1.9f)
                        Spacer(Modifier.width(5.dp))
                        RText(line, RType.sans(12.5f, FontWeight.Medium), c.accent, maxLines = 1)
                    }
                }
                RText(
                    when {
                        u == null -> relativeTime(item.createdAt)
                        u.stale -> "Stopped waiting · you can still approve"
                        u.urgent -> "${u.remainingSeconds} s left"
                        else -> "${u.remainingSeconds} s · waiting for you"
                    },
                    RType.sans(13f, FontWeight.Medium),
                    when {
                        u == null -> c.tertiary
                        u.stale -> c.warning
                        u.urgent -> c.danger
                        else -> c.secondary
                    },
                )
            }
            Spacer(Modifier.width(8.dp))
            CapsuleButton("Review", style = ButtonStyle.Accent, compact = true, onClick = onClick)
        }
    }
}
