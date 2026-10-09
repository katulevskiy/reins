package dev.reins.android.ui.main

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Timeline
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.CountPill
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.rememberNowState
import dev.reins.android.ui.autopilot.PulseDot
import dev.reins.android.ui.autopilot.modeGlyph
import dev.reins.android.ui.autopilot.modeTint
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.ui.nav.Tab

/**
 * The bottom bar: a floating capsule with the two tabs, and beside it a round button in the colour of the Autopilot
 * mode, with that mode's icon (a live countdown while a bypass runs). It opens Autopilot.
 */
@Composable
fun FloatingNavBar(
    selected: Tab,
    activityCount: Int,
    grantCount: Int,
    autopilot: AutopilotSettings?,
    onSelect: (Tab) -> Unit,
    onAutopilot: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(
            Modifier.weight(1f).glass(c, RoundedCornerShape(36.dp), 8.dp).padding(6.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            NavItem("Activity", Icons.Rounded.Timeline, selected == Tab.Activity, activityCount, c.search, "tabActivity", Modifier.weight(1f)) {
                onSelect(Tab.Activity)
            }
            NavItem("Grants", Icons.Rounded.Key, selected == Tab.Grants, grantCount, c.accent, "tabGrants", Modifier.weight(1f)) {
                onSelect(Tab.Grants)
            }
        }
        ModeButton(autopilot, onAutopilot)
    }
}

/**
 * The Autopilot mode at a glance, where Settings used to be and the same size: its icon on a wash of its colour; solid red
 * with the time left during a bypass.
 */
@Composable
private fun ModeButton(settings: AutopilotSettings?, onClick: () -> Unit) {
    val c = LocalColors.current
    val mode = settings?.mode ?: AutopilotMode.MANUAL
    val tint = modeTint(mode, c)
    val bypass = mode == AutopilotMode.BYPASS
    // Every mode in its own colour, as the Autopilot screen shows it (Manual's is the quiet grey).
    val wash by animateColorAsState(if (bypass) c.danger else tint.copy(alpha = 0.16f), label = "modeWash")
    val fg by animateColorAsState(if (bypass) Color.White else tint, label = "modeFg")
    Box(
        Modifier
            .size(60.dp)
            .glass(c, CircleShape, 8.dp)
            .clip(CircleShape)
            .background(wash)
            .pressable(shape = CircleShape, label = "Autopilot", onClick = onClick)
            .semantics { contentDescription = "Autopilot: ${AutopilotText.name(mode)}" }
            .testTag("modePill"),
        contentAlignment = Alignment.Center,
    ) {
        val until = settings?.bypassUntil
        if (bypass && until != null) {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                PulseDot(Color.White, size = 6.dp)
                Spacer(Modifier.height(3.dp))
                BypassTime(until, fg)
            }
        } else {
            GlyphIcon(modeGlyph(mode), fg, size = 26.dp, weight = 2f)
        }
    }
}

/** The time a bypass has left. Its clock lives here, and only while a bypass runs, so the bar around it stays still. */
@Composable
private fun BypassTime(until: Long, color: Color) {
    val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
    RText(AutopilotText.clock(until, now), RType.mono(12.5f, FontWeight.SemiBold), color, maxLines = 1)
}

/** The round Settings button at the top right of the two tabs. */
@Composable
fun SettingsButton(onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier
            .size(40.dp)
            .clip(CircleShape)
            .background(c.controlFill.copy(alpha = if (c.dark) 0.14f else 0.08f))
            .pressable(shape = CircleShape, label = "Settings", onClick = onClick)
            .testTag("openSettings"),
        contentAlignment = Alignment.Center,
    ) { Icon(Icons.Rounded.Settings, "Settings", Modifier.size(22.dp), tint = c.text) }
}

@Composable
private fun NavItem(
    label: String,
    icon: ImageVector,
    selected: Boolean,
    count: Int,
    countColor: Color,
    tag: String,
    modifier: Modifier,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    val bg by animateColorAsState(if (selected) c.accentSoft else Color.Transparent, label = "nav")
    Box(
        modifier
            .clip(RoundedCornerShape(30.dp))
            .background(bg)
            .pressable(shape = RoundedCornerShape(30.dp), onClick = onClick)
            .testTag(tag),
        contentAlignment = Alignment.Center,
    ) {
        Column(Modifier.padding(vertical = 9.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Box {
                Icon(icon, null, Modifier.size(26.dp), tint = if (selected) c.accent else c.secondary)
                CountPill(count, countColor, Modifier.align(Alignment.TopEnd).offset(x = 12.dp, y = (-6).dp).testTag("$tag:count"))
            }
            Spacer(Modifier.height(2.dp))
            RText(label, RType.sans(12f, if (selected) FontWeight.SemiBold else FontWeight.Medium), if (selected) c.accent else c.secondary, maxLines = 1)
        }
    }
}
