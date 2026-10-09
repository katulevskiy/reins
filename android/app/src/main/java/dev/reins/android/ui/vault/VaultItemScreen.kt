package dev.reins.android.ui.vault

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.CircleIconButton
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
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.SecureWindow
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.copySecret
import dev.reins.core.VaultField
import dev.reins.core.VaultItemKind

private const val MASK = "••••••••••"

/** One vault item: how the desktop app refers to it, its fields (secrets after the screen lock), edit and delete. */
@Composable
fun VaultItemScreen(
    viewModel: VaultViewModel,
    id: String,
    authenticator: Authenticator,
    onBack: () -> Unit,
    onEdit: () -> Unit,
) {
    val c = LocalColors.current
    val context = LocalContext.current
    val item by viewModel.item.collectAsStateWithLifecycle()
    val revealed by viewModel.revealed.collectAsStateWithLifecycle()
    val made by viewModel.madeKey.collectAsStateWithLifecycle()
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    var deleting by remember { mutableStateOf(false) }
    var copied by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(id) { viewModel.open(id) }
    DisposableEffect(id) { onDispose { viewModel.close() } }
    SecureWindow()

    val current = item?.takeIf { it.id == id }
    Screen(
        title = current?.name?.let(::untrusted) ?: "Vault item",
        subtitle = current?.let { kindLabel(it.kind) },
        onBack = onBack,
        actions = {
            if (current != null) CircleIconButton(Glyph.Pencil, Modifier.testTag("vaultEdit"), label = "Edit", onClick = onEdit)
        },
    ) {
        ui.error?.let { Banner(untrusted(it), Modifier.padding(16.dp), BannerKind.Error, tag = "vaultItemError") }
        if (current == null) {
            RText("Loading…", RType.sans(15f), c.secondary, Modifier.padding(32.dp))
            return@Screen
        }
        if (made?.id == id) {
            Banner(
                "Made on this phone. Put the public key below on the servers and Git hosts you sign in to " +
                    "(GitHub: Settings, SSH and GPG keys). The private key stays in your vault.",
                Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                tag = "sshMade",
            )
        }
        current.warning?.let { Banner(untrusted(it), Modifier.padding(horizontal = 16.dp, vertical = 8.dp), BannerKind.Warning, tag = "vaultWarning") }
        if (current.uses.isNotEmpty()) {
            Group(
                header = "Use it from the computer",
                footer = if (current.kind == VaultItemKind.SSH_KEY) null else "Tap to copy. Your phone asks you each time one is used.",
            ) {
                current.uses.forEachIndexed { i, use ->
                    if (i > 0) Hairline()
                    UseRow(use.reference, use.hint, copyable = current.kind != VaultItemKind.SSH_KEY) {
                        copyPlain(context, use.reference)
                        copied = use.reference
                    }
                }
            }
        }
        Group(header = "Fields") {
            if (current.fields.isEmpty()) {
                RText("Nothing is filled in.", RType.sans(15f), c.secondary, Modifier.padding(16.dp))
            }
            current.fields.forEachIndexed { i, field ->
                if (i > 0) Hairline()
                FieldRow(
                    field = field,
                    shown = revealed[field.key],
                    onReveal = {
                        if (revealed.containsKey(field.key)) viewModel.hide(field.key)
                        else viewModel.reveal(authenticator, field.key, field.label, copy = false)
                    },
                    onCopy = {
                        val plain = field.value
                        if (plain != null) {
                            copyPlain(context, plain)
                            copied = field.label
                        } else {
                            viewModel.reveal(authenticator, field.key, field.label, copy = true) { value ->
                                copySecret(context, field.label, value)
                                copied = field.label
                            }
                        }
                    },
                    onShare = if (field.key == "public_key") ({ share(context, field.value.orEmpty()) }) else null,
                )
            }
        }
        copied?.let {
            RText("Copied ${untrusted(it)}.", RType.sans(13f), c.secondary, Modifier.padding(start = 32.dp, top = 8.dp).testTag("vaultCopied"))
        }
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            CapsuleButton("Delete", Modifier.fillMaxWidth().testTag("vaultDelete"), style = ButtonStyle.Destructive, enabled = !ui.busy) {
                deleting = true
            }
        }
    }

    if (deleting && current != null) {
        ConfirmDialog(
            title = "Delete ${untrusted(current.name)}?",
            text = "It is deleted from your vault for good, on every device. Anything that uses it from the computer stops working.",
            confirmLabel = "Delete",
            onConfirm = {
                deleting = false
                viewModel.delete(current.id, onBack)
            },
            onDismiss = { deleting = false },
        )
    }
}

@Composable
private fun UseRow(reference: String, hint: String, copyable: Boolean, onCopy: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .testTag("vaultUse:$reference")
            .then(if (copyable) Modifier.pressable(highlight = c.controlFill, onClick = onCopy) else Modifier)
            .padding(horizontal = 16.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            RText(untrusted(reference), RType.mono(14.5f), c.text, maxLines = 2, ltr = true)
            RText(hint, RType.sans(12.5f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 3)
        }
        if (copyable) {
            Spacer(Modifier.width(10.dp))
            GlyphIcon(Glyph.Copy, c.accent, size = 18.dp)
        }
    }
}

@Composable
private fun FieldRow(field: VaultField, shown: String?, onReveal: () -> Unit, onCopy: () -> Unit, onShare: (() -> Unit)?) {
    val c = LocalColors.current
    val value = field.value ?: shown
    // The SSH agent signs on the phone: the private key is neither shown nor copied, so it never leaves the vault.
    val kept = field.key == "private_key"
    Row(
        Modifier.fillMaxWidth().testTag("vaultField:${field.key}").padding(start = 16.dp, end = 6.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            RText(untrusted(field.label), RType.sans(12.5f, FontWeight.Medium), c.secondary)
            RText(
                when {
                    kept -> "Kept in the vault. The phone signs with it."
                    value != null -> untrusted(value)
                    else -> MASK
                },
                if (!kept && (field.multiline || field.secret)) RType.mono(14.5f) else RType.sans(16f),
                c.text,
                Modifier.padding(top = 2.dp).testTag("vaultValue:${field.key}"),
                maxLines = if (field.multiline) 12 else 3,
                ltr = true,
            )
        }
        if (kept) return@Row
        if (field.secret) {
            IconAction(if (shown != null) Glyph.Lock else Glyph.Unlock, if (shown != null) "Hide" else "Show", "vaultReveal:${field.key}", onReveal)
        }
        if (onShare != null) IconAction(Glyph.Send, "Share", "vaultShare:${field.key}", onShare)
        IconAction(Glyph.Copy, "Copy", "vaultCopy:${field.key}", onCopy)
    }
}

@Composable
private fun IconAction(glyph: Glyph, label: String, tag: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.testTag(tag).pressable(shape = CircleShape, label = label, onClick = onClick).padding(10.dp),
    ) { GlyphIcon(glyph, c.accent, size = 19.dp) }
}

/** Not a secret (a reference, a username, a public key): copied as plain text. */
private fun copyPlain(context: Context, text: String) {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("Reins", text))
}

private fun share(context: Context, text: String) {
    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
    context.startActivity(Intent.createChooser(send, "Share the public key").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}
