package dev.reins.android.design

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.reins.android.R

/** The ink drawings (`res/drawable{,-night}/signal_*.xml`, from `design/signal/assets/illustrations`). */
enum class Illustration(val res: Int, val aspect: Float) {
    /** All quiet: empty lists. */
    Pond(R.drawable.signal_pond, 320f / 170f),

    /** Arrival, onboarding. */
    Kite(R.drawable.signal_kite, 320f / 240f),
}

/**
 * An empty state, calm: an ink drawing and one mood line in the serif italic ("All quiet on the water."). Light and
 * dark drawings come from the resources.
 */
@Composable
fun Calm(
    mood: String,
    modifier: Modifier = Modifier,
    illustration: Illustration = Illustration.Pond,
    tag: String? = null,
    moodStyle: androidx.compose.ui.text.TextStyle = RType.mood,
    drawingWidth: androidx.compose.ui.unit.Dp = 420.dp,
    gap: androidx.compose.ui.unit.Dp = 18.dp,
) {
    val c = LocalColors.current
    Column(
        modifier.fillMaxWidth().then(if (tag != null) Modifier.testTag(tag) else Modifier),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(gap),
    ) {
        Image(
            painterResource(illustration.res),
            contentDescription = null,
            modifier = Modifier.widthIn(max = drawingWidth).fillMaxWidth().aspectRatio(illustration.aspect),
            contentScale = ContentScale.Fit,
        )
        RText(mood, moodStyle, c.ink2, align = TextAlign.Center)
    }
}
