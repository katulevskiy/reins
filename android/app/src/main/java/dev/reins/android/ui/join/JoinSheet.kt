package dev.reins.android.ui.join

import dev.reins.android.ui.approval.AskField
import dev.reins.android.ui.approval.FactGroup
import dev.reins.android.ui.approval.FactRow
import dev.reins.android.ui.approval.AskDock
import dev.reins.android.ui.approval.AskButtons
import dev.reins.android.ui.approval.AskLoading
import dev.reins.android.ui.approval.FieldVerdict
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.draw.clip
import dev.reins.android.design.ConnectionAvatar

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.ActionKind
import dev.reins.android.design.ActionTile
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Spinner
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.quotedName

/** "482193" as "482 193": two groups of three to compare at a glance. */
internal fun spacedCode(code: String): String = if (code.length == 6 && ' ' !in code) code.take(3) + " " + code.drop(3) else code

/**
 * "Add another phone", on the approval device (Reins Signal, iOS-Connect 04): the pink accounts field, then the code
 * the new phone shows, large in the serif, and what approving gives it. [onDone] gets what to tell the user once the
 * request was answered (null after a denial); [onClose] leaves it waiting.
 */
@Composable
fun JoinSheet(viewModel: JoinViewModel, authenticator: Authenticator, onDone: (String?) -> Unit, onClose: () -> Unit = { onDone(null) }) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(ui.finished) { if (ui.finished) onDone(ui.notice) }
    val view = ui.view
    if (view == null) {
        AskLoading(ui.loading, ui.error, onClose)
        return
    }
    // The new phone names itself: Reins' own words go in the ask, its name only quoted on one short line.
    val device = quotedName(view.deviceName)
    Column(Modifier.fillMaxSize().background(c.paper)) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            AskField(
                Field.Accounts,
                "Account · New phone",
                "Add a phone to your account",
                "phone",
                onClose = onClose,
                askTag = "joinTitle",
                closeGlyph = Glyph.Close,
                closeLabel = "Close",
                top = {
                    GlyphIcon(Glyph.Phone, c.onField, size = 20.dp)
                    RText("New phone", RType.sans(14.5f, FontWeight.Medium), c.onField, Modifier.weight(1f), maxLines = 1)
                },
                verdict = null,
                // Next (Pair and join 04): the code it shows, large, in the field, and the one rule.
                below = {
                    Column(Modifier.padding(top = 14.dp)) {
                        RText(spacedCode(view.code), RType.codes.copy(fontSize = 48.sp, letterSpacing = 3.sp), c.onField, Modifier.testTag("joinCode"), maxLines = 1, ltr = true)
                        RText("Approve only if it shows this code.", RType.sans(14f, FontWeight.Medium), c.onField2, Modifier.padding(top = 6.dp), maxLines = 2)
                    }
                },
            )
            // What the phone calls itself: quoted, one line, on a row of its own, never in the ask.
            FactGroup(Modifier.padding(top = 14.dp), tag = "joinWho", rows = listOf({ FactRow("Calls itself", device, valueTag = "joinDevice") }))
            dev.reins.android.design.SectionHeader("It gets", quiet = true)
            FactGroup(
                rows = listOf(
                    { dev.reins.android.design.PaneRow("Your vault", leading = { GlyphIcon(Glyph.Vault, c.inkMeta, size = 18.dp) }, first = true, flat = true) },
                    { dev.reins.android.design.PaneRow("Approves requests", leading = { GlyphIcon(Glyph.ShieldCheck, c.inkMeta, size = 18.dp) }, first = true, flat = true) },
                ),
            )
            ui.error?.let { Banner(it, Modifier.padding(horizontal = 16.dp).padding(top = 16.dp), BannerKind.Error, tag = "joinError") }
        }
        AskDock { AskButtons("Approve", onDeny = viewModel::deny, onApprove = { viewModel.approve(authenticator) }, busy = ui.busy) }
    }
}
