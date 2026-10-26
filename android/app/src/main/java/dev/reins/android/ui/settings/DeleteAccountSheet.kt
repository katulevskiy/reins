package dev.reins.android.ui.settings

import androidx.compose.material.icons.rounded.Delete
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.background
import androidx.compose.ui.unit.sp
import dev.reins.android.design.AskText
import dev.reins.android.design.Field
import dev.reins.android.design.LabelText
import dev.reins.android.design.fieldSurface
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
    Glyph.Vault to "Your account and its vault on the server",
    Glyph.Link to "Your AI and computer connections, and their access",
    Glyph.Shield to "Your approval phone, permissions, activity and files",
    Glyph.Mail to "Your sign-in: this email would start a new, empty account",
    Glyph.Phone to "Everything Reins keeps for this account on this phone",
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
        // A red field: what goes can't come back, and it is held, never tapped.
        val shape = RoundedCornerShape(28.dp)
        Column(
            Modifier.padding(horizontal = 20.dp).fillMaxWidth().clip(shape).background(c.paper)
                .verticalScroll(rememberScrollState()).testTag("deleteAccountSheet"),
        ) {
            // Settings 06: the red field names the account; what goes is listed; the email typed; then a hold.
            Box(Modifier.fillMaxWidth().fieldSurface(Field.Delete, RoundedCornerShape(28.dp)).padding(start = 22.dp, end = 14.dp, top = 18.dp, bottom = 20.dp)) {
                Column(Modifier.padding(end = 44.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    RText("Delete", RType.display(34f), c.onField, maxLines = 1)
                    RText(email, RType.display(30f), c.onField, maxLines = 2, ltr = true)
                    RText("Can't be undone.", RType.sans(14f, FontWeight.Medium), c.onField2, Modifier.padding(top = 6.dp))
                }
                dev.reins.android.design.RoundIconButton(
                    Glyph.Close,
                    "Cancel",
                    Modifier.align(Alignment.TopEnd).testTag("cancelDeleteAccount"),
                    onField = true,
                    size = 36.dp,
                    onClick = { if (!ui.busy) onDismiss() },
                )
            }
            dev.reins.android.design.SectionHeader("Goes for good", quiet = true)
            dev.reins.android.design.FlatGroup {
                deletionItems.forEachIndexed { i, (glyph, item) ->
                    dev.reins.android.design.PaneRow(item, leading = { dev.reins.android.design.RowTile(glyph) }, first = i == 0, flat = true)
                }
            }
            dev.reins.android.design.SectionHeader("Type your email", quiet = true)
            Column(Modifier.padding(start = 20.dp, end = 20.dp, bottom = 22.dp)) {
                RTextField(
                    typed, { typed = it }, email, tag = "deleteAccountEmail", enabled = !ui.busy,
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Email,
                        autoCorrectEnabled = false,
                        capitalization = KeyboardCapitalization.None,
                    ),
                )
                ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "deleteAccountError") }
                Spacer(Modifier.height(18.dp))
                dev.reins.android.design.HoldButton(
                    "Hold to delete",
                    c.danger,
                    Modifier.fillMaxWidth().testTag("confirmDeleteAccount"),
                    icon = androidx.compose.material.icons.Icons.Rounded.Delete,
                    enabled = deletionConfirmed(typed, email) && !ui.busy,
                    busy = ui.busy,
                ) { onDelete(typed) }
            }
        }
    }
}
