package dev.reins.android.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.glass
import dev.reins.android.feedback.DialogFeedback

/** What "Delete account" deletes, line by line, before it asks for the email. */
internal val deletionItems = listOf(
    "Your account and its vault on the server",
    "Your AI and computer connections, and their access",
    "Your approval phone, permissions, activity and files",
    "Your sign-in: this email would start a new, empty account",
    "Everything Reins keeps for this account on this phone",
)

/** Whether [typed] names the account's [email] (case and surrounding spaces do not count), as the server checks it. */
internal fun deletionConfirmed(typed: String, email: String): Boolean =
    typed.isNotBlank() && typed.trim().equals(email.trim(), ignoreCase = true)

/**
 * Settings > Session > Delete account: what goes, that it cannot be undone, and the account's email typed to confirm.
 * A refusal (a typo, another phone approves, no network) shows here and changes nothing.
 */
@Composable
fun DeleteAccountSheet(email: String, ui: DeleteAccountUi, onDelete: (String) -> Unit, onDismiss: () -> Unit) {
    val c = LocalColors.current
    var typed by remember { mutableStateOf("") }
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false, dismissOnBackPress = !ui.busy, dismissOnClickOutside = !ui.busy),
    ) {
        DialogFeedback()
        Column(
            Modifier.padding(horizontal = 20.dp).fillMaxWidth().glass(c, RoundedCornerShape(26.dp), 16.dp)
                .verticalScroll(rememberScrollState()).padding(22.dp).testTag("deleteAccountSheet"),
        ) {
            RText("Delete account", RType.sans(19f, FontWeight.SemiBold), c.text)
            Spacer(Modifier.height(8.dp))
            RText("Gone for good. Can't be undone.", RType.sans(15f, lineHeight = 21f), c.secondary)
            Spacer(Modifier.height(14.dp))
            deletionItems.forEach { item ->
                Row(Modifier.padding(vertical = 4.dp), verticalAlignment = Alignment.Top) {
                    GlyphIcon(Glyph.Close, c.danger, size = 16.dp)
                    Spacer(Modifier.width(10.dp))
                    RText(item, RType.sans(14.5f, lineHeight = 20f), c.text)
                }
            }
            Spacer(Modifier.height(14.dp))
            RText("Type $email to confirm.", RType.sans(14.5f, lineHeight = 20f), c.secondary)
            Spacer(Modifier.height(8.dp))
            RTextField(
                typed, { typed = it }, "Your email", tag = "deleteAccountEmail", enabled = !ui.busy,
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Email,
                    autoCorrectEnabled = false,
                    capitalization = KeyboardCapitalization.None,
                ),
            )
            ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "deleteAccountError") }
            Spacer(Modifier.height(18.dp))
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                CapsuleButton(
                    "Delete account",
                    Modifier.fillMaxWidth().testTag("confirmDeleteAccount"),
                    style = ButtonStyle.Destructive,
                    enabled = deletionConfirmed(typed, email) && !ui.busy,
                    busy = ui.busy,
                    glyph = Glyph.Trash,
                ) { onDelete(typed) }
                CapsuleButton(
                    "Cancel",
                    Modifier.fillMaxWidth().testTag("cancelDeleteAccount"),
                    style = ButtonStyle.Secondary,
                    enabled = !ui.busy,
                    onClick = onDismiss,
                )
            }
        }
    }
}
