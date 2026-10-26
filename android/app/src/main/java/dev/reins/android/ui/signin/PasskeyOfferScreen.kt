package dev.reins.android.ui.signin

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.fieldSurface
import dev.reins.android.platform.PasskeyPrompt
import dev.reins.android.ui.settings.VaultPasskeysViewModel

/**
 * Before the recovery code, for an account without a passkey for its vault: one that opens the vault on a new or
 * reinstalled phone. Offered strongly, but Skip goes on without one, so a password manager
 * without PRF never blocks anyone. The recovery code still follows either way.
 */
@Composable
fun PasskeyOfferScreen(viewModel: VaultPasskeysViewModel, passkeys: PasskeyPrompt) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    // Arrive 03: the whole page on the accounts field; a key, the phone with a fingerprint, the vault opening; then
    // Add passkey, or Skip (the recovery code follows either way).
    Column(
        Modifier
            .fillMaxSize()
            .fieldSurface(dev.reins.android.design.Field.Accounts)
            .windowInsetsPadding(WindowInsets.systemBars)
            .padding(horizontal = 20.dp)
            .testTag("passkeyOffer"),
    ) {
        Spacer(Modifier.weight(1f))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            RoundMark(Glyph.Key, c.fieldAccounts)
            Dots()
            Box(
                Modifier.size(width = 70.dp, height = 118.dp).clip(RoundedCornerShape(18.dp)).background(Color.White),
                contentAlignment = Alignment.Center,
            ) {
                Box(Modifier.align(Alignment.TopCenter).padding(top = 9.dp).size(width = 20.dp, height = 5.dp).background(c.fieldAccounts.copy(alpha = 0.35f), CircleShape))
                GlyphIcon(Glyph.Fingerprint, c.fieldAccounts, size = 32.dp)
            }
            Dots()
            RoundMark(Glyph.LockOpen, c.success)
        }
        Spacer(Modifier.weight(1f))
        RText("Passkey", RType.display(56f), c.onField, Modifier.semantics { heading() })
        RText(
            "Opens your vault on a new phone. Your password manager keeps it.",
            RType.sans(15f, lineHeight = 21f),
            c.onField2,
            Modifier.padding(top = 8.dp),
        )
        Spacer(Modifier.weight(1.4f))
        ui.error?.let { Banner(it, Modifier.padding(bottom = 12.dp), kind = BannerKind.Error, tag = "passkeyError") }
        dev.reins.android.design.SignalButton(
            "Add passkey",
            Modifier.fillMaxWidth().testTag("addPasskey"),
            style = ButtonStyle.OnFieldWhite,
            enabled = !ui.busy,
            leading = {
                if (ui.busy) dev.reins.android.design.Spinner(c.fieldAccounts, size = 18.dp) else GlyphIcon(Glyph.Fingerprint, c.fieldAccounts, size = 18.dp)
            },
        ) { viewModel.add(passkeys) }
        dev.reins.android.design.SignalButton(
            "Skip",
            Modifier.padding(top = 10.dp, bottom = 16.dp).fillMaxWidth().testTag("skipPasskey"),
            style = ButtonStyle.OnFieldWash,
            enabled = !ui.busy,
            onClick = viewModel::decline,
        )
    }
}

/** A round white mark on the field. */
@Composable
private fun RoundMark(glyph: Glyph, tint: Color) {
    Box(Modifier.size(52.dp).clip(CircleShape).background(Color.White), contentAlignment = Alignment.Center) {
        GlyphIcon(glyph, tint, size = 24.dp)
    }
}

/** Three dots between the marks. */
@Composable
private fun Dots() {
    Row(Modifier.padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        repeat(3) { Box(Modifier.size(4.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.7f))) }
    }
}
