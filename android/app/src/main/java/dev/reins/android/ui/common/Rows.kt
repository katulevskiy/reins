package dev.reins.android.ui.common

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.LocalColors
import dev.reins.android.design.MiniLogo
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.core.GrantView

/** A permission that ended, with the ways to start it again or to get rid of it. */
@Composable
fun EndedGrantRow(grant: GrantView, modifier: Modifier = Modifier, onOpen: () -> Unit, onResume: () -> Unit, onDelete: () -> Unit) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onOpen)
            .padding(start = 20.dp, end = 12.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MiniLogo(
            grant.service.ifBlank { "gmail" },
            face = { size -> ConnectionIcon(grant.connectionId, grant.connectionLabel, size = size) },
            size = 40.dp,
            faceSize = 20.dp,
            ring = if (c.dark) androidx.compose.ui.graphics.Color(0xFF19191C) else c.elevated,
            modifier = Modifier.graphicsLayer { alpha = 0.6f },
        )
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(untrusted(grant.summary), RType.sans(15f, FontWeight.Medium), c.text, maxLines = 2)
            RText(untrusted(grant.connectionLabel) + " · " + endedLine(grant), RType.sans(12.5f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 1)
        }
        Spacer(Modifier.width(8.dp))
        RoundAction(Icons.Rounded.Refresh, "Resume", c.text, c.controlFill, Modifier.testTag("resume:${grant.id}"), onResume)
        Spacer(Modifier.width(8.dp))
        RoundAction(Icons.Rounded.Delete, "Delete", c.danger, c.danger.copy(alpha = 0.13f), Modifier.testTag("delete:${grant.id}"), onDelete)
    }
}

@Composable
private fun RoundAction(icon: ImageVector, label: String, tint: androidx.compose.ui.graphics.Color, fill: androidx.compose.ui.graphics.Color, modifier: Modifier, onClick: () -> Unit) {
    Box(
        modifier
            .size(40.dp)
            .clip(CircleShape)
            .background(fill)
            .pressable(shape = CircleShape, label = label, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, Modifier.size(20.dp), tint = tint) }
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
