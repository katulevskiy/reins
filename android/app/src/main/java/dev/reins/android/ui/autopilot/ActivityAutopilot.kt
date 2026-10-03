package dev.rewarden.android.ui.autopilot

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.rewarden.android.autopilot.AutopilotText
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.ConfirmDialog
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.ui.common.untrusted
import dev.rewarden.core.ActivityEntry
import dev.rewarden.core.AutopilotMode
import dev.rewarden.core.Verdict
import kotlinx.coroutines.launch

/** What should have happened instead of what did: a denial was a mistaken approval's correction, and the reverse. */
fun correctionFor(entry: ActivityEntry): Verdict = if (entry.outcome == "denied") Verdict.APPROVE else Verdict.DENY

/**
 * An activity entry's Autopilot part: who decided (Autopilot, a bypass, Lockdown) or what Autopilot suggested, how
 * sure it was, the decisions it was like, and "This was wrong", which teaches the profile and locks the kind again.
 */
@Composable
fun ActivityAutopilotSection(entry: ActivityEntry, onCorrect: suspend (Long, Verdict) -> String?) {
    val c = LocalColors.current
    val note = entry.autopilot
    if (note == null && entry.decidedBy.isEmpty()) return
    val scope = rememberCoroutineScope()
    var asking by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var thanked by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val mode = when (entry.decidedBy) {
        "bypass" -> AutopilotMode.BYPASS
        "lockdown" -> AutopilotMode.LOCKDOWN
        else -> note?.mode ?: AutopilotMode.AUTO
    }
    val title = when (entry.decidedBy) {
        "autopilot" -> if (entry.outcome == "denied") "Denied by Autopilot" else "Approved by Autopilot"
        "bypass" -> "Approved during a bypass"
        "lockdown" -> "Denied by Lockdown"
        else -> "Autopilot suggested: ${AutopilotText.verdictWord(note?.suggested ?: Verdict.ASK).lowercase()}"
    }
    val should = correctionFor(entry)

    Group(header = "Autopilot") {
        Column(Modifier.padding(16.dp).testTag("entryAutopilot"), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconTile(modeGlyph(mode), modeTint(mode, c), size = 42.dp, filled = entry.decidedBy.isNotEmpty())
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    RText(title, RType.sans(16f, FontWeight.SemiBold), c.text, Modifier.testTag("entryAutopilotTitle"), maxLines = 2)
                    RText(
                        buildList {
                            add("In ${AutopilotText.name(note?.mode ?: mode)}")
                            note?.profileName?.takeIf { it.isNotBlank() }?.let { add("profile ${untrusted(it)}") }
                        }.joinToString(" · "),
                        RType.sans(13f),
                        c.secondary,
                        Modifier.padding(top = 2.dp),
                        maxLines = 1,
                    )
                }
            }
            if (note != null && (note.pApprove > 0f || note.pDeny > 0f)) {
                ProbabilityRow("Approve", note.pApprove, c.success)
                ProbabilityRow("Deny", note.pDeny, c.danger)
                ProbabilityRow("Confidence", note.confidence, c.accent)
            }
            note?.reason?.takeIf { it.isNotBlank() }?.let { RText(untrusted(it), RType.sans(14.5f, lineHeight = 20f), c.text) }
            if (!note?.neighbours.isNullOrEmpty()) {
                Column {
                    Caption("Like these decisions of yours", Modifier.padding(bottom = 6.dp))
                    note!!.neighbours.forEach { line ->
                        Row(Modifier.padding(vertical = 3.dp), verticalAlignment = Alignment.Top) {
                            GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 12.dp, modifier = Modifier.padding(top = 3.dp))
                            Spacer(Modifier.width(6.dp))
                            RText(untrusted(line), RType.sans(13.5f), c.secondary, maxLines = 2)
                        }
                    }
                }
            }
            if (thanked) {
                Banner("Thanks. Autopilot learned from this and asks you about requests like it again.", tag = "corrected")
            } else if (note?.correctable == true) {
                CapsuleButton(
                    "This was wrong",
                    Modifier.fillMaxWidth().testTag("thisWasWrong"),
                    style = ButtonStyle.Secondary,
                    glyph = Glyph.Flag,
                    busy = busy,
                ) { asking = true }
            }
            error?.let { Banner(it, kind = BannerKind.Error) }
        }
    }

    if (asking) {
        ConfirmDialog(
            title = if (should == Verdict.DENY) "Should this have been denied?" else "Should this have been approved?",
            text = (if (should == Verdict.DENY) "Autopilot remembers it as a denial" else "Autopilot remembers it as an approval") +
                ", more strongly than an ordinary answer, and asks you about this kind of request again until it has learned more. What was done stays done.",
            confirmLabel = if (should == Verdict.DENY) "Deny next time" else "Approve next time",
            destructive = false,
            onConfirm = {
                asking = false
                busy = true
                scope.launch {
                    error = onCorrect(entry.id, should)
                    busy = false
                    thanked = error == null
                }
            },
            onDismiss = { asking = false },
        )
    }
}
