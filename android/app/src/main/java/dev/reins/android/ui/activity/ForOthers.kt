package dev.reins.android.ui.activity

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.Group
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.AppContainer
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.pane
import dev.reins.android.design.rememberNowState
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.circles.forOthers
import dev.reins.android.ui.circles.noteOrNull
import dev.reins.android.ui.circles.reviewStateLabel
import dev.reins.android.ui.circles.reviewWho
import dev.reins.android.ui.circles.shortName
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.CircleNoticeView
import dev.reins.core.CircleReviewView
import kotlinx.coroutines.launch
import kotlin.coroutines.cancellation.CancellationException

// For others (Decisions v2, 04 §6.2): requests of other people's accounts that wait for this phone, because this
// account helps them (a Circle). Under Waiting for you: by person, soonest deadline first, each card with the person on
// top, what their agent asks and the arrangement's scope, then Approve or Deny (with a note). Settled ones stay a few
// days, dimmed. What a watcher is told follows as a quiet list without buttons.

/**
 * The section, or nothing when no one waits on this phone and nothing settled lately. [onAnswer] answers one review
 * (approve, and the note a denial carries); it returns a message when it did not go through.
 */
@Composable
fun ForOthersSection(
    reviews: List<CircleReviewView>,
    notices: List<CircleNoticeView>,
    onAnswer: suspend (CircleReviewView, Boolean, String?) -> String?,
    modifier: Modifier = Modifier,
) {
    val c = LocalColors.current
    val now by rememberNowState(30_000)
    val seconds = now / 1000
    val shown = remember(reviews, seconds / 60) { forOthers(reviews, seconds) }
    if (shown.empty && notices.isEmpty()) return
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp).pane().testTag("forOthers")) {
        Row(
            Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, top = 12.dp, bottom = 4.dp).semantics { heading() },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(Icons.Rounded.Group, null, Modifier.size(16.dp), tint = c.ink2)
            LabelText("For others", c.ink2, Modifier.weight(1f))
            if (shown.count > 0) RText("${shown.count}", RType.section, c.ink2, Modifier.testTag("forOthersCount"), maxLines = 1)
        }
        shown.waiting.forEach { group ->
            PersonLine(group.person, group.soonest, seconds)
            group.reviews.forEach { review -> ReviewCard(review, seconds, onAnswer) }
        }
        shown.settled.forEach { review -> SettledRow(review, seconds) }
        if (notices.isNotEmpty()) {
            notices.sortedByDescending { it.at }.take(MAX_NOTICES).forEach { NoticeRow(it) }
        }
        Box(Modifier.heightIn(min = 6.dp))
    }
}

/** Whose requests follow: their mark, their name and when the soonest is due. */
@Composable
private fun PersonLine(person: String, soonest: Long, nowSeconds: Long) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().padding(start = 14.dp, end = 14.dp, top = 10.dp, bottom = 2.dp).testTag("forOthersPerson:$person"),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BlobAvatar(person, size = 20.dp)
        RText(shortName(untrusted(person)), RType.sans(13f, FontWeight.SemiBold), c.ink2, Modifier.weight(1f), maxLines = 1)
        Due(soonest, nowSeconds)
    }
}

/**
 * One request: the person and their agent, what it does (title, what it touches, what approving does), the agent's line
 * and the arrangement's scope, then Approve and Deny. Deny opens a note first ("Note (optional)").
 */
@Composable
private fun ReviewCard(review: CircleReviewView, nowSeconds: Long, onAnswer: suspend (CircleReviewView, Boolean, String?) -> String?) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    var denying by rememberSaveable(review.id) { mutableStateOf(false) }
    var note by rememberSaveable(review.id) { mutableStateOf("") }
    var busy by remember(review.id) { mutableStateOf(false) }
    var error by remember(review.id) { mutableStateOf<String?>(null) }
    var more by rememberSaveable(review.id) { mutableStateOf(false) }
    fun answer(approve: Boolean, text: String?) {
        if (busy) return
        busy = true
        error = null
        scope.launch {
            error = onAnswer(review, approve, text)
            busy = false
            if (error == null) denying = false
        }
    }
    Column(
        Modifier
            .fillMaxWidth()
            .ruleAbove(c, start = 14.dp)
            .padding(horizontal = 14.dp, vertical = 10.dp)
            .testTag("circleReview:${review.id}"),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            RText(untrusted(reviewWho(review)), RType.sans(12.5f, FontWeight.Medium), c.inkMeta, Modifier.weight(1f).testTag("reviewWho:${review.id}"), maxLines = 1)
            Due(review.deadline, nowSeconds, Modifier.testTag("reviewDue:${review.id}"))
        }
        RText(untrusted(review.title), RType.rowSentence, c.ink, Modifier.testTag("reviewTitle:${review.id}"), maxLines = 3)
        untrusted(review.subtitle).ifBlank { null }?.let { RText(it, RType.mono(12.5f), c.ink2, maxLines = 2) }
        untrusted(review.headline).ifBlank { null }?.let { RText(it, RType.sans(13.5f), c.ink2, maxLines = 2) }
        untrusted(review.line).ifBlank { null }?.let { RText(it, RType.sans(12.5f), c.inkMeta, maxLines = 1) }
        untrusted(review.scopeLine).ifBlank { null }?.let { line ->
            Chip(line, Icons.Rounded.Group, c.violet, Modifier.testTag("reviewScope:${review.id}"))
        }
        review.detail?.let(::untrusted)?.ifBlank { null }?.let { detail ->
            if (more) {
                RText(detail, RType.mono(12.5f), c.ink2, Modifier.testTag("reviewDetail:${review.id}"), maxLines = 12)
            } else {
                MoreButton(review.id) { more = true }
            }
        }
        error?.let { RText(it, RType.sans(13f, FontWeight.Medium), c.danger, Modifier.testTag("reviewError:${review.id}"), maxLines = 3) }
        if (denying) {
            RTextField(note, { note = it }, "Note (optional)", tag = "reviewNote:${review.id}", enabled = !busy, singleLine = false)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SignalButton(
                    "Cancel",
                    Modifier.weight(1f).testTag("reviewDenyCancel:${review.id}"),
                    style = ButtonStyle.Outline,
                    size = ButtonSize.Compact,
                    enabled = !busy,
                ) { denying = false; note = "" }
                SignalButton(
                    "Deny",
                    Modifier.weight(1f).testTag("reviewDenyConfirm:${review.id}"),
                    style = ButtonStyle.DangerWash,
                    size = ButtonSize.Compact,
                    enabled = !busy,
                    leading = { Icon(Icons.Rounded.Block, null, Modifier.size(16.dp), tint = c.danger) },
                ) { answer(false, noteOrNull(note)) }
            }
        } else {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SignalButton(
                    "Deny",
                    Modifier.weight(1f).testTag("reviewDeny:${review.id}"),
                    style = ButtonStyle.Soft,
                    size = ButtonSize.Compact,
                    enabled = !busy,
                    leading = { Icon(Icons.Rounded.Block, null, Modifier.size(16.dp), tint = c.ink) },
                ) { denying = true }
                SignalButton(
                    "Approve",
                    Modifier.weight(1f).testTag("reviewApprove:${review.id}"),
                    size = ButtonSize.Compact,
                    enabled = !busy,
                    leading = { Icon(Icons.Rounded.Fingerprint, null, Modifier.size(16.dp), tint = c.onInverse) },
                ) { answer(true, null) }
            }
        }
    }
}

/** "More": the whole card, when the arrangement shows it. */
@Composable
private fun MoreButton(id: String, onClick: () -> Unit) {
    SignalButton("More", Modifier.testTag("reviewMore:$id"), style = ButtonStyle.Soft, size = ButtonSize.Compact, onClick = onClick)
}

/** A review answered here or settled elsewhere: dimmed, its outcome in a word. */
@Composable
private fun SettledRow(review: CircleReviewView, nowSeconds: Long) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .ruleAbove(c, start = 14.dp)
            .alpha(0.55f)
            .padding(horizontal = 14.dp, vertical = 9.dp)
            .testTag("reviewSettled:${review.id}"),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BlobAvatar(review.person, size = 20.dp)
        Column(Modifier.weight(1f)) {
            RText(untrusted(review.title), RType.sans(14f, FontWeight.Medium), c.ink, maxLines = 1)
            RText(shortName(untrusted(review.person)), RType.sans(12.5f), c.inkMeta, maxLines = 1)
        }
        RText(reviewStateLabel(review, nowSeconds), RType.sans(12.5f, FontWeight.SemiBold), c.ink2, Modifier.testTag("reviewState:${review.id}"), maxLines = 1)
    }
}

/** What a watcher was told: a line, no buttons. */
@Composable
private fun NoticeRow(notice: CircleNoticeView) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .ruleAbove(c, start = 14.dp)
            .padding(horizontal = 14.dp, vertical = 9.dp)
            .testTag("circleNotice:${notice.id}"),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.Visibility, null, Modifier.size(16.dp), tint = c.inkMeta)
        Column(Modifier.weight(1f)) {
            RText(untrusted(notice.text), RType.sans(13.5f), c.ink2, maxLines = 2)
            notice.outcome?.let(::untrusted)?.ifBlank { null }?.let { RText(it, RType.sans(12.5f), c.inkMeta, maxLines = 1) }
        }
    }
}

/** "Due in 3 h", red under an hour. */
@Composable
private fun Due(deadline: Long, nowSeconds: Long, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val pressing = deadline - nowSeconds < 3_600
    val tint = if (pressing) c.danger else c.inkMeta
    Row(modifier.semantics(mergeDescendants = true) {}, horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(Icons.Rounded.Schedule, null, Modifier.size(13.dp), tint = tint)
        RText(dueIn(deadline, nowSeconds), RType.sans(12.5f, FontWeight.Medium).copy(fontFeatureSettings = "tnum"), tint, maxLines = 1)
    }
}

/** The watcher notices shown at most. */
private const val MAX_NOTICES = 5

/**
 * Answers a review: an approval asks the owner first (biometrics or the screen lock), as approving one's own does.
 * It shows settled at once; a failure puts it back and says why.
 */
suspend fun answerCircle(container: AppContainer, authenticator: Authenticator, review: CircleReviewView, approve: Boolean, note: String?): String? {
    if (approve) {
        when (authenticator.authenticate("Approve for ${shortName(untrusted(review.person))}", untrusted(review.title))) {
            AuthResult.Success -> Unit
            AuthResult.Cancelled -> return null
            AuthResult.Unavailable -> {
                container.feedback.play(Event.Error)
                return "Set a screen lock or fingerprint on this phone to approve."
            }
        }
    }
    return try {
        container.core.circleAnswer(review.id, approve, if (approve) null else note)
        container.feedback.play(if (approve) Event.Approved else Event.Denied)
        container.state.markCircleAnswered(review.id, approve)
        container.refreshCircles()
        null
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        container.feedback.play(Event.Error)
        container.refreshCircles()
        e.userMessage()
    }
}
