package dev.reins.android.ui.activity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.AppContainer
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalButton
import dev.reins.android.design.fieldSurface
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.OwnerPrompt
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.AutoQuestion
import dev.reins.core.FactChoice
import dev.reins.core.ZoneChoice
import kotlin.coroutines.cancellation.CancellationException

// "Auto needs information" (Decisions v2, 01 §3): a party a request named that Auto doesn't know, as a card in the
// waiting cards' shell under them. Never a notification: the cards are only listed here.

/** The four zones in the order the cards and the setup show them, least trusted last. */
val Zones = listOf(ZoneChoice.MINE, ZoneChoice.FRIENDS, ZoneChoice.WORK, ZoneChoice.OUTSIDE)

fun zoneName(zone: ZoneChoice): String = when (zone) {
    ZoneChoice.MINE -> "Mine"
    ZoneChoice.FRIENDS -> "Friends"
    ZoneChoice.WORK -> "Work"
    ZoneChoice.OUTSIDE -> "Outside"
}

fun zoneGlyph(zone: ZoneChoice): Glyph = when (zone) {
    ZoneChoice.MINE -> Glyph.Person
    ZoneChoice.FRIENDS -> Glyph.People
    ZoneChoice.WORK -> Glyph.Briefcase
    ZoneChoice.OUTSIDE -> Glyph.Globe
}

/** "Who is octo on GitHub?" for an owner, else "Who is bob@acme.io?". */
fun questionTitle(q: AutoQuestion): String {
    val key = untrusted(q.key)
    return if (q.kind == FactChoice.OWNER) "Who is $key on GitHub?" else "Who is $key?"
}

/** The open questions, one card each, newest first. */
@Composable
fun QuestionCards(
    questions: List<AutoQuestion>,
    onAnswer: (AutoQuestion, ZoneChoice) -> Unit,
    onDismiss: (AutoQuestion) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (questions.isEmpty()) return
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("questionCards"), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        questions.forEach { q -> QuestionCard(q, { onAnswer(q, it) }, { onDismiss(q) }) }
    }
}

/**
 * One question in the waiting cards' shell: the label line (Auto's mark, "Needs info", a dismiss), the question in
 * the serif, the AI whose request named it, and the four zones (the proposed one white).
 */
@Composable
private fun QuestionCard(q: AutoQuestion, onAnswer: (ZoneChoice) -> Unit, onDismiss: () -> Unit) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(26.dp)
    Column(
        Modifier
            .fillMaxWidth()
            .fieldSurface(Field.Grant, shape)
            .clip(shape)
            .testTag("question:${q.id}")
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            GlyphIcon(Glyph.Autopilot, c.onField, size = 18.dp)
            RText("Auto needs info", RType.sans(14f, FontWeight.SemiBold), c.onField, Modifier.weight(1f), maxLines = 1)
            RoundIconButton(Glyph.Close, "Dismiss", Modifier.testTag("dismissQuestion:${q.id}"), onField = true, size = 32.dp, onClick = onDismiss)
        }
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            RText(questionTitle(q), RType.askSmall, c.onField, Modifier.semantics { heading() }.testTag("questionTitle:${q.id}"), maxLines = 3)
            untrusted(q.connectionLabel).takeIf { it.isNotBlank() }?.let {
                RText(it, RType.sans(13.5f), c.onField2, Modifier.testTag("questionWho:${q.id}"), maxLines = 1)
            }
        }
        Zones.chunked(2).forEach { pair ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                pair.forEach { zone ->
                    SignalButton(
                        zoneName(zone),
                        Modifier.weight(1f).testTag("answer:${q.id}:${zone.name}"),
                        style = if (zone == q.proposed) ButtonStyle.OnFieldWhite else ButtonStyle.OnFieldWash,
                        size = ButtonSize.Small,
                        glyph = zoneGlyph(zone),
                    ) { onAnswer(zone) }
                }
            }
        }
    }
}

/**
 * Answers [q] with [zone]: anything but Outside loosens, so the core asks the owner first ([OwnerPrompt]). The card
 * leaves at once and comes back when the answer did not land. Returns what to say, or null.
 */
suspend fun answerQuestion(container: AppContainer, authenticator: Authenticator, q: AutoQuestion, zone: ZoneChoice): String? {
    val owner = OwnerPrompt(authenticator, "Auto")
    container.state.removeAutoQuestion(q.id)
    return try {
        container.core.answerAutoQuestion(q.id, zone, owner)
        container.feedback.play(Event.Selection)
        null
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        when (owner.last) {
            AuthResult.Cancelled -> null
            AuthResult.Unavailable -> "Set a screen lock or fingerprint on this phone first."
            else -> {
                container.feedback.play(Event.Error)
                e.userMessage()
            }
        }
    } finally {
        rereadQuestions(container)
    }
}

/** Dismisses [q]: it stays Outside and isn't asked about again. */
suspend fun dismissQuestion(container: AppContainer, q: AutoQuestion): String? {
    container.state.removeAutoQuestion(q.id)
    return try {
        container.core.dismissAutoQuestion(q.id)
        null
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        container.feedback.play(Event.Error)
        e.userMessage()
    } finally {
        rereadQuestions(container)
    }
}

private suspend fun rereadQuestions(container: AppContainer) {
    try {
        container.state.setAutoQuestions(container.core.autoQuestions())
    } catch (e: CancellationException) {
        throw e
    } catch (_: Exception) {
    }
}
