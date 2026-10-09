package dev.reins.android.ui.signin

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.platform.PasskeyPrompt
import dev.reins.android.ui.settings.VaultPasskeysViewModel

/**
 * Before the recovery code, for an account without a passkey for its vault: one that opens the vault on a new or
 * reinstalled phone. Offered strongly, but "Use the recovery code only" goes on without one, so a password manager
 * without PRF never blocks anyone. The recovery code still follows either way.
 */
@Composable
fun PasskeyOfferScreen(viewModel: VaultPasskeysViewModel, passkeys: PasskeyPrompt) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    Screen(title = null, modifier = Modifier.testTag("passkeyOffer")) {
        Column(Modifier.padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Spacer(Modifier.height(64.dp))
            Box(
                Modifier.size(64.dp).background(c.accentSoft, RoundedCornerShape(20.dp)),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(Glyph.ShieldCheck, c.accent, size = 32.dp) }
            Spacer(Modifier.height(6.dp))
            RText("Protect your vault with a passkey", RType.sans(28f, FontWeight.SemiBold, lineHeight = 34f), c.text)
            RText(
                "A passkey unlocks your vault on a new or reinstalled phone, with no other phone or recovery code " +
                    "needed. Your password manager keeps it, behind your fingerprint or screen lock.",
                RType.sans(16f, lineHeight = 22f),
                c.secondary,
            )
            Spacer(Modifier.height(20.dp))
            ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "passkeyError") }
            CapsuleButton(
                "Add a passkey",
                Modifier.fillMaxWidth().testTag("addPasskey"),
                style = ButtonStyle.Primary,
                busy = ui.busy,
                glyph = Glyph.Key,
            ) { viewModel.add(passkeys) }
            CapsuleButton(
                "Use the recovery code only",
                Modifier.fillMaxWidth().testTag("skipPasskey"),
                style = ButtonStyle.Secondary,
                enabled = !ui.busy,
                onClick = viewModel::decline,
            )
            RText(
                "You still get a recovery code to write down next. Either one opens your vault.",
                RType.sans(13f, lineHeight = 18f),
                c.tertiary,
                Modifier.padding(start = 4.dp),
            )
            Spacer(Modifier.height(24.dp))
        }
    }
}
