package dev.reins.android.ui.trust

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.ChipMode
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.MenuItem
import dev.reins.android.design.MenuTint
import dev.reins.android.design.Motion
import dev.reins.android.design.PaneGroup
import dev.reins.android.design.PaneRow
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalMenu
import dev.reins.android.design.field
import dev.reins.android.design.fieldSurface
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.core.AutopilotMode

// The top of Trust (Reins Next, Trust 01-05): Manual or Auto as one big switch, and "For a while" for the modes that
// run for a time (Overnight, Lockdown, Bypass), behind one row and its menu.

/**
 * Manual or Auto: a sunken track with the mode in force filled like its chip (Manual in ink, Auto in violet, both with
 * the grain). [dimAuto] while Auto needs the model. Neither is filled while Lockdown holds.
 */
@Composable
fun ModeSwitch(mode: AutopilotMode, dimAuto: Boolean, onPick: (AutopilotMode) -> Unit, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        modifier
            .fillMaxWidth()
            .height(76.dp)
            .clip(CircleShape)
            .background(c.paperSunken)
            .padding(6.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        listOf(AutopilotMode.MANUAL, AutopilotMode.AUTO).forEach { m ->
            val on = mode == m
            val fill = when {
                !on -> Color.Transparent
                m == AutopilotMode.AUTO -> c.violetFill
                else -> c.inverse
            }
            val ink = when {
                !on -> c.ink2
                m == AutopilotMode.AUTO -> Color.White
                else -> c.onInverse
            }
            val shown by animateColorAsState(fill, Motion.fade(), label = "modeFill")
            Row(
                Modifier
                    .weight(1f)
                    .fillMaxHeight()
                    .graphicsLayer { alpha = if (m == AutopilotMode.AUTO && dimAuto) 0.5f else 1f }
                    .then(if (on) Modifier.fieldSurface(shown, CircleShape) else Modifier)
                    .clip(CircleShape)
                    .pressable(shape = CircleShape, label = AutopilotText.name(m)) {
                        if (!on) {
                            onPick(m)
                            feedback.play(Event.Selection)
                        }
                    }
                    .semantics(mergeDescendants = true) {
                        role = Role.RadioButton
                        selected = on
                    }
                    .testTag("mode:${m.name}"),
                horizontalArrangement = Arrangement.spacedBy(10.dp, Alignment.CenterHorizontally),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                GlyphIcon(if (m == AutopilotMode.AUTO) Glyph.AutopilotFilled else Glyph.Person, ink, size = 22.dp)
                RText(AutopilotText.name(m), RType.sans(19f, FontWeight.SemiBold), ink, maxLines = 1)
            }
        }
    }
}

/**
 * "For a while": one row with the three timed modes' marks; it opens a menu of them. Overnight and Bypass open their
 * hold sheets ([onOvernight], [onBypass]); Lockdown starts at once ([onLockdown]).
 */
@Composable
fun ForAWhile(onOvernight: () -> Unit, onLockdown: () -> Unit, onBypass: () -> Unit, modifier: Modifier = Modifier, lockdownOn: Boolean = false) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var open by remember { mutableStateOf(false) }
    PaneGroup(modifier) {
        Box {
            PaneRow(
                "For a while",
                leading = { GlyphIcon(Glyph.Timer, c.ink2, size = 20.dp) },
                trailing = {
                    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        listOf(ChipMode.Overnight, ChipMode.Lockdown, ChipMode.Bypass).forEach { m ->
                            Box(
                                Modifier.size(26.dp).clip(RoundedCornerShape(8.dp)).background((m.field() ?: Field.Grant).fill()),
                                contentAlignment = Alignment.Center,
                            ) { GlyphIcon(m.glyph ?: Glyph.Timer, Color.White, size = 15.dp) }
                        }
                    }
                },
                chevron = true,
                first = true,
                tag = "forAWhile",
            ) {
                open = true
                feedback.play(Event.Open)
            }
            SignalMenu(
                expanded = open,
                onDismiss = { open = false },
                width = 240.dp,
                items = listOf(
                    MenuItem(AutopilotText.OVERNIGHT, glyph = Glyph.Moon, tint = MenuTint.Plain, tag = "mode:OVERNIGHT") { onOvernight() },
                    MenuItem(if (lockdownOn) "Locked down" else "Lockdown", glyph = Glyph.Block, tint = MenuTint.Warn, tag = "mode:LOCKDOWN") { if (!lockdownOn) onLockdown() },
                    MenuItem("Bypass", glyph = Glyph.Bypass, tint = MenuTint.Danger, destructive = true, tag = "mode:BYPASS") { onBypass() },
                ),
            )
        }
    }
}
