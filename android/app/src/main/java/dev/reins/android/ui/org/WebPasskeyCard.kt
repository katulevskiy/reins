package dev.reins.android.ui.org

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Card
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.core.OrgPasskeyView

/**
 * A web passkey the member just added in the browser console, waiting for this phone (O3): its fingerprint, the same
 * code the console shows, built by the core from the key hash the confirmation signs. Confirm vouches for it with the
 * enrolled key; Not me refuses (revokes) it.
 */
@Composable
fun WebPasskeyCard(key: OrgPasskeyView, busy: Boolean, enabled: Boolean, onAnswer: (Boolean) -> Unit) {
    val c = LocalColors.current
    Card(Modifier.testTag("orgPasskey:${key.credentialId}")) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                GlyphIcon(Glyph.Passkey, c.violet, size = 20.dp)
                RText("New web passkey", RType.sans(16.5f, FontWeight.SemiBold), c.ink)
            }
            RText(
                key.fingerprint,
                RType.mono(22f, FontWeight.Medium),
                c.ink,
                Modifier.testTag("orgPasskeyFingerprint:${key.credentialId}"),
                maxLines = 1,
                ltr = true,
            )
            RText("Matches your browser?", RType.sans(13.5f), c.inkMeta, maxLines = 1)
            Row(Modifier.fillMaxWidth().padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SignalButton(
                    "Not me",
                    Modifier.weight(1f).testTag("orgPasskeyRefuse:${key.credentialId}"),
                    style = ButtonStyle.DangerWash,
                    size = ButtonSize.Small,
                    glyph = Glyph.Close,
                    enabled = enabled,
                ) { onAnswer(false) }
                SignalButton(
                    "Confirm",
                    Modifier.weight(1f).testTag("orgPasskeyConfirm:${key.credentialId}"),
                    size = ButtonSize.Small,
                    glyph = Glyph.ShieldCheck,
                    enabled = enabled && !busy,
                ) { onAnswer(true) }
            }
        }
    }
}
