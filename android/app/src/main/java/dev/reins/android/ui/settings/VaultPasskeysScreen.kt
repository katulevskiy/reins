package dev.reins.android.ui.settings

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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.pressable
import dev.reins.android.platform.PasskeyJson
import dev.reins.android.platform.PasskeyPrompt
import dev.reins.android.ui.common.formatDate
import dev.reins.android.ui.common.untrusted
import dev.reins.core.VaultPasskeyView

/** Settings > Vault passkeys: the passkeys that open the vault on a new phone, adding one, removing one. */
@Composable
fun VaultPasskeysScreen(viewModel: VaultPasskeysViewModel, passkeys: PasskeyPrompt, onBack: () -> Unit) {
    val c = LocalColors.current
    val list by viewModel.passkeys.collectAsStateWithLifecycle()
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    var removing by remember { mutableStateOf<VaultPasskeyView?>(null) }
    LaunchedEffect(viewModel) { viewModel.load() }

    Screen(title = "Vault passkeys", onBack = onBack) {
        Group(
            header = "Passkeys",
            footer = "Each one opens your vault on a new or reinstalled phone, without your other phone or the recovery code.",
        ) {
            val current = list
            when {
                current == null -> RText("Loading…", RType.sans(15f), c.secondary, Modifier.padding(16.dp))
                current.isEmpty() -> Column(Modifier.padding(16.dp).testTag("noPasskeys")) {
                    RText("No passkey yet", RType.sans(16f, FontWeight.Medium), c.text)
                    RText(
                        "Add one to unlock your vault on a new phone.",
                        RType.sans(13f, lineHeight = 18f),
                        c.secondary,
                        Modifier.padding(top = 2.dp),
                    )
                }
                else -> current.forEachIndexed { i, passkey ->
                    if (i > 0) Hairline(inset = 68.dp)
                    PasskeyRow(passkey, busy = ui.busy) { removing = passkey }
                }
            }
        }
        Column(Modifier.padding(horizontal = 16.dp, vertical = 16.dp)) {
            CapsuleButton(
                "Add a passkey",
                Modifier.fillMaxWidth().testTag("addPasskey"),
                busy = ui.busy,
                glyph = Glyph.Plus,
            ) { viewModel.add(passkeys) }
            ui.error?.let { Banner(it, Modifier.padding(top = 12.dp), BannerKind.Error, tag = "passkeyError") }
        }
        RText(
            "Your password manager keeps the passkey. Reins keeps only a copy of the vault's key that the passkey alone opens.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 32.dp, end = 32.dp, bottom = 32.dp),
        )
    }

    removing?.let { passkey ->
        ConfirmDialog(
            title = "Remove this passkey?",
            text = "It no longer opens your vault. It stays in your password manager until you delete it there.",
            confirmLabel = "Remove",
            onConfirm = {
                removing = null
                viewModel.remove(passkey.credentialId)
            },
            onDismiss = { removing = null },
        )
    }
}

@Composable
private fun PasskeyRow(passkey: VaultPasskeyView, busy: Boolean, onRemove: () -> Unit) {
    val c = LocalColors.current
    val id = PasskeyJson.b64(passkey.credentialId)
    Row(
        Modifier.fillMaxWidth().testTag("passkey:$id").padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).background(c.accentSoft, RoundedCornerShape(12.dp)), contentAlignment = Alignment.Center) {
            GlyphIcon(Glyph.Key, c.accent, size = 20.dp)
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(untrusted(passkey.name), RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1)
            RText("Added ${formatDate(passkey.createdAt)}", RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 1)
        }
        Spacer(Modifier.width(8.dp))
        Row(
            Modifier
                .testTag("removePasskey:$id")
                .pressable(enabled = !busy, shape = CircleShape, onClick = onRemove)
                .padding(10.dp),
        ) { GlyphIcon(Glyph.Trash, c.danger, size = 20.dp) }
    }
}
