package dev.reins.android.ui.grants

import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.design.MiniLogo
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.draw.drawBehind
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.BrandTile
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalButton
import dev.reins.android.design.SignalDialog
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowState
import dev.reins.android.design.grantClock
import dev.reins.android.state.AppState
import dev.reins.android.ui.autopilot.LabelLink
import dev.reins.android.ui.autopilot.TrustLabel
import dev.reins.android.ui.common.endedLine
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.main.HeaderInsets
import dev.reins.core.GrantView
import dev.reins.core.StandingGrant
import kotlinx.coroutines.launch

// The grants' part of Trust (Reins Signal, iOS-Trust 01 and 05): the grants that run, as hairline rows with their time
// left, and the "Ended" page to resume or delete the rest. What the Grants tab did, moved; its lambdas unchanged.

/**
 * The grants that run, soonest end first (Next, Trust 01): a section header with how many and a + for a new one, then
 * a group of rows and, last, "Ended" with how many, which opens the ones that ended.
 */
@Composable
fun GrantsSection(state: AppState, onOpen: (String) -> Unit, onEnded: () -> Unit, onNew: () -> Unit = {}) {
    val c = LocalColors.current
    val grants by state.grants.collectAsStateWithLifecycle()
    val running = remember(grants) { grants.filter { it.active }.sortedBy { it.expiresAt ?: Long.MAX_VALUE } }
    val ended = remember(grants) { grants.count { !it.active } }
    dev.reins.android.design.SectionHeader(
        "Grants",
        glyph = Glyph.Key,
        count = running.size.toString(),
        onAdd = onNew,
        addLabel = "New grant",
        addTag = "newGrant",
    )
    dev.reins.android.design.PaneGroup {
        if (running.isEmpty()) {
            RText(
                if (grants.isEmpty()) "No grants yet." else "None running.",
                RType.sans(15f),
                c.inkMeta,
                Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 16.dp).testTag("noGrants"),
            )
        }
        running.forEachIndexed { i, grant -> GrantRow(grant, Modifier.testTag("grant:${grant.id}"), first = i == 0) { onOpen(grant.id) } }
        if (ended > 0) {
            dev.reins.android.design.PaneRow(
                "Ended",
                leading = { dev.reins.android.design.RowTile(Glyph.TimerOff) },
                value = ended.toString(),
                chevron = true,
                first = running.isEmpty() && false,
                tag = "expiredHeader",
                onClick = onEnded,
            )
        }
    }
}

/**
 * A running grant (Next, Trust 01): the service's tile with the AI's mark on its corner, what it allows, who and how
 * much it is used (uses as bars when capped), and its time left as a ring; the ring turns red when it ends soon.
 */
@Composable
fun GrantRow(grant: GrantView, modifier: Modifier = Modifier, first: Boolean = true, onClick: () -> Unit) {
    val c = LocalColors.current
    // A grant without an end has nothing to count down; the clock lives in this row only.
    val now = rememberNowState(1_000, untilMillis = grant.expiresAt?.let { it * 1000 })
    val clock = remember(grant, now) { derivedStateOf { grantClock(grant, now.value / 1000) } }
    val soon by remember(grant, now) { derivedStateOf { clock.value.soon } }
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = 60.dp)
            .then(if (!first) Modifier.drawBehind { drawLine(c.rule, androidx.compose.ui.geometry.Offset(62.dp.toPx(), 0f), androidx.compose.ui.geometry.Offset(size.width, 0f), 1.dp.toPx()) } else Modifier)
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .semantics(mergeDescendants = true) {}
            .padding(horizontal = 14.dp, vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MiniLogo(
            grant.service.ifBlank { "gmail" },
            face = { size -> ConnectionIcon(grant.connectionId, grant.connectionLabel, size = size) },
            ring = c.paperRaised,
        )
        Column(Modifier.weight(1f)) {
            RText(untrusted(grant.summary), RType.row, c.ink, maxLines = 2)
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (soon) {
                    RText("${untrusted(grant.connectionLabel)} · ", RType.meta, c.inkMeta, maxLines = 1)
                    RText("ends soon", RType.meta, c.danger, Modifier.testTag("endsSoon"), maxLines = 1)
                } else {
                    RText(grantUseLine(grant), RType.meta, c.inkMeta, Modifier.testTag("uses"), maxLines = 1)
                }
            }
            val max = grant.maxUses?.toInt()
            if (max != null && max > 1) {
                Row(Modifier.padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    UseDots(grant.uses.toInt(), max)
                    RText("${grant.uses}/$max", RType.sans(12.5f, FontWeight.Medium), c.inkMeta, maxLines = 1)
                }
            }
        }
        GrantPie(clock, size = 36.dp)
    }
}

/**
 * Trust › Ended: the grants that ran out, dimmed. Resume opens the same sheet as Extend with the old scope; Delete
 * asks once. What the "Expired" sheet of the Grants tab did, as a page.
 */
@Composable
fun EndedGrantsScreen(
    state: AppState,
    onBack: () -> Unit,
    onOpen: (String) -> Unit,
    onResume: suspend (grantId: String, seconds: Long, standing: StandingGrant?) -> String?,
    onDelete: suspend (grantId: String) -> String?,
) {
    val c = LocalColors.current
    val grants by state.grants.collectAsStateWithLifecycle()
    val ended = remember(grants) { grants.filterNot { it.active }.sortedByDescending { it.createdAt } }
    var resuming by remember { mutableStateOf<GrantView?>(null) }
    var resumingMore by remember { mutableStateOf<GrantView?>(null) }
    var deleting by remember { mutableStateOf<GrantView?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    Box(Modifier.fillMaxSize().background(c.paper).testTag("expiredList")) {
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = 24.dp, end = 24.dp, bottom = 32.dp),
        ) {
            item(key = "back") {
                Box(Modifier.windowInsetsPadding(HeaderInsets).padding(top = 8.dp, start = 0.dp)) {
                    RoundIconButton(Glyph.ChevronLeft, "Back", Modifier.testTag("back"), onClick = onBack)
                }
            }
            item(key = "title") {
                Column(Modifier.padding(top = 14.dp, bottom = 16.dp)) {
                    RText("Ended", RType.title, c.ink, maxLines = 1)
                    RText("Resume any of these as it was, or let them go.", RType.body, c.ink2, Modifier.padding(top = 8.dp))
                }
            }
            error?.let { item(key = "error") { Banner(it, Modifier.padding(bottom = 8.dp), BannerKind.Error, tag = "resumeError") } }
            items(ended, key = { "e:${it.id}" }) { grant ->
                EndedRow(grant, Modifier.testTag("ended:${grant.id}"), onOpen = { onOpen(grant.id) }, onResume = { resuming = grant }, onDelete = { deleting = grant })
            }
        }
        // Resume opens the same sheet as Extend, with the old scope; More options holds the rest.
        LengthSheet(
            resuming,
            resume = true,
            onClose = { resuming = null },
            onConfirm = { seconds ->
                val grant = resuming ?: return@LengthSheet
                resuming = null
                error = null
                scope.launch {
                    error = resumeAsItWas(grant, seconds, onResume)
                    if (error == null && state.grants.value.any { it.id == grant.id && it.active }) onBack()
                }
            },
            onMore = {
                resumingMore = resuming
                resuming = null
            },
        )
    }

    resumingMore?.let { grant ->
        ResumeDialog(
            grant = grant,
            onDismiss = { resumingMore = null },
            onResume = { seconds, standing ->
                resumingMore = null
                error = null
                scope.launch {
                    error = onResume(grant.id, seconds, standing)
                    // Running again: back to Trust, where it now is.
                    if (error == null && state.grants.value.any { it.id == grant.id && it.active }) onBack()
                }
            },
        )
    }
    deleting?.let { grant ->
        SignalDialog(
            title = "Delete this grant for good?",
            text = untrusted(grant.summary) + ". It can't be resumed.",
            confirmLabel = "Delete",
            destructive = true,
            italic = "for good?",
            onConfirm = {
                deleting = null
                error = null
                scope.launch { error = onDelete(grant.id) }
            },
            onDismiss = { deleting = null },
        )
    }
}

/** An ended grant: its service at 55 %, what it allowed, who and how it ended, then Resume and Delete. */
@Composable
private fun EndedRow(grant: GrantView, modifier: Modifier, onOpen: () -> Unit, onResume: () -> Unit, onDelete: () -> Unit) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().hairlineBelow(c.rule).padding(bottom = 9.dp)) {
        // The grant itself opens its page; Resume and Delete sit under its words.
        Row(
            modifier
                .fillMaxWidth()
                .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onOpen)
                .semantics(mergeDescendants = true) {}
                .padding(top = 9.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            BrandTile(grant.service.ifBlank { "gmail" }, dim = true)
            Column(Modifier.weight(1f)) {
                RText(untrusted(grant.summary), RType.row, c.ink, maxLines = 2)
                RText(untrusted(grant.connectionLabel) + " · " + endedLine(grant).replaceFirstChar { it.lowercase() }, RType.meta, c.inkMeta, maxLines = 1)
            }
        }
        Row(Modifier.padding(start = 50.dp, top = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SignalButton("Resume", Modifier.testTag("resume:${grant.id}"), style = ButtonStyle.Soft, size = ButtonSize.Compact, glyph = Glyph.Refresh, onClick = onResume)
            SignalButton("Delete", Modifier.testTag("delete:${grant.id}"), style = ButtonStyle.DangerWash, size = ButtonSize.Compact, onClick = onDelete)
        }
    }
}
