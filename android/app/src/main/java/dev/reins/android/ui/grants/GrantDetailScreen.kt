package dev.reins.android.ui.grants

import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.sp
import dev.reins.android.design.AskText
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Choice
import dev.reins.android.design.Chooser
import dev.reins.android.design.Field
import dev.reins.android.design.FieldHeader
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.HoldButton
import dev.reins.android.design.LabelText
import dev.reins.android.design.RColors
import dev.reins.android.design.Ring
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalButton
import dev.reins.android.design.pressable
import dev.reins.android.ui.approval.AskDock
import dev.reins.android.ui.approval.FactGroup
import dev.reins.android.ui.approval.FactRow
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.PageDock
import dev.reins.android.design.ProgressRing
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Timers
import dev.reins.android.design.ValueRow
import dev.reins.android.design.clockColor
import dev.reins.android.design.compactDuration
import dev.reins.android.design.grantClock
import dev.reins.android.design.rememberNowState
import dev.reins.android.design.serviceName
import dev.reins.android.state.AppState
import dev.reins.android.ui.activity.HistoryRow
import dev.reins.android.ui.activity.dayTime
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.inactiveWord
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.BottomSheet
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.android.ui.settings.SettingsPage
import dev.reins.android.ui.settings.primaryFill
import dev.reins.core.GrantView
import dev.reins.core.StandingGrant
import kotlinx.coroutines.launch

/** What a grant never covers, where that is known: the line under its checks. */
fun neverLines(grant: GrantView): List<String> {
    val mail = grant.service.isBlank() || grant.service == "gmail"
    val git = grant.service in setOf("github", "gitlab", "codeberg", "bitbucket", "desktop")
    return when (grant.action) {
        "headlines" -> listOf("Opening anything")
        "send" -> if (mail) listOf("Reading your mail") else listOf("Reading anything")
        "read" -> if (mail) listOf("Sending or changing anything", "Codes and passwords") else listOf("Sending or changing anything")
        "search", "list", "accounts" -> listOf("Changing anything")
        "write" -> if (git) listOf("Force-push, delete branches") else emptyList()
        else -> emptyList()
    }
}

/** "5 h", "4 min", "6 d": the big ring's number and unit. */
fun ringLabel(seconds: Long?): String {
    if (seconds == null) return "∞"
    val compact = compactDuration(seconds)
    val n = compact.takeWhile { it.isDigit() }
    return when (compact.drop(n.length)) {
        "s" -> "$n s"
        "m" -> "$n min"
        "h" -> "$n h"
        "d" -> "$n d"
        "w" -> "$n wk"
        else -> "$n y"
    }
}

/** Where a grant came from, in a few words (the same texts as iOS). */
internal fun grantOrigin(origin: String, label: String): String = when {
    origin == "ai_request" -> "$label asked and you allowed it"
    origin == "user" -> "Created by you in advance"
    origin == "retry" -> "A one-time pass after you approved late"
    origin == "starter" -> "Your starting rule, when you connected $label"
    origin == "bypass" || origin.startsWith("bypass:") -> "Approved by Bypass; ends with it"
    origin == "overnight" || origin.startsWith("overnight:") -> "Approved by ${dev.reins.android.autopilot.AutopilotText.OVERNIGHT}; ends with it"
    else -> "Chosen while approving a request"
}

/**
 * A grant (Reins Signal, iOS-Trust 02): a violet field, because it is power already handed over, with the time left
 * as its number and a ring that drains live. Under it, what the grant may and never does, its uses, when it ends,
 * where it came from, and what it was used for. A running grant extends (a sheet with the chooser) or ends now, held;
 * an ended one resumes (the same sheet) or is deleted.
 */
@Composable
fun GrantDetailScreen(
    grantId: String,
    state: AppState,
    onBack: () -> Unit,
    onOpenEntry: (Long) -> Unit,
    onRevoke: suspend (String) -> String?,
    onResume: suspend (grantId: String, seconds: Long, standing: StandingGrant?) -> String? = { _, _, _ -> null },
    onDelete: suspend (grantId: String) -> String? = { null },
    /** Extend: a running grant goes on for this many seconds from now. */
    onExtend: suspend (grantId: String, seconds: Long) -> String? = { _, _ -> null },
    /** A permission for secrets: "Revoke on all computers" for its project and environment. */
    onRevokeSecrets: (suspend (project: String?, env: String?) -> RevokeEverywhereResult)? = null,
) {
    val c = LocalColors.current
    val grants by state.grants.collectAsStateWithLifecycle()
    val activity by state.activity.collectAsStateWithLifecycle()
    val grant = grants.firstOrNull { it.id == grantId }
    var resuming by remember { mutableStateOf(false) }
    var resumeMore by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }
    var extending by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    if (grant == null) {
        Box(Modifier.fillMaxSize().background(c.paper)) {
            SettingsPage(title = null, onBack = onBack) { EmptyState(Glyph.Key, "This grant is gone", null, tag = "grantGone") }
        }
        return
    }
    Box(Modifier.fillMaxSize().background(c.paper)) {
        Column(Modifier.fillMaxSize()) {
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                GrantField(grant, onBack)
                Column(Modifier.padding(top = 10.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    FactGroup(
                        tag = "itMay",
                        rows = buildList<@Composable () -> Unit> {
                            grant.lines.forEach { line -> add { MayLine(untrusted(line), allowed = true) } }
                            if (grant.lines.isEmpty()) add { MayLine(untrusted(grant.summary), allowed = true) }
                            neverLines(grant).forEach { line -> add { MayLine(line, allowed = false) } }
                        },
                    )
                    val now = (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000
                    FactGroup(
                        rows = listOf(
                            { UsesRow(grant) },
                            { FactRow(if (grant.active) "Ends" else inactiveWord(grant), grant.expiresAt?.let { dayTime(it, now) } ?: "Never on its own") },
                            { FactRow("Came from", grantOrigin(grant.origin, untrusted(grant.connectionLabel)), Modifier.testTag("origin"), maxLines = 2) },
                        ),
                    )
                    val used = remember(activity, grant.id) { activity.filter { it.grantId == grant.id } }
                    Column {
                        LabelText("Used for · ${used.size}", c.inkMeta, Modifier.padding(horizontal = 22.dp).padding(bottom = 4.dp).testTag("usedFor"))
                        if (used.isEmpty()) {
                            RText("Nothing yet", RType.meta, c.inkMeta, Modifier.padding(horizontal = 22.dp, vertical = 8.dp))
                        }
                        used.take(20).forEach { entry ->
                            Box(Modifier.testTag("usedEntry:${entry.id}")) { HistoryRow(entry, now, first = false, last = false, inset = 6.dp) { onOpenEntry(entry.id) } }
                        }
                    }
                    if (grant.action == "secrets" && onRevokeSecrets != null) {
                        RevokeEverywhere(grant.secretsProject, grant.secretsEnv, onRevokeSecrets, Modifier.padding(top = 4.dp))
                    }
                    Spacer(Modifier.height(24.dp))
                }
            }
            AskDock {
                error?.let { RText(it, RType.sans(13.5f, FontWeight.Medium, lineHeight = 18f), c.danger, Modifier.fillMaxWidth().testTag("grantError"), maxLines = 3) }
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    if (grant.active) {
                        SignalButton("Extend", Modifier.weight(1f).testTag("extend"), style = ButtonStyle.Soft, glyph = Glyph.Timer, enabled = !busy) { extending = true }
                        // Ending is held, like what can't be undone: the grant's power goes at once.
                        HoldButton("End now", c.danger, Modifier.weight(1f).testTag("revoke"), icon = null, enabled = !busy, busy = busy) {
                            busy = true
                            error = null
                            scope.launch {
                                error = onRevoke(grant.id)
                                busy = false
                                if (error == null) onBack()
                            }
                        }
                    } else {
                        SignalButton("Resume", Modifier.weight(1f).testTag("resume"), style = ButtonStyle.Soft, glyph = Glyph.Refresh, enabled = !busy) { resuming = true }
                        SignalButton("Delete", Modifier.weight(1f).testTag("deleteEnded"), style = ButtonStyle.DangerWash, enabled = !busy) { deleting = true }
                    }
                }
            }
        }

        // Extend for (a running grant) and Resume for (an ended one): the same sheet, the chooser with its menu.
        LengthSheet(
            grant.takeIf { extending || resuming },
            resume = resuming,
            onClose = {
                extending = false
                resuming = false
            },
            onConfirm = { seconds ->
                val wasResume = resuming
                extending = false
                resuming = false
                busy = true
                error = null
                scope.launch {
                    error = if (wasResume) resumeAsItWas(grant, seconds, onResume) else onExtend(grant.id, seconds)
                    busy = false
                }
            },
            onMore = if (resuming) {
                {
                    resuming = false
                    resumeMore = true
                }
            } else {
                null
            },
        )
    }

    if (deleting) {
        ConfirmDialog(
            title = "Delete this grant for good?",
            text = untrusted(grant.summary) + "\n\nIt can't be resumed.",
            confirmLabel = "Delete",
            onConfirm = {
                deleting = false
                busy = true
                scope.launch {
                    error = onDelete(grant.id)
                    busy = false
                    if (error == null) onBack()
                }
            },
            onDismiss = { deleting = false },
        )
    }

    // A custom time, a number of uses, or a different scope: the full resume form.
    if (resumeMore) {
        ResumeDialog(
            grant = grant,
            onDismiss = { resumeMore = false },
            onResume = { seconds, standing ->
                resumeMore = false
                busy = true
                scope.launch {
                    error = onResume(grant.id, seconds, standing)
                    busy = false
                }
            },
        )
    }
}

/** The violet field: back, "Grant · Gmail", the time left with its ring, what it allows, and whose it is. */
@Composable
private fun GrantField(grant: GrantView, onBack: () -> Unit) {
    val c = LocalColors.current
    val now = rememberNowState(1_000, untilMillis = grant.expiresAt?.let { it * 1000 })
    val clock = remember(grant) { derivedStateOf { grantClock(grant, now.value / 1000) } }
    FieldHeader(Field.Grant) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            RoundIconButton(Glyph.ChevronLeft, "Back", onField = true, size = 36.dp, onClick = onBack)
            LabelText("Grant · ${serviceName(grant.service.ifBlank { "gmail" })}", c.onField2, Modifier.weight(1f))
        }
        Row(Modifier.padding(top = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            val value = clock.value
            Row(Modifier.weight(1f).testTag("timeLeft"), verticalAlignment = Alignment.Bottom) {
                if (grant.active) {
                    RText(value.remaining?.let(::compactDuration) ?: "∞", RType.serif(56f, 58f), c.onField, maxLines = 1, ltr = true)
                    RText(if (value.remaining != null) " left" else " no end", RType.serif(26f, 34f, italic = true), c.onField, Modifier.padding(bottom = 6.dp), maxLines = 1)
                } else {
                    RText(inactiveWord(grant), RType.serif(48f, 52f), c.onField, maxLines = 1)
                }
            }
            if (grant.active) {
                Box(Modifier.testTag("pie")) { Ring(value.fraction, c.onField, c.onField.copy(alpha = 0.3f), 30.dp) }
            }
        }
        AskText(untrusted(grant.summary), c.onField, Modifier.testTag("grantTitle"), style = RType.serif(36f, 38f), minSize = 28.sp, maxLines = 4)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            CompositionLocalProvider(LocalColors provides RColors.Light) { ConnectionIcon(grant.connectionId, grant.connectionLabel, size = 20.dp) }
            RText(
                "${untrusted(grant.connectionLabel)} · ${serviceName(grant.service.ifBlank { "gmail" })}",
                RType.sans(14f, FontWeight.Medium),
                c.onField2,
                maxLines = 1,
            )
        }
    }
}

/** "Uses   ▬▬▬▭▭▭   3 of 10": a short bar per use when there is a cap. */
@Composable
private fun UsesRow(grant: GrantView) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(vertical = 9.dp).semantics(mergeDescendants = true) {}.testTag("uses"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText("Uses", RType.sans(15f), c.ink2, Modifier.weight(1f), maxLines = 1)
        grant.maxUses?.toInt()?.let { UseDots(grant.uses.toInt(), it, Modifier.padding(end = 12.dp)) }
        RText(
            grant.maxUses?.let { "${grant.uses} of $it" } ?: if (grant.uses == 0u) "Not used yet" else grant.uses.toString(),
            RType.sans(15f, FontWeight.Medium),
            c.ink,
            maxLines = 1,
        )
    }
}

/** A line of what it may do: a green check, or a red ban for what it never does. */
@Composable
private fun MayLine(text: String, allowed: Boolean) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(vertical = 13.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        GlyphIcon(if (allowed) Glyph.Check else Glyph.Block, if (allowed) c.success else c.danger, size = 18.dp)
        RText(text, RType.sans(15f, lineHeight = 20f), if (allowed) c.ink else c.ink2, Modifier.weight(1f), maxLines = 3)
    }
}
