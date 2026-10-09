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
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType

/** The phone has no screen lock, so no approval can work yet: says so, with the way to Android's settings. */
@Composable
fun ScreenLockCard(modifier: Modifier = Modifier, onSet: () -> Unit) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .background(c.warning.copy(alpha = 0.12f), RoundedCornerShape(18.dp))
            .padding(14.dp)
            .testTag("screenLockOff"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).background(c.warning.copy(alpha = 0.18f), CircleShape), contentAlignment = Alignment.Center) {
            GlyphIcon(Glyph.Lock, c.warning, size = 20.dp)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            RText("No screen lock", RType.sans(15.5f, FontWeight.SemiBold), c.text)
            RText(
                "Approving needs this phone's PIN, pattern, password or fingerprint. Until you set one, every approval fails.",
                RType.sans(13f, lineHeight = 18f),
                c.secondary,
                Modifier.padding(top = 2.dp),
            )
        }
        Spacer(Modifier.width(10.dp))
        CapsuleButton("Set one", Modifier.testTag("setScreenLock"), style = ButtonStyle.Accent, compact = true, onClick = onSet)
    }
}
