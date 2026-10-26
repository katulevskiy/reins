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
