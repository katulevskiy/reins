package dev.rewarden.android.ui.join

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.design.ActionKind
import dev.rewarden.android.design.ActionTile
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Spinner
import dev.rewarden.android.platform.Authenticator
import dev.rewarden.android.ui.common.untrusted

/**
 * "Add another phone", on the approval device: the code the new phone shows, large, then Deny or Approve. [onDone]
 * gets what to tell the user once the request was answered (null after a denial).
 */
@Composable
fun JoinSheet(viewModel: JoinViewModel, authenticator: Authenticator, onDone: (String?) -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(ui.finished) { if (ui.finished) onDone(ui.notice) }
    val view = ui.view
    if (view == null) {
        Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            if (ui.loading) Spinner(c.secondary, size = 32.dp)
            ui.error?.let { Banner(it, kind = BannerKind.Error) }
        }
        return
    }
    val device = untrusted(view.deviceName)
    Column(Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            Column(Modifier.padding(start = 20.dp, end = 60.dp, top = 8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ActionTile(ActionKind.Join, 1, size = 48.dp)
                    Spacer(Modifier.width(14.dp))
                    RText("Add $device?", RType.sans(26f, FontWeight.SemiBold), c.text, Modifier.testTag("joinTitle"), maxLines = 2)
                }
                RText(
                    "Another phone signed in to your account and asks for its keys. Approve only if it is yours and shows this code:",
                    RType.sans(15f, lineHeight = 21f),
                    c.secondary,
                    Modifier.padding(top = 14.dp),
                )
            }
            Column(
                Modifier
                    .padding(start = 16.dp, end = 16.dp, top = 18.dp)
                    .fillMaxWidth()
                    .background(c.accent.copy(alpha = 0.09f), RoundedCornerShape(22.dp))
                    .border(1.5.dp, c.accent.copy(alpha = 0.55f), RoundedCornerShape(22.dp))
                    .padding(18.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                RText(
                    view.code,
                    RType.mono(40f, FontWeight.SemiBold).copy(letterSpacing = 2.sp),
                    c.text,
                    Modifier.testTag("joinCode"),
                    maxLines = 1,
                    ltr = true,
                )
            }
            ui.error?.let { Banner(it, Modifier.padding(16.dp), BannerKind.Error, tag = "joinError") }
        }
        Row(
            Modifier
                .fillMaxWidth()
                .background(c.background)
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            CapsuleButton("Deny", Modifier.weight(1f).testTag("deny"), style = ButtonStyle.Secondary, enabled = !ui.busy, onClick = viewModel::deny)
            CapsuleButton("Approve", Modifier.weight(1.3f).testTag("approve"), enabled = !ui.busy, busy = ui.busy) {
                viewModel.approve(authenticator)
            }
        }
    }
}
