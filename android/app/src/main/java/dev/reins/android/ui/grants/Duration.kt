package dev.reins.android.ui.grants

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play

/** One choice of a [DurationControl]: what it says and its test tag. */
data class DurationChoice<T>(val value: T, val label: String, val tag: String)

/**
 * "Once | 1 hour | 1 day | Custom ▾", the approval dock's control: the quick choices as a connected group, and the rest
 * behind the last segment's menu (which then shows the one picked).
 */
@Composable
fun <T> DurationControl(
    quick: List<DurationChoice<T>>,
    more: List<DurationChoice<T>>,
    selected: T,
    modifier: Modifier = Modifier,
    customTag: String = "durationCustom",
    onSelect: (T) -> Unit,
) {
    val feedback = LocalFeedback.current
    var menu by remember { mutableStateOf(false) }
    val picked = more.firstOrNull { it.value == selected }
    Row(modifier.fillMaxWidth().height(42.dp), horizontalArrangement = Arrangement.spacedBy(3.dp)) {
        quick.forEachIndexed { i, choice ->
            Segment(
                choice.label, choice.value == selected, first = i == 0, last = false, modifier = Modifier.weight(1f).testTag(choice.tag),
            ) {
                if (choice.value != selected) {
                    onSelect(choice.value)
                    feedback.play(Event.Selection)
                }
            }
        }
        if (more.isNotEmpty()) {
            Box(Modifier.weight(1.15f)) {
                Segment(picked?.label ?: "Custom", picked != null, first = quick.isEmpty(), last = true, modifier = Modifier.fillMaxWidth().testTag(customTag), chevron = true) {
                    menu = true
                }
                DropdownMenu(menu, onDismissRequest = { menu = false }) {
                    more.forEach { choice ->
                        DropdownMenuItem(
                            text = { RText(choice.label, RType.sans(15f, FontWeight.Medium), LocalColors.current.text) },
                            onClick = {
                                menu = false
                                onSelect(choice.value)
                                feedback.play(Event.Selection)
                            },
                            modifier = Modifier.testTag(choice.tag),
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun Segment(label: String, on: Boolean, first: Boolean, last: Boolean, modifier: Modifier, chevron: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    val outer = 21.dp
    val inner = 6.dp
    val shape = if (on) {
        RoundedCornerShape(outer)
    } else {
        RoundedCornerShape(
            topStart = if (first) outer else inner, bottomStart = if (first) outer else inner,
            topEnd = if (last) outer else inner, bottomEnd = if (last) outer else inner,
        )
    }
    val bg by animateColorAsState(if (on) c.accent.copy(alpha = if (c.dark) 0.24f else 0.16f) else segmentFill(c), label = "segment")
    Row(
        modifier
            .height(42.dp)
            .clip(shape)
            .background(bg)
            .pressable(shape = shape, dim = 0.7f, onClick = onClick)
            .semantics {
                selected = on
                role = Role.Tab
            },
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(label, RType.sans(14f, FontWeight.SemiBold), if (on) onAccentWash(c) else c.secondary, maxLines = 1)
        if (chevron) {
            Spacer(Modifier.width(2.dp))
            Icon(Icons.Rounded.KeyboardArrowDown, null, Modifier.size(16.dp), tint = if (on) onAccentWash(c) else c.secondary)
        }
    }
}

fun segmentFill(c: dev.reins.android.design.RColors): Color = if (c.dark) Color(0xFF1C1C20) else c.text.copy(alpha = 0.06f)

/** Text on the accent's wash: a pale violet in the dark, the accent in the light. */
fun onAccentWash(c: dev.reins.android.design.RColors): Color = if (c.dark) Color(0xFFDCD6FF) else c.accent
