package dev.reins.android.ui.circles

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.HelpButton
import dev.reins.android.design.HelpText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType

/**
 * A guardianship's terms (Decisions v2 P7, 04 §4), in the Add sheet when the role is Guardian: the notice the ward
 * picks before the code, 0, 3 or 7 days. Leaving ends it after this notice; adults can always leave.
 */
@Composable
internal fun GuardianTermsStep(days: Int?, onPick: (Int) -> Unit, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    var help by rememberSaveable { mutableStateOf(false) }
    Column(modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Rounded.Schedule, null, Modifier.size(18.dp), tint = c.ink2)
            RText("Notice to leave", RType.sans(15f, FontWeight.Medium), c.ink2, Modifier.weight(1f), maxLines = 1)
            HelpButton(help, "Notice to leave") { help = it }
        }
        Row(Modifier.fillMaxWidth().padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            NOTICE_DAYS.forEach { d ->
                Segment(noticeLabel(d), null, days == d, "notice:$d", Modifier.weight(1f)) { onPick(d) }
            }
        }
        days?.let {
            RText(noticeLine(it), RType.sans(14f), c.inkMeta, Modifier.padding(top = 8.dp).testTag("noticeLine"), maxLines = 1)
        }
        HelpText(
            "If you leave, it ends after this notice, and your guardian is told. 7 days at most: you can always leave.",
            help,
            Modifier.padding(top = 6.dp),
        )
    }
}

/**
 * Material Symbols `shield_person` (rounded, 400): the Guardian role. Not in the Compose icon set, so drawn from the
 * symbol's own path (its viewBox is 0 -960 960 960).
 */
val ShieldPerson: ImageVector by lazy {
    ImageVector.Builder(name = "ShieldPerson", defaultWidth = 24.dp, defaultHeight = 24.dp, viewportWidth = 960f, viewportHeight = 960f)
        .addGroup(translationY = 960f)
        .addPath(pathData = addPathNodes(SHIELD_PERSON), fill = SolidColor(Color.Black))
        .clearGroup()
        .build()
}

private const val SHIELD_PERSON =
    "M380.5-480.5Q340-521 340-580t40.5-99.5Q421-720 480-720t99.5 40.5Q620-639 620-580t-40.5 99.5Q539-440 480-440t-99.5-40.5Z" +
        "M523-537q17-17 17-43t-17-43q-17-17-43-17t-43 17q-17 17-17 43t17 43q17 17 43 17t43-17Zm-43 57Zm0-315-240 90v189q0 54 15 " +
        "105t41 96q42-21 88-33t96-12q50 0 96 12t88 33q26-45 41-96t15-105v-189l-240-90Zm-70 523q-34 8-65 22 29 30 63 52t72 34q38-12 " +
        "72-34t63-52q-31-14-65-22t-70-8q-36 0-70 8Zm57 187q-6-1-12-3-135-45-215-166.5T160-516v-189q0-25 14.5-45t37.5-29l240-90q14-5 " +
        "28-5t28 5l240 90q23 9 37.5 29t14.5 45v189q0 140-80 261.5T505-88q-6 2-12 3t-13 1q-7 0-13-1Z"
