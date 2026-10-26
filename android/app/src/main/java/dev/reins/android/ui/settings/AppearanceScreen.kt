package dev.reins.android.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Appearance
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.MenuItem
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalMenu
import dev.reins.android.design.ThemeChoice
import dev.reins.android.design.Toggle
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.main.HeaderInsets

/**
 * Settings › Appearance (Reins Signal, iOS-Settings 04): paper that follows the system, or Paper or Graphite; the grain
 * on fields; the sans you read in (Instrument or Geist). Changes apply at once, no restart. Kept on this phone.
 */
@Composable
fun AppearanceScreen(onBack: () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val feedback = LocalFeedback.current
    val choice by Appearance.state(context)
    Column(Modifier.fillMaxSize().background(c.paper).verticalScroll(rememberScrollState()).testTag("appearance")) {
        Box(Modifier.windowInsetsPadding(HeaderInsets).padding(start = 24.dp, top = 8.dp)) {
            dev.reins.android.design.BackArrow(onBack, Modifier)
        }
        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(top = 14.dp)) {
            RText("Appearance", RType.title, c.ink, maxLines = 1)
            Row(Modifier.fillMaxWidth().padding(top = 20.dp), horizontalArrangement = Arrangement.SpaceBetween) {
                ThemeChoice.entries.forEach { theme ->
                    val on = choice.theme == theme
                    Column(
                        Modifier
                            .clip(RoundedCornerShape(20.dp))
                            .pressable(shape = RoundedCornerShape(20.dp), label = theme.label) {
                                if (!on) {
                                    Appearance.set(context, choice.copy(theme = theme))
                                    feedback.play(Event.Selection)
                                }
                            }
                            .semantics {
                                role = Role.RadioButton
                                selected = on
                            }
                            .testTag("theme:${theme.name}"),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        ThemeMini(theme, on)
                        RText(theme.label, RType.sans(13.5f, if (on) FontWeight.SemiBold else FontWeight.Normal), if (on) c.ink else c.ink2, maxLines = 1)
                    }
                }
            }
            Row(
                Modifier
                    .padding(top = 16.dp)
                    .fillMaxWidth()
                    .heightIn(min = 56.dp)
                    .border(0.dp, Color.Transparent)
                    .hairlineBelow(c.rule)
                    .padding(vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    RText("Grain on fields", RType.row, c.ink, maxLines = 1)
                    RText("A painted finish on requests", RType.meta, c.inkMeta, maxLines = 1)
                }
                Toggle(choice.grain, Modifier.testTag("grainSwitch")) { Appearance.set(context, choice.copy(grain = it)) }
            }
        }
    }
}

/** A small phone in [theme]: an orange field on top and two lines of paper (94 × 150, radius 18); chosen, a violet ring. */
@Composable
private fun ThemeMini(theme: ThemeChoice, on: Boolean) {
    val c = LocalColors.current
    val dark = theme.dark(androidx.compose.foundation.isSystemInDarkTheme())
    val paper = if (dark) Color(0xFF121215) else Color(0xFFF6F6F8)
    val line = if (dark) Color(0xFF2A2A30) else Color(0xFFE3E3EA)
    val shape = RoundedCornerShape(18.dp)
    Box(
        Modifier
            .size(94.dp, 150.dp)
            .border(if (on) 2.dp else 1.dp, if (on) c.violet else c.rule, shape)
            .padding(if (on) 2.dp else 1.dp)
            .clip(RoundedCornerShape(16.dp))
            .background(paper),
    ) {
        Box(Modifier.fillMaxWidth().height(52.dp).background(Color(0xFFC2410C), RoundedCornerShape(bottomStart = 10.dp, bottomEnd = 10.dp)))
        Box(Modifier.padding(start = 10.dp, end = 10.dp, top = 64.dp).fillMaxWidth().height(6.dp).background(line, RoundedCornerShape(3.dp)))
        Box(Modifier.padding(start = 10.dp, end = 30.dp, top = 78.dp).fillMaxWidth().height(6.dp).background(line, RoundedCornerShape(3.dp)))
    }
}
