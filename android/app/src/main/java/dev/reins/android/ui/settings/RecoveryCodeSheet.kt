package dev.reins.android.ui.settings

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
        // An account field (iOS-Arrival 04): pink because it changes how the account opens.
        val shape = RoundedCornerShape(28.dp)
        Column(
            Modifier.padding(horizontal = 20.dp).fillMaxWidth().clip(shape).background(c.paper).verticalScroll(rememberScrollState()).testTag("recoveryCodeSheet"),
        ) {
            Column(
                Modifier.fillMaxWidth().fieldSurface(Field.Accounts, RoundedCornerShape(bottomStart = 28.dp, bottomEnd = 28.dp)).padding(22.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                LabelText("Account · Recovery", c.onField2)
                AskText("Write down your recovery code", c.onField, target = "recovery code", style = RType.serif(32f, 36f), minSize = 26.sp)
                RText("It opens your vault if you lose this phone. Nobody else has it, us included.", RType.sans(14.5f, lineHeight = 20f), c.onField2)
            }
            Column(Modifier.padding(22.dp)) {
                SelectionContainer {
                    RText(
                        recoveryCodeLines(code).joinToString("\n"),
                        RType.mono(19f, FontWeight.Medium).copy(letterSpacing = 1.sp, lineHeight = 28.sp),
                        c.ink,
                        Modifier.fillMaxWidth()
                            .background(c.paperRaised, RoundedCornerShape(16.dp))
                            .border(1.dp, c.rule, RoundedCornerShape(16.dp))
                            .clickable(onClickLabel = "Copy recovery code", onClick = onCopy)
                            .padding(vertical = 16.dp, horizontal = 14.dp)
                            .testTag("recoveryCode"),
                        ltr = true,
                    )
                }
                RText("A copy clears from the clipboard in 60 s.", RType.meta, c.inkMeta, Modifier.padding(top = 8.dp))
                Spacer(Modifier.height(12.dp))
                if (required) {
                    Row(
                        Modifier.fillMaxWidth().clickable { recorded = !recorded }.testTag("recoveryRecordedRow"),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Checkbox(checked = recorded, onCheckedChange = { recorded = it }, modifier = Modifier.testTag("recoveryRecorded"))
                        RText("I wrote it down", RType.row, c.ink, Modifier.weight(1f))
                    }
                    Spacer(Modifier.height(12.dp))
                }
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    SignalButton("Copy", Modifier.weight(1f).testTag("copyRecoveryCode"), style = ButtonStyle.Soft, size = ButtonSize.Small, glyph = Glyph.Copy, onClick = onCopy)
                    SignalButton(if (required) "Continue" else "Done", Modifier.weight(1f).testTag("recoveryCodeDone"), enabled = !required || recorded, size = ButtonSize.Small, onClick = onDone)
                }
            }
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
