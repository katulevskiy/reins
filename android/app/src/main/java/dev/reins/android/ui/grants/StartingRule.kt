package dev.reins.android.ui.grants

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.CheckRow
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Tag
import dev.reins.core.StartingPolicy

/** The words of the starting rule, kept apart so they can be tested (and match the iPhone app). */
object StartingRuleText {
    const val HEADER = "When you connect a new AI"

    fun title(policy: StartingPolicy): String = when (policy) {
        StartingPolicy.READS_FOR_A_DAY -> "Let it read for a day"
        StartingPolicy.ASK_EVERY_TIME -> "Ask me every time"
    }

    fun detail(policy: StartingPolicy): String = when (policy) {
        StartingPolicy.READS_FOR_A_DAY ->
            "Searches and reads for 24 hours. Codes and passwords still ask."
        StartingPolicy.ASK_EVERY_TIME -> "Every read asks too."
    }

    const val ALWAYS_ASKS =
        "Always asks, whatever you choose: sending, changing or deleting anything; passwords, codes and secrets; force pushes; new connections."

    /** The line on a pairing sheet: what approving the connection gives it, or null when nothing. */
    fun onPairing(policy: StartingPolicy?): String? = when (policy) {
        StartingPolicy.READS_FOR_A_DAY -> "Your starting rule lets it search and read for 24 hours without asking. Sending and changing anything still ask."
        else -> null
    }
}

/**
 * The starting rule, picked in one tap: "Let it read for a day" (recommended) or "Ask me every time". Used in the setup
 * and at the top of Grants; [selected] null means nothing was chosen yet (which asks for everything).
 */
@Composable
fun StartingRuleChooser(selected: StartingPolicy?, modifier: Modifier = Modifier, onChoose: (StartingPolicy) -> Unit) {
    val c = LocalColors.current
    Group(modifier, header = StartingRuleText.HEADER, footer = StartingRuleText.ALWAYS_ASKS) {
        listOf(StartingPolicy.READS_FOR_A_DAY, StartingPolicy.ASK_EVERY_TIME).forEachIndexed { i, policy ->
            if (i > 0) Hairline(inset = 52.dp)
            CheckRow(
                checked = selected == policy,
                onChange = { onChoose(policy) },
                tag = "rule:${policy.name}",
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    RText(StartingRuleText.title(policy), RType.sans(16f, FontWeight.SemiBold), c.text)
                    RText(StartingRuleText.detail(policy), RType.sans(13f, lineHeight = 18f), c.secondary, Modifier.padding(top = 2.dp))
                    if (policy == StartingPolicy.READS_FOR_A_DAY) Tag("Recommended", Modifier.padding(top = 6.dp), tint = c.success)
                }
            }
        }
    }
}
