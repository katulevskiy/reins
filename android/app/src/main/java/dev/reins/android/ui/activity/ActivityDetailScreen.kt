package dev.rewarden.android.ui.activity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.design.ActionKind
import dev.rewarden.android.design.ActionTile
import dev.rewarden.android.design.BlobAvatar
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.pressable
import dev.rewarden.android.design.ConnectorTags
import dev.rewarden.android.design.EmptyState
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.Hairline
import dev.rewarden.android.design.ListRow
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen
import dev.rewarden.android.state.AppState
import dev.rewarden.android.ui.common.ConnectionIcon
import dev.rewarden.android.ui.common.OutcomeTag
import dev.rewarden.android.ui.common.formatFull
import dev.rewarden.android.ui.common.entryTitle
import dev.rewarden.android.ui.common.formatTime
import dev.rewarden.android.ui.common.operationTitle
import dev.rewarden.android.ui.common.untrusted

/** One operation, opened: who asked, what exactly, what was released or sent and to whom. */
@Composable
fun ActivityDetailScreen(
    entryId: Long,
    state: AppState,
    onBack: () -> Unit,
    onOpenGrant: (String) -> Unit,
    onOpenEmail: (entryId: Long, index: Int) -> Unit = { _, _ -> },
    /** "This was wrong": records what should have happened; returns an error to show, or null. */
    onCorrect: suspend (Long, dev.rewarden.core.Verdict) -> String? = { _, _ -> null },
) {
    val c = LocalColors.current
    val entries by state.activity.collectAsStateWithLifecycle()
    val grants by state.grants.collectAsStateWithLifecycle()
    val entry = entries.firstOrNull { it.id == entryId }

    Screen(title = "Details", onBack = onBack) {
        if (entry == null) {
            EmptyState(Glyph.List, "Not found", "That entry is no longer in the history.", tag = "entryGone")
            return@Screen
        }
        val action = ActionKind.of(entry.action)
        val info = entry.info
        Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ConnectionIcon(entry.connectionId, entry.connectionLabel, size = 40.dp)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    RText(untrusted(entry.connectionLabel), RType.sans(17f, FontWeight.SemiBold), c.text, maxLines = 1)
                    RText(formatFull(entry.at), RType.sans(12.5f), c.tertiary, maxLines = 1)
                }
                OutcomeTag(entry.outcome)
            }
            Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                ActionTile(action, entry.count.toInt(), size = 48.dp)
                Spacer(Modifier.width(14.dp))
                RText(
                    if (action == ActionKind.Grant) entryTitle("", entry.action, 1, entry.service, entry.outcome).removePrefix(": ") else operationTitle(entry.action, entry.count.toInt(), entry.service, entry.opTitle, entry.op),
                    RType.sans(26f, FontWeight.SemiBold, lineHeight = 32f),
                    c.text,
                    Modifier.weight(1f).testTag("detailTitle"),
                    maxLines = 2,
                )
            }
            ConnectorTags(entry.service, entry.account, Modifier.padding(top = 12.dp))
            RText(untrusted(entry.detail), RType.sans(15.5f, lineHeight = 21f), c.secondary, Modifier.padding(top = 12.dp).testTag("detailSummary"))
        }

        dev.rewarden.android.ui.autopilot.ActivityAutopilotSection(entry, onCorrect)

        info.query?.let { query ->
            Group(header = "Search") { ListRow(query, ltrSubtitle = true, modifier = Modifier.testTag("detailQuery")) }
        }

        info.email?.let { email ->
            Group(header = "The email") {
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

        if (info.messages.isNotEmpty()) {
            val other = entry.op.isNotEmpty()
            val noun = if (other) "Items" else "Emails"
            Group(header = if (entry.outcome == "denied") "$noun that were not shared" else "$noun shared (${info.messages.size})") {
                info.messages.forEachIndexed { i, m ->
                    if (i > 0) Hairline()
                    // Only the id is kept, never the text: opening an email fetches it from Gmail again.
                    val openable = m.id.isNotEmpty() && !other
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .testTag("detailMessage:$i")
                            .then(if (openable) Modifier.pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp)) { onOpenEmail(entry.id, i) } else Modifier)
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(Modifier.weight(1f)) {
                            RText(untrusted(m.from), RType.sans(15f, FontWeight.SemiBold), c.text, maxLines = 1, ltr = true)
                            if (m.subject.isNotEmpty()) {
                                RText(untrusted(m.subject), RType.sans(14.5f), c.text, Modifier.padding(top = 1.dp), maxLines = 2)
                            }
                            // Another integration keeps what was shared, so it can be read here without asking anyone.
                            if (m.text.isNotEmpty()) {
                                RText(untrusted(m.text), RType.sans(14f, lineHeight = 19f), c.secondary, Modifier.padding(top = 2.dp).testTag("detailText:$i"))
                            }
                            if (m.date != 0L) RText(formatTime(m.date), RType.sans(12f), c.tertiary, Modifier.padding(top = 3.dp))
                        }
                        if (openable) {
                            Spacer(Modifier.width(8.dp))
                            GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp)
                        }
                    }
                }
            }
        }

        if (info.accounts.isNotEmpty()) {
            Group(header = if (entry.outcome == "denied") "Accounts that were not shared" else "Accounts shared (${info.accounts.size})") {
                info.accounts.forEachIndexed { i, address ->
                    if (i > 0) Hairline(inset = 68.dp)
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp).testTag("sharedAccount:$i"), verticalAlignment = Alignment.CenterVertically) {
                        BlobAvatar(address, size = 36.dp)
                        Spacer(Modifier.width(14.dp))
                        RText(address, RType.sans(15.5f, FontWeight.Medium), c.text, maxLines = 1, ltr = true)
                    }
                }
            }
        }

        info.grantSummary?.let { untrusted(it) }?.let { summary ->
            Group(header = "Permission") { ListRow(summary) }
        }
        info.note?.let { note ->
            Group(header = "Note") { ListRow(untrusted(note)) }
        }

        entry.grantId?.let { id ->
            val known = grants.any { it.id == id }
            Group(header = "Covered by") {
                ListRow(
                    if (known) "A standing grant" else "A grant that no longer exists",
                    subtitle = if (known) grants.first { it.id == id }.summary else null,
                    glyph = Glyph.Key,
                    chevron = known,
                    modifier = Modifier.testTag("openGrantFromEntry"),
                    onClick = if (known) ({ onOpenGrant(id) }) else null,
                )
            }
        }
        Spacer(Modifier.padding(bottom = 32.dp))
    }
}
