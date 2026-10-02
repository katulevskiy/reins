package dev.rewarden.android.ui.settings

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import dev.rewarden.android.BuildConfig
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.glass
import dev.rewarden.android.feedback.DialogFeedback

/** The recovery code in lines of four groups ("ABCD EFGH IJKL MNOP"), easier to copy out by hand than one long line. */
fun recoveryCodeLines(code: String): List<String> =
    code.split('-', ' ').filter { it.isNotEmpty() }.chunked(4).map { it.joinToString(" ") }

/** The account's recovery code, shown after biometrics, in its own window that screenshots and recents never show. */
@Composable
fun RecoveryCodeSheet(code: String, onCopy: () -> Unit, onDone: () -> Unit) {
    val c = LocalColors.current
    Dialog(
        onDismissRequest = onDone,
        properties = DialogProperties(
            usePlatformDefaultWidth = false,
            securePolicy = if (BuildConfig.SECURE_SCREENS) SecureFlagPolicy.SecureOn else SecureFlagPolicy.Inherit,
        ),
    ) {
        DialogFeedback()
        Column(
            Modifier.padding(horizontal = 20.dp).fillMaxWidth().glass(c, RoundedCornerShape(26.dp), 16.dp).padding(22.dp).testTag("recoveryCodeSheet"),
        ) {
            RText("Recovery code", RType.sans(19f, FontWeight.SemiBold), c.text)
            Spacer(Modifier.height(14.dp))
            Column(
                Modifier
                    .fillMaxWidth()
                    .background(c.controlFill, RoundedCornerShape(16.dp))
                    .padding(vertical = 16.dp, horizontal = 12.dp)
                    .testTag("recoveryCode"),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                recoveryCodeLines(code).forEach { line ->
                    RText(line, RType.mono(19f, FontWeight.Medium).copy(letterSpacing = 1.sp), c.text, maxLines = 1, ltr = true)
                }
            }
            Spacer(Modifier.height(14.dp))
            RText(
                "This code opens your account's vault if you lose this phone. Anyone with it and your sign-in can read " +
                    "your vault. Write it down and keep it somewhere safe; Reins cannot show it to you again if this phone is gone.",
                RType.sans(14.5f, lineHeight = 20f),
                c.secondary,
            )
            Spacer(Modifier.height(20.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                CapsuleButton("Copy", Modifier.weight(1f).testTag("copyRecoveryCode"), style = ButtonStyle.Secondary, glyph = Glyph.Copy, onClick = onCopy)
                CapsuleButton("Done", Modifier.weight(1f).testTag("recoveryCodeDone"), style = ButtonStyle.Primary, onClick = onDone)
            }
        }
    }
}

/** Copies [text] marked as sensitive, so Android 13 and later do not preview it on screen. */
internal fun copySecret(context: Context, label: String, text: String) {
    val clip = ClipData.newPlainText(label, text)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
        clip.description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
    }
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(clip)
}
