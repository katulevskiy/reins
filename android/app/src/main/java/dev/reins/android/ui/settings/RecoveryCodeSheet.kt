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
        Column(
            Modifier.padding(horizontal = 20.dp).fillMaxWidth().glass(c, RoundedCornerShape(26.dp), 16.dp).verticalScroll(rememberScrollState()).padding(22.dp).testTag("recoveryCodeSheet"),
        ) {
            RText("Recovery code", RType.sans(19f, FontWeight.SemiBold), c.text)
            Spacer(Modifier.height(14.dp))
            SelectionContainer {
                RText(
                    recoveryCodeLines(code).joinToString("\n"),
                    RType.mono(19f, FontWeight.Medium).copy(letterSpacing = 1.sp, lineHeight = 26.sp),
                    c.text,
                    Modifier.fillMaxWidth()
                        .background(c.controlFill, RoundedCornerShape(16.dp))
                        .clickable(onClickLabel = "Copy recovery code", onClick = onCopy)
                        .padding(vertical = 16.dp, horizontal = 12.dp)
                        .testTag("recoveryCode"),
                    ltr = true,
                )
            }
            Spacer(Modifier.height(14.dp))
            RText(
                "Opens your vault if you lose this phone. Write it down and keep it safe.",
                RType.sans(14.5f, lineHeight = 20f),
                c.secondary,
            )
            Spacer(Modifier.height(20.dp))
            if (required) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(checked = recorded, onCheckedChange = { recorded = it }, modifier = Modifier.testTag("recoveryRecorded"))
                    RText("I wrote it down", RType.sans(14.5f, lineHeight = 20f), c.text, Modifier.weight(1f))
                }
                Spacer(Modifier.height(12.dp))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                CapsuleButton("Copy", Modifier.weight(1f).testTag("copyRecoveryCode"), style = ButtonStyle.Secondary, glyph = Glyph.Copy, onClick = onCopy)
                CapsuleButton(if (required) "Continue" else "Done", Modifier.weight(1f).testTag("recoveryCodeDone"), enabled = !required || recorded, style = ButtonStyle.Primary, onClick = onDone)
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
