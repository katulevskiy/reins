package dev.reins.android.ui.grants

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.reins.android.design.AskText
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Choice
import dev.reins.android.design.Chooser
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.pressable
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.BottomSheet
import dev.reins.core.GrantView

/** How long: the chooser's own segments, then "Custom ▾" with the rest in its menu. */
internal val LengthQuick = listOf(
    Choice(3_600L, "1 hour", "extend:HOUR"),
    Choice(86_400L, "1 day", "extend:DAY"),
    Choice(7 * 86_400L, "7 days", "extend:WEEK"),
)
internal val LengthMore = listOf(
    Choice(4 * 3_600L, "4 hours", "extend:4H"),
    Choice(30 * 86_400L, "30 days", "extend:MONTH"),
)

/** The lengths a grant may get: an ended grant to all mail comes back for a week at most. */
internal fun lengthMore(grant: GrantView, resume: Boolean): List<Choice<Long>> =
    if (resume && initialResumeDraft(grant).allMail) LengthMore.filter { it.value <= 7 * 86_400L } else LengthMore

/**
 * "Extend *for*" a running grant, or "Resume *for*" an ended one (Reins Signal, iOS-Trust 04): the chooser, its
 * Custom menu growing from the segment, then Cancel and the action with the fingerprint. [onMore] (resume only) opens
 * the full form: a custom time, a number of uses, another scope.
 */
@Composable
internal fun BoxScope.LengthSheet(grant: GrantView?, resume: Boolean, onClose: () -> Unit, onConfirm: (seconds: Long) -> Unit, onMore: (() -> Unit)? = null) {
    val c = LocalColors.current
    var seconds by remember(grant?.id, resume) { mutableLongStateOf(3_600L) }
    BottomSheet(grant != null, null, onClose = onClose, tag = if (resume) "resumeSheet" else "extendSheet") {
        val g = grant ?: return@BottomSheet
        Column(Modifier.padding(start = 22.dp, end = 22.dp, top = 4.dp, bottom = 8.dp)) {
            AskText(if (resume) "Resume for" else "Extend for", c.ink, target = "for", style = RType.serif(32f, 36f), minSize = 26.sp)
            RText(
                if (resume) "${untrusted(g.connectionLabel)} may use it again, as it was." else "From now. Uses so far stay.",
                RType.meta,
                c.inkMeta,
                Modifier.padding(top = 6.dp, bottom = 16.dp),
            )
            Chooser(LengthQuick, seconds, { seconds = it }, more = lengthMore(g, resume), moreTag = "extend:custom")
            if (onMore != null) {
                Row(
                    Modifier.fillMaxWidth().padding(top = 8.dp).pressable(shape = RoundedCornerShape(12.dp), label = "More options", onClick = onMore).testTag("resumeOptions").padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RText("More options", RType.row, c.ink, Modifier.weight(1f))
                    GlyphIcon(Glyph.ChevronRight, c.ink2, size = 16.dp)
                }
            }
            Row(Modifier.padding(top = 18.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SignalButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Outline, onClick = onClose)
                SignalButton(
                    if (resume) "Resume" else "Extend",
                    Modifier.weight(1.3f).testTag(if (resume) "confirmResume" else "confirmExtend"),
                    glyph = Glyph.Fingerprint,
                ) { onConfirm(seconds) }
            }
        }
    }
}

/** Resumes [grant] for [seconds] exactly as it was: no changed scope or uses (those are under More options). */
internal suspend fun resumeAsItWas(grant: GrantView, seconds: Long, onResume: suspend (String, Long, dev.reins.core.StandingGrant?) -> String?): String? =
    onResume(grant.id, seconds, null)
