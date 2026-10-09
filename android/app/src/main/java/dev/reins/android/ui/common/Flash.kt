package dev.reins.android.ui.common

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.state.AppState
import kotlinx.coroutines.delay

/** How long a flash stays (a tap takes it away sooner). */
private const val FLASH_MS = 4_000L

/**
 * A few words at the bottom of the screen when something done in the background did not work ("Not approved: offline").
 * It goes by itself; nothing waits for it.
 */
@Composable
fun FlashHost(state: AppState, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val message by state.flash.collectAsStateWithLifecycle()
    var shown by remember { mutableStateOf("") }
    LaunchedEffect(message) {
        val m = message ?: return@LaunchedEffect
        shown = m
        delay(FLASH_MS)
        if (state.flash.value == m) state.flash(null)
    }
    AnimatedVisibility(message != null, modifier, enter = fadeIn() + slideInVertically { it / 2 }, exit = fadeOut() + slideOutVertically { it / 2 }) {
        RText(
            shown,
            RType.sans(14.5f, FontWeight.Medium),
            c.text,
            Modifier
                .padding(horizontal = 24.dp)
                .glass(c, RoundedCornerShape(18.dp), 8.dp)
                .pressable(shape = RoundedCornerShape(18.dp)) { state.flash(null) }
                .padding(horizontal = 16.dp, vertical = 12.dp)
                .testTag("flash"),
            maxLines = 2,
        )
    }
}
