package dev.reins.android.ui.signin

import androidx.compose.ui.unit.sp

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
            // On paper, not a field (iOS-Arrival 03): nothing waits on an AI here. The pink kind tile says it is an
            // account matter.
            Spacer(Modifier.height(56.dp))
            dev.reins.android.design.KindTile(dev.reins.android.design.Field.Accounts, Glyph.Key, size = 44.dp)
            dev.reins.android.design.AskText(
                "Protect your vault with a passkey",
                c.ink,
                Modifier.padding(top = 6.dp),
                target = "with a passkey",
                style = RType.serif(38f, 42f),
                minSize = 30.sp,
            )
            RText(
                "It unlocks your vault on a new phone, without your other phone or a recovery code. Your password manager keeps it.",
                RType.sans(16f, lineHeight = 22f),
                c.ink2,
            )
            Column(Modifier.padding(top = 6.dp)) {
                OfferLine(Glyph.Check, "Works with your password manager")
                OfferLine(Glyph.Lock, "Reins keeps only a sealed copy of the key")
            }
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
                "A recovery code comes next.",
                RType.sans(13f, lineHeight = 18f),
                c.tertiary,
                Modifier.padding(start = 4.dp),
            )
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun OfferLine(glyph: Glyph, text: String) {
    val c = LocalColors.current
    androidx.compose.foundation.layout.Row(
        Modifier.padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        GlyphIcon(glyph, c.ink2, size = 16.dp)
        RText(text, RType.meta, c.ink2)
    }
}
