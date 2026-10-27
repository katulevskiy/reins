package dev.reins.android.ui.autopilot

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
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ActivityEntry
import dev.reins.core.AutopilotMode
import dev.reins.core.Verdict
import kotlinx.coroutines.launch

/** What should have happened instead of what did: a denial was a mistaken approval's correction, and the reverse. */
fun correctionFor(entry: ActivityEntry): Verdict = if (entry.outcome == "denied") Verdict.APPROVE else Verdict.DENY


/** The matrix's line on an entry ("Auto · Mine · Undoable"); recorded in shadow, it reads "Would be: …". */
fun matrixLineText(m: dev.reins.core.MatrixView): String = if (m.shadow) "Would be: ${m.line}" else m.line

/** The matrix's decision (or what it would have been) as one quiet row of an entry's autopilot section. */
@Composable
fun MatrixLine(m: dev.reins.core.MatrixView, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp).testTag("matrixLine"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(Glyph.Autopilot, c.inkMeta, size = 16.dp)
        Spacer(Modifier.width(10.dp))
        RText(matrixLineText(m), RType.sans(14.5f), c.secondary, Modifier.weight(1f).testTag("matrixLineText"), maxLines = 2)
    }
}

/**
 * "Ask me next time" on something Auto did: from now on that kind of thing asks (and that zone and kind pause for a
 * day). Stricter only, so no owner check. [done] shows it was said.
 */
@Composable
fun AskMeNextTime(done: Boolean, busy: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .then(if (!done && !busy) Modifier.pressable(highlight = c.controlFill, shape = androidx.compose.foundation.shape.RoundedCornerShape(0.dp), label = "Ask me next time", onClick = onClick) else Modifier)
            .testTag(if (done) "askedNextTime" else "askMeNextTime")
            .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(if (done) Glyph.Check else Glyph.Bell, if (done) c.success else c.accent, size = 18.dp)
        Spacer(Modifier.width(10.dp))
        RText(if (done) "Asks next time" else "Ask me next time", RType.sans(15f, FontWeight.Medium), if (done) c.secondary else c.accent, Modifier.weight(1f), maxLines = 1)
    }
}
