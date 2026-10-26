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
import androidx.compose.ui.draw.clip
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

    // Settings 05: each passkey its own group (its phone, when it was added), then Add a passkey.
    Screen(title = "Passkeys", onBack = onBack) {
        val current = list
        when {
            current == null -> RText("Loading…", RType.sans(15f), c.inkMeta, Modifier.padding(horizontal = 24.dp, vertical = 16.dp))
            current.isEmpty() -> dev.reins.android.design.PaneGroup(Modifier.padding(top = 8.dp).testTag("noPasskeys")) {
                dev.reins.android.design.PaneRow("No passkey yet", leading = { dev.reins.android.design.RowTile(Glyph.Passkey) }, first = true)
            }
            else -> current.forEach { passkey -> PasskeyGroup(passkey, busy = ui.busy) { removing = passkey } }
        }
        Column(Modifier.padding(horizontal = 16.dp, vertical = 14.dp)) {
            CapsuleButton(
                "Add a passkey",
                Modifier.fillMaxWidth().testTag("addPasskey"),
                style = dev.reins.android.design.ButtonStyle.Soft,
                busy = ui.busy,
                glyph = Glyph.Plus,
            ) { viewModel.add(passkeys) }
            ui.error?.let { Banner(it, Modifier.padding(top = 12.dp), BannerKind.Error, tag = "passkeyError") }
            RText(
                "Opens your vault on a new phone. Your password manager keeps it.",
                RType.meta,
                c.inkMeta,
                Modifier.padding(start = 8.dp, end = 8.dp, top = 12.dp),
            )
        }
        androidx.compose.foundation.layout.Spacer(Modifier.padding(bottom = 32.dp))
    }

    removing?.let { passkey ->
        ConfirmDialog(
            title = "Remove this passkey?",
            text = "It stops opening your vault.",
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
private fun PasskeyGroup(passkey: VaultPasskeyView, busy: Boolean, onRemove: () -> Unit) {
    val c = LocalColors.current
    val id = PasskeyJson.b64(passkey.credentialId)
    dev.reins.android.design.PaneGroup(Modifier.padding(top = 8.dp, bottom = 6.dp).testTag("passkey:$id")) {
        Row(
            Modifier.fillMaxWidth().padding(start = 14.dp, end = 6.dp, top = 14.dp, bottom = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.size(44.dp).clip(RoundedCornerShape(12.dp)).background(c.inverse), contentAlignment = Alignment.Center) {
                GlyphIcon(Glyph.Phone, c.onInverse, size = 24.dp)
            }
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                RText(untrusted(passkey.name), RType.display(26f), c.ink, maxLines = 1)
                RText("Passkey", RType.meta, c.inkMeta, Modifier.padding(top = 2.dp), maxLines = 1)
            }
            Row(
                Modifier
                    .testTag("removePasskey:$id")
                    .pressable(enabled = !busy, shape = CircleShape, label = "Remove", onClick = onRemove)
                    .padding(12.dp),
            ) { GlyphIcon(Glyph.Trash, c.inkMeta, size = 20.dp) }
        }
        dev.reins.android.design.PaneRow(
            "Added ${formatDate(passkey.createdAt)}",
            leading = { GlyphIcon(Glyph.Calendar, c.inkMeta, size = 18.dp) },
        )
    }
}
