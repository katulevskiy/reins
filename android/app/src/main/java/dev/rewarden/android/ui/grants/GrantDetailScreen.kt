package dev.rewarden.android.ui.grants

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import kotlinx.coroutines.launch
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.design.ActionKind
import dev.rewarden.android.design.ActionTile
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.ConfirmDialog
import dev.rewarden.android.design.ConnectorTags
import dev.rewarden.android.design.EmptyState
import dev.rewarden.android.design.ExpiryPie
import dev.rewarden.android.design.UsesMeter
import dev.rewarden.android.design.grantClock
import dev.rewarden.android.design.rememberNowMillis
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.Hairline
import dev.rewarden.android.design.ListRow
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen
import dev.rewarden.android.design.Tag
import dev.rewarden.android.state.AppState
import dev.rewarden.core.StandingGrant
import dev.rewarden.android.ui.common.ActivityRow
import dev.rewarden.android.ui.common.ConnectionIcon
import dev.rewarden.android.ui.common.formatExpiry
import dev.rewarden.android.ui.common.formatFull
import dev.rewarden.android.ui.common.inactiveWord
import dev.rewarden.android.ui.common.relativeTime
import dev.rewarden.android.ui.common.untrusted

@Composable
fun GrantDetailScreen(
    grantId: String,
    state: AppState,
    onBack: () -> Unit,
    onOpenEntry: (Long) -> Unit,
    onRevoke: suspend (String) -> String?,
    onResume: suspend (grantId: String, seconds: Long, standing: StandingGrant?) -> String? = { _, _, _ -> null },
    onDelete: suspend (grantId: String) -> String? = { null },
) {
    val c = LocalColors.current
    val grants by state.grants.collectAsStateWithLifecycle()
    val activity by state.activity.collectAsStateWithLifecycle()
    val grant = grants.firstOrNull { it.id == grantId }
    var confirming by remember { mutableStateOf(false) }
    var resuming by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = androidx.compose.runtime.rememberCoroutineScope()

    Screen(title = "Grant", subtitle = grant?.let { untrusted(it.connectionLabel) }, onBack = onBack) {
        if (grant == null) {
            EmptyState(Glyph.Key, "This grant is gone", "It was deleted or removed.", tag = "grantGone")
            return@Screen
        }
        val action = ActionKind.of(grant.action)
        Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (grant.active) {
                    ExpiryPie(grantClock(grant, rememberNowMillis(1_000) / 1000), size = 60.dp)
                } else {
                    ActionTile(action, 1, size = 52.dp)
                }
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    RText(untrusted(grant.summary), RType.sans(21f, FontWeight.SemiBold, lineHeight = 27f), c.text, Modifier.testTag("grantTitle"))
                    Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        if (grant.active) Tag("Active", tint = c.success) else Tag(inactiveWord(grant), tint = c.tertiary)
                        Tag(when (grant.action) { "send" -> "Sending"; "write" -> "Writing"; "list" -> "Listing"; "accounts" -> "Showing accounts"; else -> "Reading" }, tint = action.tint(c))
                    }
                }
            }
            ConnectorTags(grant.service, grant.account, Modifier.padding(top = 12.dp))
        }

        Group(header = "What it allows") {
            grant.lines.forEachIndexed { i, line ->
                if (i > 0) Hairline()
                ListRow(untrusted(line), ltrSubtitle = true)
            }
            if (grant.lines.isEmpty()) ListRow("Nothing specific")
        }

        Group(header = "Details") {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                ConnectionIcon(grant.connectionId, grant.connectionLabel, size = 28.dp)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    RText("AI", RType.sans(12.5f), c.tertiary)
                    RText(untrusted(grant.connectionLabel), RType.sans(16f, FontWeight.Medium), c.text)
                }
            }
            Hairline()
            ListRow("Expires", subtitle = grant.expiresAt?.let { "${formatExpiry(it)} · ${formatFull(it)}" } ?: "Never on its own")
            Hairline()
            Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp)) {
                RText("Uses", RType.sans(16f, FontWeight.Medium), c.text)
                UsesMeter(grant.uses.toInt(), grant.maxUses?.toInt(), Modifier.padding(top = 6.dp))
            }
            Hairline()
            ListRow("Last used", subtitle = grant.lastUsedAt?.let { "${relativeTime(it)} · ${formatFull(it)}" } ?: "Not used yet")
            Hairline()
            ListRow("Created", subtitle = formatFull(grant.createdAt))
            Hairline()
            ListRow("Came from", subtitle = originText(grant.origin, grant.connectionLabel))
        }

        val used = activity.filter { it.grantId == grant.id }
        Group(header = "Used for") {
            if (used.isEmpty()) {
                ListRow("Nothing yet", subtitle = "Operations this grant covers will be listed here.")
            }
            used.take(10).forEachIndexed { i, entry ->
                if (i > 0) Hairline(inset = 74.dp)
                ActivityRow(entry) { onOpenEntry(entry.id) }
            }
        }

        error?.let { Banner(it, Modifier.padding(16.dp), BannerKind.Error) }
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (!grant.active) {
                CapsuleButton(
                    "Resume",
                    Modifier.fillMaxWidth().testTag("resume"),
                    busy = busy,
                    glyph = Glyph.Refresh,
                ) { resuming = true }
            }
            if (grant.active) {
                CapsuleButton(
                    "Delete grant",
                    Modifier.fillMaxWidth().testTag("revoke"),
                    style = ButtonStyle.Destructive,
                    busy = busy,
                    glyph = Glyph.Trash,
                ) { confirming = true }
            } else {
                CapsuleButton(
                    "Delete for good",
                    Modifier.fillMaxWidth().testTag("deleteEnded"),
                    style = ButtonStyle.Destructive,
                    busy = busy,
                    glyph = Glyph.Trash,
                ) { deleting = true }
            }
        }
    }

    if (confirming && grant != null) {
        ConfirmDialog(
            title = "Delete this grant?",
            text = untrusted(grant.summary) + "\n\nYou can resume it later from the Expired list.",
            confirmLabel = "Delete",
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

fun originText(origin: String, label: String): String = when (origin) {
    "ai_request" -> "${untrusted(label)} asked and you allowed it"
    "user" -> "Created by you in advance"
    "retry" -> "A one-time pass after you approved late"
    else -> "Chosen while approving a request"
}
