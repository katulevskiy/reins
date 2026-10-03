package dev.reins.android.ui.signin

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
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
import dev.reins.android.design.Screen
import dev.reins.android.design.Spinner
import dev.reins.android.ui.common.OTHER_APPROVAL_DEVICE
import dev.reins.android.ui.common.SecureWindow

/**
 * Signed in through "Continue" to an account whose keys are on another phone: ask that phone, enter the recovery code,
 * or sign out. Shown instead of the app until the keys are open (also after a relaunch). With [takeover], the server
 * refused to make this phone the approval device because another phone approves for the account: the same two ways
 * let it take over, and "Not now" goes back to the app.
 */
@Composable
fun UnlockScreen(viewModel: UnlockViewModel, email: String, takeover: Boolean = false) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    BackHandler(enabled = ui.step != UnlockStep.Choose) {
        if (ui.step == UnlockStep.Asking) viewModel.cancelAsk() else viewModel.back()
    }
    Screen(title = null, modifier = Modifier.testTag("unlock")) {
        Column(Modifier.padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Spacer(Modifier.height(64.dp))
            Box(
                Modifier.size(64.dp).background(c.accentSoft, RoundedCornerShape(20.dp)),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(Glyph.Lock, c.accent, size = 32.dp) }
            Spacer(Modifier.height(6.dp))
            RText(
                if (takeover) "Another phone approves for you" else "Your account is on another phone",
                RType.sans(28f, FontWeight.SemiBold, lineHeight = 34f),
                c.text,
            )
            RText(
                if (takeover) {
                    OTHER_APPROVAL_DEVICE
                } else {
                    "You're signed in as $email, but its vault opens on the phone you set it up with. Approve this " +
                        "phone from there, or enter your recovery code."
                },
                RType.sans(16f, lineHeight = 22f),
                c.secondary,
                Modifier.testTag(if (takeover) "takeoverText" else "unlockText"),
            )
            Spacer(Modifier.height(20.dp))
            when (ui.step) {
                UnlockStep.Choose -> Choose(viewModel, ui, takeover)
                UnlockStep.Asking -> Asking(viewModel, ui)
                UnlockStep.Recovery -> Recovery(viewModel, ui)
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun Choose(viewModel: UnlockViewModel, ui: UnlockUi, takeover: Boolean) {
    ui.ended?.let { Banner(it, kind = BannerKind.Warning, tag = "joinEnded") }
    ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "unlockError") }
    CapsuleButton(
        "Ask my other phone",
        Modifier.fillMaxWidth().testTag("askOtherPhone"),
        style = ButtonStyle.Primary,
        busy = ui.busy,
        glyph = Glyph.Phone,
        onClick = viewModel::askOtherPhone,
    )
    CapsuleButton(
        "Enter recovery code",
        Modifier.fillMaxWidth().testTag("enterRecoveryCode"),
        style = ButtonStyle.Secondary,
        enabled = !ui.busy,
        glyph = Glyph.Key,
        onClick = viewModel::showRecovery,
    )
    Row(
        Modifier.fillMaxWidth().padding(top = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterHorizontally),
    ) {
        if (takeover) TextLink("Not now", "takeoverLater", enabled = !ui.busy, onClick = viewModel::later)
        TextLink("Sign out", "unlockSignOut", enabled = !ui.busy, onClick = viewModel::signOut)
    }
}

/** The request is out: the code to compare, large, and a way back. */
@Composable
private fun Asking(viewModel: UnlockViewModel, ui: UnlockUi) {
    val c = LocalColors.current
    Column(
        Modifier
            .fillMaxWidth()
            .background(c.accent.copy(alpha = 0.09f), RoundedCornerShape(22.dp))
            .border(1.5.dp, c.accent.copy(alpha = 0.55f), RoundedCornerShape(22.dp))
            .padding(20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        RText("Check that your other phone shows", RType.sans(15f, lineHeight = 21f), c.secondary)
        RText(
            ui.code.orEmpty(),
            RType.mono(44f, FontWeight.SemiBold).copy(letterSpacing = 2.sp),
            c.text,
            Modifier.testTag("joinCode"),
            maxLines = 1,
            ltr = true,
        )
    }
    Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Spinner(c.secondary, size = 22.dp)
        Spacer(Modifier.width(12.dp))
        RText(
            "Open Reins on your other phone and approve. This screen moves on by itself.",
            RType.sans(14.5f, lineHeight = 20f),
            c.secondary,
            Modifier.weight(1f),
        )
    }
    CapsuleButton("Cancel", Modifier.fillMaxWidth().testTag("cancelJoin"), style = ButtonStyle.Secondary, onClick = viewModel::cancelAsk)
}

/** The recovery code, or the master password of an account made with one. Kept only here and in the call. */
@Composable
private fun Recovery(viewModel: UnlockViewModel, ui: UnlockUi) {
    val c = LocalColors.current
    var secret by remember { mutableStateOf("") }
    SecureWindow()
    RTextField(
        secret, { secret = it }, "Recovery code or master password", tag = "recoveryCode", enabled = !ui.busy, mono = true,
        keyboardOptions = KeyboardOptions(
            keyboardType = KeyboardType.Password,
            autoCorrectEnabled = false,
            capitalization = KeyboardCapitalization.None,
        ),
    )
    RText(
        "The code looks like ABCD-EFGH-… Spaces, dashes and capitals do not matter.",
        RType.sans(13f, lineHeight = 18f),
        c.tertiary,
        Modifier.padding(start = 4.dp),
    )
    ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "unlockError") }
    CapsuleButton(
        "Unlock",
        Modifier.fillMaxWidth().testTag("unlockAccount"),
        style = ButtonStyle.Primary,
        enabled = secret.isNotBlank(),
        busy = ui.busy,
    ) { viewModel.unlock(secret) }
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) {
        TextLink("Back", "unlockBack", enabled = !ui.busy, onClick = viewModel::back)
    }
}
