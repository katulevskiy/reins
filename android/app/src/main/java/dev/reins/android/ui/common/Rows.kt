package dev.reins.android.ui.common

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ActionKind
import dev.reins.android.design.ActionTile
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ExpiryPie
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.TimeBarFrame
import dev.reins.android.design.UsesMeter
import dev.reins.android.design.clockColor
import dev.reins.android.design.grantClock
import dev.reins.android.design.rememberNowMillis
import dev.reins.android.design.ConnectorTags
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Tag
import dev.reins.android.design.pressable
import dev.reins.core.ActivityEntry
import dev.reins.core.GrantView

/** One operation in a list: what it was, who asked, which account, when, and how it ended. */
@Composable
fun ActivityRow(entry: ActivityEntry, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val action = ActionKind.of(entry.action)
    Row(
        modifier
            .fillMaxWidth()
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.Top,
    ) {
        ActionTile(action, entry.count.toInt(), size = 44.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ConnectionIcon(entry.connectionId, entry.connectionLabel, size = 20.dp)
                Spacer(Modifier.width(8.dp))
                RText(
                    entryTitle(entry.connectionLabel, entry.action, entry.count.toInt(), entry.service, entry.outcome, entry.opTitle, entry.op),
                    RType.sans(16f, FontWeight.SemiBold),
                    c.text,
                    Modifier.weight(1f),
                    maxLines = 2,
                )
            }
            RText(untrusted(entry.detail), RType.sans(14f), c.secondary, maxLines = 2)
            ConnectorTags(entry.service, entry.account, Modifier.padding(top = 3.dp))
        }
        Spacer(Modifier.width(10.dp))
        Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            RText(relativeTime(entry.at), RType.sans(12.5f), c.tertiary, maxLines = 1)
            OutcomeTag(entry.outcome)
            if (entry.decidedBy.isNotEmpty()) dev.reins.android.ui.autopilot.AutopilotBadge(entry.decidedBy)
        }
    }
}

@Composable
fun OutcomeTag(outcome: String) {
    val c = LocalColors.current
    when (outcome) {
        "denied" -> Tag("Denied", tint = c.danger)
        "error" -> Tag("Failed", tint = c.warning)
        "granted" -> Tag("Granted", tint = c.accent)
        "sent" -> Tag("Sent")
        "released" -> Tag("Allowed")
        else -> Tag(outcome.replaceFirstChar { it.uppercase() })
    }
}

/** A running permission: the time left as a pie and as a bar along the border, how much it was used, who and what. */
@Composable
fun GrantTile(grant: GrantView, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val action = ActionKind.of(grant.action)
    val now = rememberNowMillis(1_000)
    val clock = grantClock(grant, now / 1000)
    Box(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
        TimeBarFrame(
            fraction = clock.fraction,
            color = clockColor(clock, c),
            modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(18.dp))
                .background(c.elevated)
                .pressable(highlight = c.controlFill, shape = RoundedCornerShape(18.dp), onClick = onClick),
        ) {
            Row(Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, top = 14.dp, bottom = 14.dp), verticalAlignment = Alignment.CenterVertically) {
                ExpiryPie(clock, size = 54.dp)
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                    RText(untrusted(grant.summary), RType.sans(16f, FontWeight.SemiBold), c.text, maxLines = 2)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        ConnectionIcon(grant.connectionId, grant.connectionLabel, size = 18.dp)
                        Spacer(Modifier.width(7.dp))
                        RText(untrusted(grant.connectionLabel), RType.sans(14f, FontWeight.Medium), c.secondary, maxLines = 1)
                        Spacer(Modifier.width(8.dp))
                        Tag(when (grant.action) { "send" -> "Sends"; "write" -> "Writes"; "list" -> "Lists"; "accounts" -> "Shows accounts"; else -> "Reads" }, tint = action.tint(c))
                    }
                    UsesMeter(grant.uses.toInt(), grant.maxUses?.toInt())
                    ConnectorTags(grant.service, grant.account, Modifier.padding(top = 1.dp))
                }
                if (clock.soon) {
                    Spacer(Modifier.width(8.dp))
                    Tag("Ends soon", Modifier.testTag("endsSoon"), tint = c.danger)
                }
            }
        }
    }
}

/** A permission that ended, with the ways to start it again or to get rid of it. */
@Composable
fun EndedGrantRow(grant: GrantView, modifier: Modifier = Modifier, onOpen: () -> Unit, onResume: () -> Unit, onDelete: () -> Unit) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onOpen)
            .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Box(
            Modifier.size(44.dp).background(c.controlFill, androidx.compose.foundation.shape.CircleShape),
            contentAlignment = Alignment.Center,
        ) { GlyphIcon(if (grant.state == "revoked") Glyph.Trash else Glyph.Clock, c.tertiary, size = 20.dp) }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            RText(untrusted(grant.summary), RType.sans(15.5f, FontWeight.Medium), c.text, maxLines = 2)
            Row(verticalAlignment = Alignment.CenterVertically) {
                ConnectionIcon(grant.connectionId, grant.connectionLabel, size = 18.dp)
                Spacer(Modifier.width(6.dp))
                RText(untrusted(grant.connectionLabel) + " · " + endedLine(grant), RType.sans(13f), c.secondary, maxLines = 1)
            }
            ConnectorTags(grant.service, grant.account, Modifier.padding(top = 2.dp))
            Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CapsuleButton("Resume", Modifier.testTag("resume:${grant.id}"), style = ButtonStyle.Secondary, compact = true, glyph = Glyph.Refresh, onClick = onResume)
                CapsuleButton("Delete", Modifier.testTag("delete:${grant.id}"), style = ButtonStyle.Destructive, compact = true, glyph = Glyph.Trash, onClick = onDelete)
            }
        }
    }
}

/** How a finished grant ended: "Expired 2 d ago", "All 5 uses spent", "Deleted". */
fun endedLine(g: GrantView): String = when (g.state) {
    "revoked" -> "Deleted"
    "used_up" -> "All ${g.maxUses ?: g.uses} uses spent"
    else -> g.expiresAt?.let { "Expired ${relativeTime(it)}" } ?: "Expired"
}

fun inactiveWord(g: GrantView): String = when (g.state) {
    "revoked" -> "Deleted"
    "used_up" -> "Used up"
    else -> "Expired"
}
