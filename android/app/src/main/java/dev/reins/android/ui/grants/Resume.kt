package dev.reins.android.ui.grants

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.text.input.KeyboardType
import dev.reins.android.feedback.DialogFeedback
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.feedbackAction
import dev.reins.android.feedback.play
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.RTextField
import dev.reins.android.design.pressable
import dev.reins.core.StandingGrant
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.foundation.layout.Column
import dev.reins.android.AppContainer
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SelectChip
import dev.reins.android.design.glass
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.GrantView
import kotlin.coroutines.cancellation.CancellationException

/**
 * Resuming gives an AI access again, so it needs the same authentication as approving. Returns the reason it did not
 * happen, or null when it did (or the user changed their mind at the prompt). [standing] carries the changes made in
 * "More options"; without it the grant comes back exactly as it was.
 */
suspend fun resumeGrant(
    container: AppContainer,
    authenticator: Authenticator,
    grantId: String,
    seconds: Long,
    standing: StandingGrant? = null,
): String? = try {
    when (authenticator.authenticate("Resume grant", "")) {
        AuthResult.Success -> {
            container.feedback.play(Event.GrantCreated)
            if (standing != null) {
                container.core.resumeGrantEdited(grantId, standing)
            } else {
                container.core.resumeGrant(grantId, seconds.toULong())
            }
            container.refreshPending()
            null
        }
        AuthResult.Cancelled -> null
        AuthResult.Unavailable -> "Set a screen lock or fingerprint on this phone first.".also { container.feedback.play(Event.Error) }
    }
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    container.feedback.play(Event.Error)
    e.userMessage()
}

/** Removes an ended grant for good. Returns the reason it did not happen, or null. */
suspend fun deleteGrant(container: AppContainer, grantId: String): String? = try {
    container.feedback.play(Event.Revoked)
    container.core.deleteGrant(grantId)
    container.refreshPending()
    null
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    container.feedback.play(Event.Error)
    e.userMessage()
}

/**
 * Asks how long to resume an ended grant for. The quick choices are up front; "More options" (like on an approval)
 * holds a custom time, the number of uses, and who or what the grant covers.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ResumeDialog(grant: GrantView, onResume: (seconds: Long, standing: StandingGrant?) -> Unit, onDismiss: () -> Unit) {
    val c = LocalColors.current
    var draft by remember(grant.id) { mutableStateOf(initialResumeDraft(grant)) }
    var error by remember { mutableStateOf<String?>(null) }
    val editable = grant.editableScope != null
    val allMail = editable && draft.allMail
    val periods = resumePeriods(allMail)
    val custom = draft.customAmount.isNotBlank()
    val rotation by animateFloatAsState(if (draft.moreOpen) 90f else 0f, label = "more")
    fun edit(change: (ResumeDraft) -> ResumeDraft) {
        draft = change(draft)
        error = null
    }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DialogFeedback()
        Column(
            Modifier
                .padding(horizontal = 24.dp)
                .fillMaxWidth()
                .heightIn(max = 640.dp)
                .testTag("resumeDialog")
                .glass(c, RoundedCornerShape(26.dp), 16.dp),
        ) {
            Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()).animateContentSize().padding(22.dp)) {
                RText("Resume this grant?", RType.sans(19f, FontWeight.SemiBold), c.text)
                Spacer(Modifier.height(8.dp))
                RText(untrusted(grant.summary), RType.sans(15f, lineHeight = 21f), c.secondary)
                RText(
                    "${untrusted(grant.connectionLabel)} may use it again for:",
                    RType.sans(14f, lineHeight = 20f),
                    c.secondary,
                    Modifier.padding(top = 10.dp),
                )
                FlowRow(
                    Modifier.padding(top = 14.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    periods.forEach { p ->
                        SelectChip(p.label, !custom && draft.period == p, Modifier.testTag("period:${p.name}")) {
                            edit { it.copy(period = p, customAmount = "") }
                        }
                    }
                }

                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(top = 6.dp)
                        .testTag("resumeMore")
                        .pressable(shape = RoundedCornerShape(12.dp), onClick = feedbackAction(Event.expand(!draft.moreOpen)) { edit { it.copy(moreOpen = !it.moreOpen) } })
                        .padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RText("More options", RType.sans(15.5f, FontWeight.Medium), c.accent, Modifier.weight(1f))
                    GlyphIcon(Glyph.ChevronRight, c.accent, size = 16.dp, modifier = Modifier.rotate(rotation))
                }
                AnimatedVisibility(draft.moreOpen) {
                    Column {
                        Label("Custom time")
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                            RTextField(
                                draft.customAmount,
                                { text -> edit { it.copy(customAmount = text.filter(Char::isDigit).take(4)) } },
                                "e.g. 90",
                                Modifier.width(110.dp),
                                tag = "customAmount",
                                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                            )
                            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                CustomUnit.entries.forEach { unit ->
                                    SelectChip(unit.label, draft.customUnit == unit, Modifier.testTag("unit:${unit.name}")) {
                                        edit { it.copy(customUnit = unit) }
                                    }
                                }
                            }
                        }

                        Label("Uses")
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                            SelectChip("No limit", !draft.limitUses, Modifier.testTag("usesNone")) { edit { it.copy(limitUses = false) } }
                            SelectChip("Limit to", draft.limitUses, Modifier.testTag("usesLimit")) { edit { it.copy(limitUses = true) } }
                            if (draft.limitUses) {
                                RTextField(
                                    draft.uses,
                                    { text -> edit { it.copy(uses = text.filter(Char::isDigit).take(4)) } },
                                    "5",
                                    Modifier.width(90.dp),
                                    tag = "resumeUses",
                                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                                )
                            }
                        }

                        if (editable) {
                            val send = grant.action == "send"
                            if (!send) {
                                Label("Which emails?")
                                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    SelectChip("Specific senders", !draft.allMail, Modifier.testTag("resumeScope:specific")) {
                                        edit { it.copy(allMail = false) }
                                    }
                                    SelectChip("All mail", draft.allMail, Modifier.testTag("resumeScope:all")) {
                                        edit {
                                            val period = if (it.period == ResumePeriod.MONTH) ResumePeriod.WEEK else it.period
                                            it.copy(allMail = true, period = period)
                                        }
                                    }
                                }
                            }
                            if (!draft.allMail) {
                                Label(if (send) "Send to" else "From")
                                RTextField(
                                    draft.partiesText,
                                    { text -> edit { it.copy(partiesText = text.take(1000)) } },
                                    if (send) "a@b.com, @b.com" else "alerts@bank.com, @bank.com",
                                    tag = "resumeParties",
                                    mono = true,
                                    singleLine = false,
                                )
                                Label("Subject contains (optional)")
                                RTextField(draft.subject, { text -> edit { it.copy(subject = text.take(200)) } }, "e.g. statement", tag = "resumeSubject")
                            }
                        } else {
                            RText(
                                "Comes back as it was.",
                                RType.sans(13.5f, lineHeight = 19f),
                                c.tertiary,
                                Modifier.padding(top = 14.dp),
                            )
                        }
                        Spacer(Modifier.height(4.dp))
                    }
                }
                error?.let { Banner(it, Modifier.padding(top = 12.dp), BannerKind.Error, tag = "resumeInvalid") }
            }
            Row(Modifier.padding(start = 22.dp, end = 22.dp, bottom = 22.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                CapsuleButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Secondary, onClick = onDismiss)
                CapsuleButton("Resume", Modifier.weight(1f).testTag("confirmResume"), style = ButtonStyle.Primary) {
                    when (val built = buildResume(grant, draft)) {
                        is ResumeResult.Invalid -> error = built.message
                        is ResumeResult.Ok -> onResume(built.seconds, built.standing)
                    }
                }
            }
        }
    }
}

@Composable
private fun Label(text: String) {
    RText(text, RType.sans(13f, FontWeight.SemiBold), LocalColors.current.secondary, Modifier.padding(top = 16.dp, bottom = 8.dp))
}
