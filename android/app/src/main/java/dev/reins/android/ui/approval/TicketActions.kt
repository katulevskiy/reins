package dev.reins.android.ui.approval

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.EditNote
import androidx.compose.material.icons.rounded.QuestionAnswer
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RTextField
import dev.reins.android.design.SignalButton
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.core.PendingItem

// Decisions v2 on the approval screen (03 §5): Later (an Instant request becomes a ticket and the agent carries on),
// Why? (a question back to the agent), and a note the agent receives with a denial.

/** Whether the screen offers Later: an Instant request the server can park. */
internal fun offersLater(item: PendingItem?): Boolean = item != null && item.canDefer && item.ticket == null

/** Whether it offers Why?: the question waits on a ticket, so the request is one or can become one. */
internal fun offersWhy(item: PendingItem?): Boolean = item != null && (item.ticket != null || item.canDefer)

/** Later · Why? · Deny with note, small, above Deny and the action. Deny itself stays one tap. */
@Composable
internal fun TicketActions(item: PendingItem?, busy: Boolean, viewModel: ApprovalViewModel) {
    val nav = LocalApprovalNav.current
    val feedback = LocalFeedback.current
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (offersLater(item)) {
            SmallAction("Later", Icons.Rounded.Schedule, "later", busy, onClick = viewModel::decideLater)
        }
        if (offersWhy(item)) {
            SmallAction("Why?", Icons.Rounded.QuestionAnswer, "askWhy", busy) {
                feedback.play(Event.Open)
                nav.overlay = ApprovalOverlay.Why
            }
        }
        SmallAction("Deny with note", Icons.Rounded.EditNote, "denyWithNote", busy) {
            feedback.play(Event.Open)
            nav.overlay = ApprovalOverlay.DenyNote
        }
    }
}

@Composable
private fun SmallAction(title: String, icon: ImageVector, tag: String, busy: Boolean, onClick: () -> Unit) {
    val c = LocalColors.current
    SignalButton(
        title,
        Modifier.testTag(tag),
        style = ButtonStyle.Soft,
        size = ButtonSize.Compact,
        enabled = !busy,
        leading = { Icon(icon, null, Modifier.size(16.dp), tint = c.ink) },
        onClick = onClick,
    )
}

/** Deny, with a note the agent receives. Empty, it is a plain denial. */
@Composable
internal fun DenyNoteSheet(viewModel: ApprovalViewModel, onClose: () -> Unit) {
    var note by rememberSaveable { mutableStateOf("") }
    SheetHead("Deny", onClose)
    Column(Modifier.padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        RTextField(
            note,
            { note = it.take(NOTE_MAX) },
            "Note to the agent (optional)",
            Modifier.heightIn(min = 96.dp),
            tag = "denyNoteField",
            singleLine = false,
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
        )
        val c = LocalColors.current
        SignalButton(
            "Deny",
            Modifier.fillMaxWidth().testTag("denyNoteConfirm"),
            leading = { Icon(Icons.Rounded.Block, null, Modifier.size(20.dp), tint = c.onInverse) },
        ) { viewModel.deny(note) }
    }
}

/** Why?: a short question to the agent. Its answer shows under "Waiting for you", as the agent's words. */
@Composable
internal fun WhySheet(viewModel: ApprovalViewModel, onClose: () -> Unit) {
    var question by rememberSaveable { mutableStateOf("") }
    SheetHead("Why?", onClose)
    Column(Modifier.padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        RTextField(
            question,
            { question = it.take(ApprovalViewModel.WHY_MAX) },
            "Ask the agent",
            Modifier.heightIn(min = 96.dp),
            tag = "whyField",
            singleLine = false,
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
        )
        SignalButton("Ask", Modifier.fillMaxWidth().testTag("whySend"), enabled = question.isNotBlank()) { viewModel.askWhy(question) }
    }
}

/** A note is a line or two, not a letter. */
private const val NOTE_MAX = 500
