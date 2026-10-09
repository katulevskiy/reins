package dev.reins.android.ui.pairing

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
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.ActionKind
import dev.reins.android.design.ActionTile
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.feedbackAction
import dev.reins.android.design.ConnectionAvatar
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Spinner
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.untrusted

/** Connecting a new AI or computer: confirm the two-digit code it shows (in the browser or the desktop app), name it, approve. */
@Composable
fun PairingSheet(viewModel: PairingViewModel, authenticator: Authenticator, onDone: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(ui.finished) { if (ui.finished) onDone() }
    val view = ui.view
    if (view == null) {
        Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            if (ui.loading) Spinner(c.secondary, size = 32.dp)
            ui.error?.let { Banner(it, kind = BannerKind.Error) }
        }
        return
    }
    Column(Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            Column(Modifier.padding(start = 20.dp, end = 60.dp, top = 8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ConnectionAvatar(untrusted(view.clientName), null, size = 44.dp)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        RText(untrusted(view.clientName), RType.sans(17f, FontWeight.SemiBold), c.text, maxLines = 1)
                        RText(view.clientHost, RType.mono(13f), c.secondary, maxLines = 1, ltr = true)
                    }
                }
                Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    ActionTile(ActionKind.Pair, 1, size = 48.dp)
                    Spacer(Modifier.width(14.dp))
                    RText("Connect", RType.sans(26f, FontWeight.SemiBold), c.text, Modifier.testTag("what"))
                }
                RText(
                    "Only continue if you just started this connection yourself. Tap the two-digit code shown on your computer or in your browser.",
                    RType.sans(15f, lineHeight = 21f),
                    c.secondary,
                    Modifier.padding(top = 14.dp),
                )
            }
            view.keyFingerprint?.let { DesktopKeyCard(it) }
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 20.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                ui.codes.forEach { code ->
                    val label = "%02d".format(code)
                    CapsuleButton(
                        label,
                        Modifier.weight(1f).testTag("code:$label"),
                        style = if (ui.chosen == code) ButtonStyle.Accent else ButtonStyle.Secondary,
                        onClick = feedbackAction(Event.Selection) { viewModel.choose(code) },
                    )
                }
            }
            RText("NAME THIS CONNECTION", RType.sans(12.5f, FontWeight.Medium), c.secondary, Modifier.padding(start = 24.dp, bottom = 8.dp))
            RTextField(ui.label, viewModel::setLabel, "Name", Modifier.padding(horizontal = 16.dp), tag = "label")
            val policy by viewModel.startingPolicy.collectAsStateWithLifecycle()
            dev.reins.android.ui.grants.StartingRuleText.onPairing(policy)?.let {
                RText(it, RType.sans(13.5f, lineHeight = 19f), c.tertiary, Modifier.padding(start = 24.dp, end = 24.dp, top = 12.dp).testTag("startingRuleNote"))
            }
            ui.error?.let { Banner(it, Modifier.padding(16.dp), BannerKind.Error) }
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
            CapsuleButton("Connect", Modifier.weight(1.3f).testTag("approve"), enabled = !ui.busy && ui.chosen != null, busy = ui.busy) {
                viewModel.approve(authenticator)
            }
        }
    }
}

/**
 * The Reins desktop app's key, as eight digits the computer shows too. Matching them is what stops the server from
 * slipping in a key of its own, so they come first, large, and set apart like a permission request.
 */
@Composable
private fun DesktopKeyCard(fingerprint: String) {
    val c = LocalColors.current
    Column(
        Modifier
            .padding(start = 16.dp, end = 16.dp, top = 18.dp)
            .fillMaxWidth()
            .testTag("keyFingerprint")
            .semantics(mergeDescendants = true) {}
            .background(c.accent.copy(alpha = 0.09f), RoundedCornerShape(22.dp))
            .border(1.5.dp, c.accent.copy(alpha = 0.55f), RoundedCornerShape(22.dp))
            .padding(18.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            GlyphIcon(Glyph.Key, c.accent, size = 22.dp)
            Spacer(Modifier.width(8.dp))
            RText("Desktop app key", RType.sans(16f, FontWeight.SemiBold), c.text)
        }
        RText(fingerprint, RType.mono(36f, FontWeight.SemiBold).copy(letterSpacing = 1.5.sp), c.text, maxLines = 1, ltr = true)
        RText(
            "Check that your computer shows the same numbers. If they differ, deny.",
            RType.sans(14.5f, lineHeight = 20f),
            c.secondary,
        )
    }
}
