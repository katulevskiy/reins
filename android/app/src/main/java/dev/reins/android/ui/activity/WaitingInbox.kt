package dev.reins.android.ui.activity

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.QuestionAnswer
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.SyncProblem
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.pane
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowState
import dev.reins.android.ui.common.AskSentenceText
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.IntegrationIcon
import dev.reins.android.ui.common.actionField
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.untrustedName
import dev.reins.core.PendingItem

// Waiting for you (Decisions v2, 03 §5.2): requests that became tickets, in a pane of their own above Today. The agent
// is not blocked on them; each row says when it is due, a "Why?" asked and what the agent said back (as the agent's
// words, never as a fact), and what changed since it was approved. One agent's tickets of one kind go together.

/**
 * The section: soonest deadline first, by agent. A tap opens the request's approval screen; [onBatch] answers a group
 * (Allow: its routine requests after one owner's check; Deny: all of them), as the stack answers a burst.
 */
@Composable
fun WaitingInbox(
    pending: List<PendingItem>,
    details: PendingDetails,
    onOpen: (PendingItem) -> Unit,
    onBatch: (Burst, Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val agents = remember(pending) { inboxByAgent(pending) }
    if (agents.isEmpty()) return
    val groups = remember(pending) { batchGroups(pending) }
    val c = LocalColors.current
    val count = agents.sumOf { it.items.size }
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp).pane().testTag("waitingInbox")) {
        Row(
            Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, top = 12.dp, bottom = 4.dp).semantics { heading() },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            LabelText("Waiting for you", c.ink2, Modifier.weight(1f))
            RText("$count", RType.section, c.ink2, Modifier.testTag("waitingCount"), maxLines = 1)
        }
        agents.forEach { agent ->
            AgentLine(agent)
            agent.items.forEachIndexed { i, item ->
                TicketRow(item, details, divider = i > 0) { onOpen(item) }
            }
            groups.filter { it.connectionId == agent.connectionId }.forEach { group -> BatchRow(group, onBatch) }
        }
        Box(Modifier.heightIn(min = 6.dp))
    }
}

/** Who the tickets below are from. */
@Composable
private fun AgentLine(agent: AgentTickets) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, top = 8.dp, bottom = 2.dp).testTag("ticketAgent:${agent.connectionId}"),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ConnectionIcon(agent.connectionId, agent.label, size = 18.dp)
        RText(untrustedName(agent.label), RType.sans(13f, FontWeight.SemiBold), c.ink2, Modifier.weight(1f), maxLines = 1)
    }
}

/** One ticket: what it is, as the waiting cards say it, when it is due, and where the conversation stands. */
@Composable
private fun TicketRow(item: PendingItem, details: PendingDetails, divider: Boolean, onClick: () -> Unit) {
    val c = LocalColors.current
    val ticket = item.ticket ?: return
    val sentence = remember(item, details.views[item.id]) { askSentence(item, details.views[item.id]) }
    val words = remember(sentence) { sentence.copy(text = rowAction(sentence.text)) }
    Row(
        Modifier
            .fillMaxWidth()
            .then(if (divider) Modifier.ruleAbove(c) else Modifier)
            .pressable(highlight = c.paperSunken.copy(alpha = 0.6f), shape = RoundedCornerShape(0.dp), label = "Review", onClick = onClick)
            .heightIn(min = 56.dp)
            .padding(horizontal = 14.dp, vertical = 9.dp)
            .testTag("ticket:${item.id}"),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Box(Modifier.padding(end = 5.dp, bottom = 5.dp)) {
            IntegrationIcon(sentence.provider, sentence.action, size = 36.dp, ring = c.paperRaised, field = actionField(sentence.action))
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            AskSentenceText(
                words,
                style = RType.rowSentence,
                color = c.ink,
                detailsColor = c.inkMeta,
                detailsStyle = RType.mono(12f),
                maxLines = 3,
                detailsMaxLines = 1,
                tag = "ticketTitle:${item.id}",
            )
            ticket.question?.let { question ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Chip("Why?", Icons.Rounded.QuestionAnswer, c.violet, Modifier.testTag("ticketWhy:${item.id}"))
                    RText(untrusted(question), RType.sans(13.5f), c.ink2, Modifier.weight(1f), maxLines = 2)
                }
            }
            ticket.agentReply?.let { AgentSays(it, Modifier.testTag("agentSays:${item.id}")) }
            ticket.stale?.let { stale ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Chip("Stale", Icons.Rounded.SyncProblem, c.warning, Modifier.testTag("ticketStale:${item.id}"))
                    RText(untrusted(stale), RType.sans(13.5f), c.ink2, Modifier.weight(1f), maxLines = 2)
                }
            }
        }
        Due(ticket.deadline, Modifier.testTag("ticketDue:${item.id}"))
    }
}

/** "Due in 3 h", red once under an hour or past. Its own clock: a tick redraws only this. */
@Composable
private fun Due(deadline: Long, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val now by rememberNowState(30_000, untilMillis = deadline * 1000)
    val seconds = now / 1000
    val pressing = deadline - seconds < 3_600
    Row(modifier.semantics(mergeDescendants = true) {}, horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(Icons.Rounded.Schedule, null, Modifier.size(13.dp), tint = if (pressing) c.danger else c.inkMeta)
        RText(
            dueIn(deadline, seconds),
            RType.sans(12.5f, FontWeight.Medium, tracking = 0.01f).copy(fontFeatureSettings = "tnum"),
            if (pressing) c.danger else c.inkMeta,
            maxLines = 1,
        )
    }
}

/** A small tinted pill: a state of the ticket ("Why?", "Stale"). */
@Composable
private fun Chip(text: String, icon: ImageVector, tint: Color, modifier: Modifier = Modifier) {
    Row(
        modifier.semantics(mergeDescendants = true) {}.clip(CircleShape).background(tint.copy(alpha = 0.13f)).padding(horizontal = 8.dp, vertical = 3.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(13.dp), tint = tint)
        RText(text, RType.sans(12f, FontWeight.SemiBold), tint, maxLines = 1)
    }
}

/**
 * The agent's answer to "Why?": its own words in quotes and italics, inside a ruled box under "Agent says", so it never
 * reads as one of Reins' facts.
 */
@Composable
private fun AgentSays(reply: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(12.dp)
    Column(
        modifier
            .semantics(mergeDescendants = true) {}
            .fillMaxWidth()
            .clip(shape)
            .border(1.dp, c.rule, shape)
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        RText("Agent says", RType.sans(11.5f, FontWeight.SemiBold, tracking = 0.02f), c.inkMeta, maxLines = 1)
        RText("“${untrusted(reply)}”", RType.sans(13.5f).copy(fontStyle = FontStyle.Italic), c.ink2, maxLines = 6)
    }
}

/** One agent's tickets of one kind: Allow N (only the routine ones) and Deny N (all). */
@Composable
private fun BatchRow(group: TicketGroup, onBatch: (Burst, Boolean) -> Unit) {
    val c = LocalColors.current
    val first = group.items.first()
    val kind = untrusted(first.opTitle).ifBlank { first.action.replaceFirstChar { it.uppercase() } }
    Row(
        Modifier.fillMaxWidth().ruleAbove(c, start = 14.dp).padding(horizontal = 14.dp, vertical = 10.dp).testTag("batch:${group.key}"),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText("$kind · ${group.items.size}", RType.sans(13.5f, FontWeight.Medium), c.ink2, Modifier.weight(1f), maxLines = 1)
        val allow = group.allowIds.size
        if (allow > 0) {
            SignalButton(
                "Allow $allow",
                Modifier.testTag("batchAllow:${group.key}"),
                style = ButtonStyle.Soft,
                size = ButtonSize.Compact,
                leading = { Icon(Icons.Rounded.Fingerprint, null, Modifier.size(16.dp), tint = c.ink) },
            ) { onBatch(group.burst, true) }
        }
        SignalButton(
            "Deny ${group.denyIds.size}",
            Modifier.testTag("batchDeny:${group.key}"),
            style = ButtonStyle.Soft,
            size = ButtonSize.Compact,
            leading = { Icon(Icons.Rounded.Block, null, Modifier.size(16.dp), tint = c.ink) },
        ) { onBatch(group.burst, false) }
    }
}
