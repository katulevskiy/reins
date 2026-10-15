package dev.reins.android.ui.grants

import androidx.compose.animation.AnimatedVisibility
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

/** The extend sheet's choices: from now on, for this long. */
private val ExtendQuick = listOf(DurationChoice(3_600L, "1 hour", "extend:HOUR"), DurationChoice(86_400L, "1 day", "extend:DAY"))
private val ExtendMore = listOf(
    DurationChoice(4 * 3_600L, "4 hours", "extend:4H"),
    DurationChoice(7 * 86_400L, "7 days", "extend:WEEK"),
    DurationChoice(30 * 86_400L, "30 days", "extend:MONTH"),
)

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
    var confirming by remember { mutableStateOf(false) }
    var resuming by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }
    var extending by remember { mutableStateOf(false) }
    var extendFor by rememberSaveable { mutableStateOf(3_600L) }
    var usedOpen by rememberSaveable { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    Box(Modifier.fillMaxSize().background(c.background)) {
        SettingsPage(title = null, onBack = onBack) {
            if (grant == null) {
                EmptyState(Glyph.Key, "This grant is gone", null, tag = "grantGone")
                return@SettingsPage
            }
            Hero(grant)

            RText("It may", RType.sans(14f, FontWeight.Medium), c.accent, Modifier.padding(start = 22.dp, top = 22.dp, bottom = 8.dp))
            SettingsGroup(gap = 0.dp) {
                Column(Modifier.padding(vertical = 4.dp).testTag("itMay")) {
                    grant.lines.forEach { MayLine(untrusted(it), allowed = true) }
                    if (grant.lines.isEmpty()) MayLine(untrusted(grant.summary), allowed = true)
                    grant.maxUses?.toInt()?.let { MayLine(if (it == 1) "Once" else "Up to $it times", allowed = true) }
                    neverLines(grant).forEach { MayLine(it, allowed = false) }
                }
            }

            val used = remember(activity, grant.id) { activity.filter { it.grantId == grant.id } }
            val now = (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000
            SettingsGroup(gap = 12.dp) {
                ValueRow(
                    "Uses",
                    Modifier.testTag("uses"),
                    value = grant.maxUses?.let { "${grant.uses} of $it" } ?: if (grant.uses == 0u) "Not used yet" else grant.uses.toString(),
                )
                ValueRow(
                    if (grant.active) "Ends" else inactiveWord(grant),
                    value = grant.expiresAt?.let { dayTime(it, now) } ?: "Never on its own",
                )
                ValueRow("Came from", Modifier.testTag("origin"), value = grantOrigin(grant.origin, untrusted(grant.connectionLabel)))
                ValueRow("Used for", Modifier.testTag("usedFor"), value = used.size.toString(), open = usedOpen) { usedOpen = !usedOpen }
                AnimatedVisibility(usedOpen) {
                    Column(Modifier.padding(bottom = 8.dp)) {
                        if (used.isEmpty()) RText("Nothing yet", RType.sans(14f), c.tertiary, Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
                        used.take(20).forEach { entry ->
                            Box(Modifier.testTag("usedEntry:${entry.id}")) { HistoryRow(entry, now, first = false, last = false, inset = 0.dp) { onOpenEntry(entry.id) } }
                        }
                    }
                }
            }
            if (grant.action == "secrets" && onRevokeSecrets != null) {
                RevokeEverywhere(grant.secretsProject, grant.secretsEnv, onRevokeSecrets, Modifier.padding(top = 18.dp))
            }
            Spacer(Modifier.height(130.dp))
        }

        if (grant != null) {
            PageDock(Modifier.align(Alignment.BottomCenter)) {
                error?.let { Banner(it, Modifier.padding(bottom = 10.dp), BannerKind.Error) }
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    val neutral = if (c.dark) Color(0xFF232327) else c.controlFill
                    if (grant.active) {
                        PillButton("Extend", Modifier.weight(1f).testTag("extend"), icon = Icons.Rounded.Schedule, background = neutral, foreground = c.text, height = 56.dp, textSize = 16f, busy = busy) {
                            extending = true
                        }
                        PillButton("End now", Modifier.weight(1f).testTag("revoke"), icon = Icons.Rounded.Block, background = c.danger.copy(alpha = 0.14f), foreground = c.danger, height = 56.dp, textSize = 16f) {
                            confirming = true
                        }
                    } else {
                        PillButton("Resume", Modifier.weight(1f).testTag("resume"), icon = Icons.Rounded.Refresh, background = neutral, foreground = c.text, height = 56.dp, textSize = 16f, busy = busy) {
                            resuming = true
                        }
                        PillButton("Delete", Modifier.weight(1f).testTag("deleteEnded"), icon = Icons.Rounded.Delete, background = c.danger.copy(alpha = 0.14f), foreground = c.danger, height = 56.dp, textSize = 16f) {
                            deleting = true
                        }
                    }
                }
            }
        }

        BottomSheet(extending && grant != null, "Extend for", onClose = { extending = false }, tag = "extendSheet") {
            Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 8.dp)) {
                DurationControl(ExtendQuick, ExtendMore, extendFor, customTag = "extend:custom") { extendFor = it }
                RText("From now. Uses so far stay.", RType.sans(13f), c.tertiary, Modifier.padding(start = 6.dp, top = 10.dp, bottom = 16.dp))
                val (bg, fg) = primaryFill(c)
                PillButton("Extend", Modifier.fillMaxWidth().testTag("confirmExtend"), icon = Icons.Rounded.Fingerprint, background = bg, foreground = fg, height = 56.dp, textSize = 16.5f) {
                    val id = grant?.id ?: return@PillButton
                    extending = false
                    busy = true
                    error = null
                    scope.launch {
                        error = onExtend(id, extendFor)
                        busy = false
                    }
                }
            }
        }
    }

    if (confirming && grant != null) {
        ConfirmDialog(
            title = "End this grant now?",
            text = untrusted(grant.summary) + "\n\nYou can resume it later from Expired.",
            confirmLabel = "End now",
            onConfirm = {
                confirming = false
                busy = true
                scope.launch {
                    error = onRevoke(grant.id)
                    busy = false
                    if (error == null) onBack()
                }
            },
            onDismiss = { confirming = false },
        )
    }

    if (deleting && grant != null) {
        ConfirmDialog(
            title = "Delete this grant for good?",
            text = untrusted(grant.summary) + "\n\nIt cannot be resumed afterwards.",
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

    if (resuming && grant != null) {
        ResumeDialog(
            grant = grant,
            onDismiss = { resuming = false },
            onResume = { seconds, standing ->
                resuming = false
                busy = true
                scope.launch {
                    error = onResume(grant.id, seconds, standing)
                    busy = false
                }
            },
        )
    }
}

/** The big ring with the time left, what the grant allows in one sentence, and whose it is. */
@Composable
private fun Hero(grant: GrantView) {
    val c = LocalColors.current
    val now = rememberNowState(1_000, untilMillis = grant.expiresAt?.let { it * 1000 })
    val clock by remember(grant, now) { derivedStateOf { grantClock(grant, now.value / 1000) } }
    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 6.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(120.dp).testTag("pie"), contentAlignment = Alignment.Center) {
            if (grant.active) {
                val color = clockColor(clock, c)
                ProgressRing(clock.fraction, color, ringTrack(c), size = 120.dp, stroke = 14.dp)
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    RText(ringLabel(clock.remaining), RType.sans(28f, FontWeight.SemiBold), c.text, maxLines = 1)
                    RText("left", RType.sans(12f), c.tertiary, maxLines = 1)
                }
            } else {
                ProgressRing(0f, c.tertiary, ringTrack(c), size = 120.dp, stroke = 14.dp)
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(Icons.Rounded.History, null, Modifier.size(28.dp), tint = c.tertiary)
                    RText(inactiveWord(grant), RType.sans(12.5f, FontWeight.Medium), c.tertiary, maxLines = 1)
                }
            }
        }
        RText(untrusted(grant.summary), RType.sans(22f, FontWeight.SemiBold, lineHeight = 28f), c.text, Modifier.padding(top = 14.dp).testTag("grantTitle"), align = TextAlign.Center)
        Row(Modifier.padding(top = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            ConnectionIcon(grant.connectionId, grant.connectionLabel, size = 16.dp)
            Spacer(Modifier.width(6.dp))
            RText("${untrusted(grant.connectionLabel)} · ${serviceName(grant.service.ifBlank { "gmail" })}", RType.sans(13.5f), c.tertiary, maxLines = 1)
        }
    }
}

/** A line of "It may": a green check for what it may do, a red ban for what it never does. */
@Composable
private fun MayLine(text: String, allowed: Boolean) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 11.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(if (allowed) Icons.Rounded.Check else Icons.Rounded.Block, null, Modifier.size(18.dp), tint = if (allowed) c.success else c.danger)
        Spacer(Modifier.width(12.dp))
        RText(text, RType.sans(15f), if (allowed) c.text else c.tertiary, maxLines = 3)
    }
}
