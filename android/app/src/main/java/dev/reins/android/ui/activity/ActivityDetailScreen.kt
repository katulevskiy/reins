package dev.reins.android.ui.activity

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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Undo
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Cancel
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.OnWarning
import dev.reins.android.design.PageDock
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.ServiceTile
import dev.reins.android.design.Timers
import dev.reins.android.design.ValueRow
import dev.reins.android.design.pressable
import dev.reins.android.design.toneCard
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.android.ui.settings.SettingsPage
import dev.reins.core.ActivityEntry
import dev.reins.core.Verdict
import kotlinx.coroutines.launch

/** One operation, opened: who asked what of which service, how it ended, and what exactly went out. */
@Composable
fun ActivityDetailScreen(
    entryId: Long,
    state: AppState,
    onBack: () -> Unit,
    onOpenGrant: (String) -> Unit,
    onOpenEmail: (entryId: Long, index: Int) -> Unit = { _, _ -> },
    /** "This was wrong": records what should have happened; returns an error to show, or null. */
    onCorrect: suspend (Long, Verdict) -> String? = { _, _ -> null },
) {
    val c = LocalColors.current
    val entries by state.activity.collectAsStateWithLifecycle()
    val grants by state.grants.collectAsStateWithLifecycle()
    val entry = entries.firstOrNull { it.id == entryId }
    val scope = rememberCoroutineScope()
    var asking by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var thanked by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val correctable = entry?.autopilot?.correctable == true

    Box(Modifier.fillMaxSize().background(c.background)) {
        SettingsPage(title = null, onBack = onBack) {
            if (entry == null) {
                EmptyState(Glyph.List, "Not found", null, tag = "entryGone")
                return@SettingsPage
            }
            Hero(entry)
            Details(entry, grants.any { it.id == entry.grantId }, onOpenGrant, onOpenEmail)
            Spacer(Modifier.height(if (correctable || thanked || error != null) 120.dp else 32.dp))
        }
        if (entry != null && (correctable || thanked || error != null)) {
            PageDock(Modifier.align(Alignment.BottomCenter).testTag("entryDock")) {
                error?.let { Banner(it, Modifier.padding(bottom = 10.dp), BannerKind.Error) }
                if (thanked) {
                    Banner("Learned. It asks about these again.", tag = "corrected")
                } else if (correctable) {
                    PillButton(
                        "This was wrong",
                        Modifier.fillMaxWidth().testTag("thisWasWrong"),
                        icon = Icons.AutoMirrored.Rounded.Undo,
                        // The bright yellow in both themes: the dark text needs it.
                        background = Color(0xFFFACC15),
                        foreground = OnWarning,
                        height = 56.dp,
                        textSize = 16f,
                        busy = busy,
                    ) { asking = true }
                }
            }
        }
    }

    if (asking && entry != null) {
        val should = dev.reins.android.ui.autopilot.correctionFor(entry)
        ConfirmDialog(
            title = if (should == Verdict.DENY) "Should this have been denied?" else "Should this have been approved?",
            text = "Autopilot learns from it. What was done stays done.",
            confirmLabel = if (should == Verdict.DENY) "Deny next time" else "Approve next time",
            destructive = false,
            onConfirm = {
                asking = false
                busy = true
                scope.launch {
                    error = onCorrect(entry.id, should)
                    busy = false
                    thanked = error == null
                }
            },
            onDismiss = { asking = false },
        )
    }
}

/** The entry's hero: the AI, three dots, the service; what it did, large; and who decided. */
@Composable
private fun Hero(entry: ActivityEntry) {
    val c = LocalColors.current
    val tone = if (entry.outcome == "denied") c.danger else toneColor(toneOf(entry.action), c)
    val decided = entry.decidedBy.isNotEmpty() || entry.autopilot != null
    Column(
        Modifier
            .padding(horizontal = 12.dp)
            .fillMaxWidth()
            .toneCard(tone, radius = 28.dp, top = if (c.dark) 0.16f else 0.10f, bottom = if (c.dark) 0.04f else 0.03f)
            .testTag(if (decided) "entryAutopilot" else "entryHero")
            .padding(start = 20.dp, end = 20.dp, top = 22.dp, bottom = 18.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            ConnectionIcon(entry.connectionId, entry.connectionLabel, size = 52.dp)
            Spacer(Modifier.width(14.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                Box(Modifier.size(5.dp).background(c.tertiary, CircleShape))
                Box(Modifier.size(5.dp).background(c.tertiary, CircleShape))
                Box(Modifier.size(5.dp).background(tone, CircleShape))
            }
            Spacer(Modifier.width(14.dp))
            ServiceTile(entry.service.ifBlank { "gmail" }, size = 52.dp, fallback = kindIcon(entry.action), fallbackTint = tone)
        }
        SentenceText(historyParts(entry), Modifier.padding(top = 16.dp).testTag("detailTitle"), size = 23f, lineHeight = 30f, maxLines = 3, align = TextAlign.Center)
        untrusted(entry.detail).takeIf { it.isNotBlank() }?.let {
            RText(it, RType.sans(13.5f, lineHeight = 18f), c.tertiary, Modifier.padding(top = 6.dp).testTag("detailSummary"), maxLines = 2, align = TextAlign.Center)
        }
        OutcomePill(entry, Modifier.padding(top = 14.dp))
    }
}

@Composable
private fun OutcomePill(entry: ActivityEntry, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val (icon: ImageVector, tint: Color) = when (outcomeOf(entry)) {
        Outcome.Auto -> Icons.Rounded.Bolt to c.accent
        Outcome.AutoDenied -> Icons.Rounded.Bolt to c.danger
        Outcome.Bypass -> Icons.Rounded.Bolt to c.danger
        Outcome.Lockdown -> Icons.Rounded.Lock to c.secondary
        Outcome.Grant -> Icons.Rounded.Key to c.accent
        Outcome.Denied -> Icons.Rounded.Cancel to c.danger
        Outcome.Failed -> Icons.Rounded.Warning to c.warning
        Outcome.Approved -> Icons.Rounded.CheckCircle to c.success
    }
    Row(
        modifier.clip(RoundedCornerShape(10.dp)).background(tint.copy(alpha = 0.16f)).padding(start = 9.dp, end = 12.dp, top = 7.dp, bottom = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(15.dp), tint = tint)
        Spacer(Modifier.width(6.dp))
        RText(outcomeLine(entry), RType.sans(13f, FontWeight.SemiBold), tint, Modifier.testTag("entryAutopilotTitle"), maxLines = 1)
    }
}

private fun whenLine(at: Long): String = dayTime(at, (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000)

@Composable
private fun Details(entry: ActivityEntry, grantKnown: Boolean, onOpenGrant: (String) -> Unit, onOpenEmail: (Long, Int) -> Unit) {
    val c = LocalColors.current
    val info = entry.info
    // Another integration's items keep their text: shown straight away. Emails open from a row.
    var messagesOpen by rememberSaveable { mutableStateOf(entry.op.isNotEmpty()) }
    var accountsOpen by rememberSaveable { mutableStateOf(false) }
    var whyOpen by rememberSaveable { mutableStateOf(false) }
    val denied = entry.outcome == "denied"
    SettingsGroup(gap = 14.dp) {
        ValueRow("When", value = whenLine(entry.at))
        entry.account?.takeIf { it.isNotBlank() }?.let { ValueRow("Account", value = it, mono = true) }
        info.query?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { ValueRow("Search", Modifier.testTag("detailQuery"), value = it, mono = true) }
        if (info.messages.isNotEmpty()) {
            val noun = if (entry.op.isNotEmpty()) "Items" else "Emails"
            ValueRow(
                if (denied) "$noun not shared" else "$noun shared",
                Modifier.testTag("emailsShared"),
                value = info.messages.size.toString(),
                open = messagesOpen,
            ) { messagesOpen = !messagesOpen }
            AnimatedVisibility(messagesOpen) { Messages(entry, onOpenEmail) }
        }
        if (info.accounts.isNotEmpty()) {
            ValueRow(
                if (denied) "Accounts not shared" else "Accounts shared",
                Modifier.testTag("accountsShared"),
                value = info.accounts.size.toString(),
                open = accountsOpen,
            ) { accountsOpen = !accountsOpen }
            AnimatedVisibility(accountsOpen) {
                Column {
                    info.accounts.forEachIndexed { i, address ->
                        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("sharedAccount:$i"), verticalAlignment = Alignment.CenterVertically) {
                            BlobAvatar(address, size = 30.dp)
                            Spacer(Modifier.width(12.dp))
                            RText(address, RType.mono(14f), c.text, maxLines = 1, ltr = true)
                        }
                    }
                }
            }
        }
        entry.grantId?.let { id ->
            ValueRow(
                if (grantKnown) "Grant" else "A grant that ended",
                Modifier.testTag("openGrantFromEntry"),
                value = info.grantSummary?.let(::untrusted),
                onClick = if (grantKnown) ({ onOpenGrant(id) }) else null,
            )
        }
        val note = entry.autopilot
        if (note != null && (note.reason.isNotBlank() || note.neighbours.isNotEmpty() || note.pApprove > 0f || note.pDeny > 0f)) {
            val title = when {
                entry.decidedBy == "autopilot" && denied -> "Why Autopilot denied it"
                entry.decidedBy == "autopilot" -> "Why Autopilot allowed it"
                else -> "What Autopilot thought"
            }
            ValueRow(title, Modifier.testTag("whyAutopilot"), open = whyOpen) { whyOpen = !whyOpen }
            AnimatedVisibility(whyOpen) {
                Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 14.dp).testTag("whyDetail"), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    RText(
                        "Approve ${AutopilotText.percent(note.pApprove)} · Deny ${AutopilotText.percent(note.pDeny)} · ${AutopilotText.name(note.mode)}",
                        RType.sans(13.5f, FontWeight.Medium),
                        c.secondary,
                    )
                    note.reason.takeIf { it.isNotBlank() }?.let { RText(untrusted(it), RType.sans(14.5f, lineHeight = 20f), c.text) }
                    note.neighbours.forEach { RText("· " + untrusted(it), RType.sans(13.5f), c.tertiary, maxLines = 2) }
                }
            }
        }
    }

    info.email?.let { email ->
        SettingsGroup(header = "The email") {
            Column(Modifier.padding(16.dp).testTag("detailEmail"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                RText("To", RType.sans(12f, FontWeight.Medium), c.tertiary)
                email.to.forEach { RText(untrusted(it), RType.mono(14f), c.text, ltr = true) }
                if (email.cc.isNotEmpty()) {
                    RText("Cc", RType.sans(12f, FontWeight.Medium), c.tertiary, Modifier.padding(top = 6.dp))
                    email.cc.forEach { RText(untrusted(it), RType.mono(14f), c.text, ltr = true) }
                }
                RText("Subject", RType.sans(12f, FontWeight.Medium), c.tertiary, Modifier.padding(top = 6.dp))
                RText(untrusted(email.subject), RType.sans(16f, FontWeight.SemiBold), c.text)
                RText("Message", RType.sans(12f, FontWeight.Medium), c.tertiary, Modifier.padding(top = 6.dp))
                RText(untrusted(email.body), RType.sans(15f, lineHeight = 21f), c.text)
            }
        }
    }
    info.note?.let { note ->
        SettingsGroup(header = "Note") {
            RText(untrusted(note), RType.sans(15f, lineHeight = 21f), c.text, Modifier.padding(16.dp))
        }
    }
    if (entry.grantId == null) {
        info.grantSummary?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { summary ->
            SettingsGroup(header = "Permission") { RText(summary, RType.sans(15f, lineHeight = 21f), c.text, Modifier.padding(16.dp)) }
        }
    }
}

/** What was shared (or held back): one line per email or item; an email opens from Gmail again. */
@Composable
private fun Messages(entry: ActivityEntry, onOpenEmail: (Long, Int) -> Unit) {
    val c = LocalColors.current
    val other = entry.op.isNotEmpty()
    Column(Modifier.padding(bottom = 6.dp)) {
        entry.info.messages.forEachIndexed { i, m ->
            // Only the id is kept, never the text: opening an email fetches it from Gmail again.
            val openable = m.id.isNotEmpty() && !other
            Row(
                Modifier
                    .fillMaxWidth()
                    .testTag("detailMessage:$i")
                    .then(if (openable) Modifier.pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp)) { onOpenEmail(entry.id, i) } else Modifier)
                    .padding(horizontal = 16.dp, vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    RText(untrusted(m.from), RType.sans(14.5f, FontWeight.SemiBold), c.text, maxLines = 1, ltr = true)
                    if (m.subject.isNotEmpty()) RText(untrusted(m.subject), RType.sans(14f), c.secondary, maxLines = 2)
                    // Another integration keeps what was shared, so it can be read here without asking anyone.
                    if (m.text.isNotEmpty()) {
                        RText(untrusted(m.text), RType.sans(14f, lineHeight = 19f), c.secondary, Modifier.padding(top = 2.dp).testTag("detailText:$i"))
                    }
                }
                if (openable) {
                    Spacer(Modifier.width(8.dp))
                    Icon(androidx.compose.material.icons.Icons.Rounded.ChevronRight, null, Modifier.size(18.dp), tint = c.tertiary)
                }
            }
        }
    }
}
