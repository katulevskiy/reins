package dev.reins.android.ui.grants

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.CloudOff
import androidx.compose.material.icons.rounded.DesktopAccessDisabled
import androidx.compose.material3.Icon
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
import dev.reins.android.AppContainer
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.settings.PillButton
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch

/** What "Revoke on all computers" came to: the computers told, those not told yet; or why it did not happen. */
sealed interface RevokeEverywhereResult {
    data class Revoked(val told: Int, val notDelivered: List<String>) : RevokeEverywhereResult

    data class Failed(val message: String) : RevokeEverywhereResult
}

/**
 * Ends the permissions for the secrets of [project] (and [env]) on the phone, then tells every paired computer to drop
 * the values it holds. A computer not told yet is told on a later sync.
 */
suspend fun revokeSecretsEverywhere(container: AppContainer, project: String?, env: String?): RevokeEverywhereResult = try {
    container.feedback.play(Event.Revoked)
    val revoked = container.core.revokeSecretsEverywhere(project, env)
    container.refreshPending()
    RevokeEverywhereResult.Revoked(revoked.told.toInt(), revoked.notDelivered)
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    container.feedback.play(Event.Error)
    RevokeEverywhereResult.Failed(e.userMessage())
}

/** "Revoke on all computers" for a project's environment, after a confirmation, and what came of it. */
@Composable
fun RevokeEverywhere(
    project: String?,
    env: String?,
    onRevoke: suspend (project: String?, env: String?) -> RevokeEverywhereResult,
    modifier: Modifier = Modifier,
) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    var confirming by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var result by remember(project, env) { mutableStateOf<RevokeEverywhereResult?>(null) }
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("revokeEverywhereSection")) {
        PillButton(
            "Revoke on all computers",
            Modifier.fillMaxWidth().testTag("revokeEverywhere"),
            icon = Icons.Rounded.DesktopAccessDisabled,
            background = c.danger.copy(alpha = 0.14f),
            foreground = c.danger,
            height = 52.dp,
            textSize = 15.5f,
            busy = busy,
        ) { confirming = true }
        when (val shown = result) {
            is RevokeEverywhereResult.Revoked -> Column(Modifier.padding(start = 6.dp, end = 6.dp, top = 12.dp).testTag("revokeResult")) {
                ResultLine(
                    Icons.Rounded.CheckCircle,
                    c.success,
                    if (shown.told > 0) "Revoked · ${if (shown.told == 1) "1 computer" else "${shown.told} computers"} told" else "Revoked",
                    "revokeTold",
                )
                shown.notDelivered.forEachIndexed { i, name ->
                    ResultLine(Icons.Rounded.CloudOff, c.warning, "Not yet delivered to ${untrusted(name)}", "notDelivered:$i")
                }
            }
            is RevokeEverywhereResult.Failed -> Banner(shown.message, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "revokeError")
            null -> Unit
        }
    }
    if (confirming) {
        val what = listOfNotNull(project, env).joinToString(" · ") { untrusted(it) }.ifEmpty { "Every project" }
        ConfirmDialog(
            title = "Revoke on all computers?",
            text = "$what. Computers drop the values they hold.",
            confirmLabel = "Revoke",
            onConfirm = {
                confirming = false
                busy = true
                scope.launch {
                    result = onRevoke(project, env)
                    busy = false
                }
            },
            onDismiss = { confirming = false },
        )
    }
}

@Composable
private fun ResultLine(icon: androidx.compose.ui.graphics.vector.ImageVector, tint: androidx.compose.ui.graphics.Color, text: String, tag: String) {
    val c = LocalColors.current
    Row(Modifier.padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, null, Modifier.size(17.dp), tint = tint)
        Spacer(Modifier.width(9.dp))
        RText(text, RType.sans(14.5f, FontWeight.Medium), c.text, Modifier.testTag(tag), maxLines = 2)
    }
}
