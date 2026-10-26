package dev.reins.android.ui.grants

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.design.GrantClock
import dev.reins.android.design.LocalColors
import dev.reins.android.design.MiniLogo
import dev.reins.android.design.ProgressRing
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.clockColor
import dev.reins.android.design.grantClock
import dev.reins.android.design.plainCard
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowState
import dev.reins.android.design.toneCard
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.untrusted
import dev.reins.core.GrantView

/** The track of a grant's ring. */
fun ringTrack(c: RColors): Color = if (c.dark) Color(0xFF26262B) else c.text.copy(alpha = 0.08f)

/** "Claude Code · 3 of 10 uses", "Claude Code · used 12 times", "Claude Code · not used yet". */
fun grantUseLine(grant: GrantView): String {
    val uses = grant.uses.toInt()
    val max = grant.maxUses?.toInt()
    val what = when {
        max != null -> "$uses of $max uses"
        uses == 0 -> "not used yet"
        uses == 1 -> "used once"
        else -> "used $uses times"
    }
    return "${untrusted(grant.connectionLabel)} · $what"
}

/** Caps this small get one bar per use. */
const val MAX_DOTS = 20

/** One short bar per use, the used ones in the accent; a big cap is one bar that fills. */
@Composable
fun UseDots(uses: Int, max: Int, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val off = ringTrack(c)
    if (max <= MAX_DOTS) {
        Row(modifier.testTag("useDots"), horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            repeat(max) { i -> Box(Modifier.size(width = 14.dp, height = 4.dp).clip(RoundedCornerShape(2.dp)).background(if (i < uses) c.accent else off)) }
        }
    } else {
        val f = (uses.toFloat() / max).coerceIn(0f, 1f)
        Box(modifier.testTag("useDots").width(120.dp).height(4.dp).clip(RoundedCornerShape(2.dp)).background(off)) {
            Box(Modifier.fillMaxWidth(f).height(4.dp).background(c.accent))
        }
    }
}

/** The time left as a ring with "4m", "5h", "6d" or "∞" inside; red when it ends soon. Its clock recomposes only this. */
@Composable
fun GrantPie(clock: State<GrantClock>, size: Dp = 48.dp) {
    val c = LocalColors.current
    val value = clock.value
    val color = clockColor(value, c)
    Box(Modifier.size(size).testTag("pie"), contentAlignment = Alignment.Center) {
        ProgressRing(value.fraction, color, ringTrack(c), size = size, stroke = size * 0.125f)
        RText(value.label, RType.mono(size.value * 0.26f, FontWeight.SemiBold), color, maxLines = 1, ltr = true)
    }
}
