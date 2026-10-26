package dev.reins.android.ui.settings

import androidx.compose.ui.semantics.text
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.fillMaxSize
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.material3.Checkbox
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import dev.reins.android.BuildConfig
import androidx.compose.foundation.border
import androidx.compose.ui.draw.clip
import dev.reins.android.design.AskText
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.Field
import dev.reins.android.design.LabelText
import dev.reins.android.design.SignalButton
import dev.reins.android.design.fieldSurface
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.glass
import dev.reins.android.feedback.DialogFeedback

/** The recovery code in lines of four groups ("ABCD EFGH IJKL MNOP"), easier to copy out by hand than one long line. */
fun recoveryCodeLines(code: String): List<String> =
    code.split('-', ' ').filter { it.isNotEmpty() }.chunked(4).map { it.joinToString(" ") }

/** The account's recovery code, shown after biometrics, in its own window that screenshots and recents never show. */
@Composable
fun RecoveryCodeSheet(code: String, onCopy: () -> Unit, onDone: () -> Unit, required: Boolean = false) {
    val c = LocalColors.current
    // An acknowledgement is deliberately not saved across process restarts.
    var recorded by remember(code) { mutableStateOf(false) }
    Dialog(
        onDismissRequest = { if (!required) onDone() },
        properties = DialogProperties(
            usePlatformDefaultWidth = false,
            dismissOnBackPress = !required,
            dismissOnClickOutside = !required,
            securePolicy = if (BuildConfig.SECURE_SCREENS) SecureFlagPolicy.SecureOn else SecureFlagPolicy.Inherit,
        ),
    ) {
        DialogFeedback()
        // Next (Arrive 04): a page of its own. The account field (pink: it changes how the account opens) with the
        // title; the code as twelve numbered groups; Copy; "I wrote it down"; Continue at the foot.
        Column(Modifier.fillMaxSize().background(c.paper).testTag("recoveryCodeSheet")) {
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                dev.reins.android.design.FieldHeader(Field.Accounts) {
                    Row(Modifier.padding(top = 20.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        androidx.compose.foundation.layout.Box(Modifier.size(40.dp).clip(androidx.compose.foundation.shape.CircleShape).background(c.onFieldWash), contentAlignment = Alignment.Center) {
                            dev.reins.android.design.GlyphIcon(Glyph.FileLock, c.onField, size = 18.dp)
                        }
                        RText("Recovery code", RType.display(40f), c.onField, Modifier.weight(1f), maxLines = 2)
                    }
                    RText("The only way back in if you lose this phone.", RType.sans(15f, lineHeight = 21f), c.onField2)
                }
                val groups = code.split('-', ' ').filter { it.isNotEmpty() }
                Column(
                    Modifier
                        .padding(start = 16.dp, end = 16.dp, top = 16.dp)
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(20.dp))
                        .background(c.paperRaised)
                        .clickable(onClickLabel = "Copy recovery code", onClick = onCopy)
                        .semantics(mergeDescendants = true) { text = androidx.compose.ui.text.AnnotatedString(recoveryCodeLines(code).joinToString("\n")) }
                        .testTag("recoveryCode")
                        .padding(vertical = 12.dp, horizontal = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    groups.chunked(3).forEachIndexed { row, three ->
                        Row(Modifier.fillMaxWidth()) {
                            three.forEachIndexed { i, g ->
                                Row(Modifier.weight(1f), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally)) {
                                    RText("${row * 3 + i + 1}", RType.mono(10f), c.inkFaint, maxLines = 1)
                                    RText(g, RType.mono(19f, FontWeight.Medium), c.ink, maxLines = 1, ltr = true)
                                }
                            }
                            repeat(3 - three.size) { Spacer(Modifier.weight(1f)) }
                        }
                    }
                }
                SignalButton(
                    "Copy",
                    Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp).fillMaxWidth().testTag("copyRecoveryCode"),
                    style = ButtonStyle.Soft,
                    size = ButtonSize.Small,
                    glyph = Glyph.Copy,
                    onClick = onCopy,
                )
                RText("A copy clears from the clipboard in 60 s.", RType.meta, c.inkMeta, Modifier.padding(start = 24.dp, top = 8.dp))
                if (required) {
                    Row(
                        Modifier.padding(horizontal = 16.dp).padding(top = 12.dp).fillMaxWidth().clip(RoundedCornerShape(20.dp)).background(c.paperRaised)
                            .clickable { recorded = !recorded }.testTag("recoveryRecordedRow").padding(horizontal = 14.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        dev.reins.android.design.RowTile(Glyph.Pencil)
                        RText("I wrote it down", RType.row, c.ink, Modifier.weight(1f))
                        dev.reins.android.design.Toggle(recorded, Modifier.testTag("recoveryRecorded")) { recorded = it }
                    }
                }
            }
            SignalButton(
                if (required) "Continue" else "Done",
                Modifier.navigationBarsPadding().padding(16.dp).fillMaxWidth().testTag("recoveryCodeDone"),
                enabled = !required || recorded,
                onClick = onDone,
            )
        }
    }
}

/** Copies [text] marked as sensitive, so Android 13 and later do not preview it on screen, and clears it after a minute. */
internal fun copySecret(context: Context, label: String, text: String) {
    val clip = ClipData.newPlainText(label, text)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        clip.description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
    }
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return
    clipboard.setPrimaryClip(clip)
    // Gone from the clipboard after a minute. (Android 10 and later do not let an app in the background read the
    // clipboard to check it is still this, so it is cleared whatever it holds then.)
    android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({
        runCatching { clipboard.clearPrimaryClip() }
    }, SECRET_CLIP_MILLIS)
}

/** How long a copied secret stays on the clipboard. */
internal const val SECRET_CLIP_MILLIS = 60_000L
