package dev.reins.android.ui.grants

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.LocalColors
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.core.StartingPolicy

/** The words of the starting rule, kept apart so they can be tested (and match the iPhone app). */
object StartingRuleText {
    const val HEADER = "New AI connections"

    /** Ask every time comes first: it is the recommended one. */
    val ORDER = listOf(StartingPolicy.ASK_EVERY_TIME, StartingPolicy.HEADLINES_FIRST_HOUR)

    fun title(policy: StartingPolicy): String = when (policy) {
        StartingPolicy.ASK_EVERY_TIME -> "Ask every time"
        StartingPolicy.HEADLINES_FIRST_HOUR -> "Headlines for its first hour"
    }

    fun detail(policy: StartingPolicy): String = when (policy) {
        StartingPolicy.ASK_EVERY_TIME -> "Nothing is allowed until you say so."
        StartingPolicy.HEADLINES_FIRST_HOUR -> "Searches show sender, subject and date. Opening anything still asks."
    }

    fun recommended(policy: StartingPolicy): Boolean = policy == StartingPolicy.ASK_EVERY_TIME

    /** Behind the (?). */
    const val HELP = "What a newly connected AI may do before it asks. Codes and passwords always ask."

    /** The line on a pairing sheet: what approving the connection gives it, or null when nothing. */
    fun onPairing(policy: StartingPolicy?): String? = when (policy) {
        StartingPolicy.HEADLINES_FIRST_HOUR -> "For its first hour it sees headlines only: sender, subject and date. Opening anything still asks."
        else -> null
    }
}

/**
 * The starting rule, picked in one tap: "Ask every time" (recommended) or "Headlines for its first hour". Used in the
 * setup and at the end of Grants; [selected] null means nothing was chosen yet (which asks every time).
 */
@Composable
fun StartingRuleChooser(selected: StartingPolicy?, modifier: Modifier = Modifier, onChoose: (StartingPolicy) -> Unit) {
    var help by rememberSaveable { mutableStateOf(false) }
    SettingsGroup(modifier, header = StartingRuleText.HEADER, help = StartingRuleText.HELP, helpOpen = help, onHelp = { help = it }) {
        StartingRuleText.ORDER.forEach { policy ->
            RuleRow(policy, selected == policy) { onChoose(policy) }
        }
    }
}

@Composable
private fun RuleRow(policy: StartingPolicy, on: Boolean, onClick: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        Modifier
            .fillMaxWidth()
            .testTag("rule:${policy.name}")
            .selectable(on, role = Role.RadioButton) {
                if (!on) {
                    onClick()
                    feedback.play(Event.Selection)
                }
            }
            .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Radio(on, Modifier.padding(top = 1.dp))
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                RText(StartingRuleText.title(policy), RType.sans(16f, FontWeight.SemiBold), c.text, Modifier.weight(1f, fill = false), maxLines = 1)
                if (StartingRuleText.recommended(policy)) {
                    Spacer(Modifier.width(8.dp))
                    RText(
                        "Recommended",
                        RType.sans(11.5f, FontWeight.SemiBold),
                        c.success,
                        Modifier.background(c.success.copy(alpha = 0.13f), CircleShape).padding(horizontal = 8.dp, vertical = 2.dp),
                        maxLines = 1,
                    )
                }
            }
            RText(StartingRuleText.detail(policy), RType.sans(13f, lineHeight = 18f), c.tertiary, Modifier.padding(top = 3.dp))
        }
    }
}

/**
 * The starting rule on Trust (Signal): its label with the (?) beside it, and the two rules as hairline rows with a
 * radio. The same choice as [StartingRuleChooser], drawn like the rest of Trust.
 */
@Composable
fun StartingRuleSection(selected: StartingPolicy?, modifier: Modifier = Modifier, onChoose: (StartingPolicy) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var help by rememberSaveable { mutableStateOf(false) }
    Column(modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            dev.reins.android.design.LabelText(StartingRuleText.HEADER, c.inkMeta)
            dev.reins.android.ui.settings.HelpIcon(help, StartingRuleText.HEADER) { help = it }
        }
        dev.reins.android.design.HelpText(StartingRuleText.HELP, help, Modifier.padding(bottom = 6.dp))
        StartingRuleText.ORDER.forEach { policy ->
            val on = selected == policy
            Row(
                Modifier
                    .fillMaxWidth()
                    .hairlineBelow(c.rule)
                    .testTag("rule:${policy.name}")
                    .selectable(on, role = Role.RadioButton) {
                        if (!on) {
                            onChoose(policy)
                            feedback.play(Event.Selection)
                        }
                    }
                    .padding(vertical = 12.dp),
                verticalAlignment = Alignment.Top,
            ) {
                dev.reins.android.design.SignalRadio(on)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        RText(StartingRuleText.title(policy), RType.row, c.ink, Modifier.weight(1f, fill = false), maxLines = 1)
                        if (StartingRuleText.recommended(policy)) {
                            Spacer(Modifier.width(8.dp))
                            dev.reins.android.design.Pill("Recommended", c.success)
                        }
                    }
                    RText(StartingRuleText.detail(policy), RType.meta, c.inkMeta, Modifier.padding(top = 2.dp))
                }
            }
        }
    }
}

/** A Material radio in the accent colour. */
@Composable
fun Radio(on: Boolean, modifier: Modifier = Modifier, tint: Color = LocalColors.current.accent) {
    val c = LocalColors.current
    val ring by animateColorAsState(if (on) tint else c.tertiary, label = "radio")
    Box(modifier.size(22.dp).border(2.dp, ring, CircleShape), contentAlignment = Alignment.Center) {
        if (on) Box(Modifier.size(11.dp).background(tint, CircleShape))
    }
}
