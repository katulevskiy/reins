package dev.reins.android.ui.main

import androidx.compose.ui.semantics.selected
import dev.reins.android.ui.autopilot.modeIcon
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Bolt
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Timeline
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
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.rememberNowState
import dev.reins.android.ui.autopilot.PulseDot
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
 * The bottom bar, as on iOS: the four tabs in a floating Material bar, the chosen one's filled icon on a pill of the
 * accent and its label below.
 */
@Composable
fun FloatingNavBar(
    selected: Tab,
    activityCount: Int,
    grantCount: Int,
    autopilot: AutopilotSettings?,
    onSelect: (Tab) -> Unit,
    modifier: Modifier = Modifier,
) {
    val c = LocalColors.current
    val mode = autopilot?.mode ?: AutopilotMode.MANUAL
    Row(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 14.dp, vertical = 10.dp)
            .glass(c, RoundedCornerShape(30.dp), 8.dp)
            .padding(start = 6.dp, end = 6.dp, top = 8.dp, bottom = 7.dp),
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        NavItem("Activity", Icons.Rounded.Timeline, Icons.Outlined.Timeline, selected == Tab.Activity, activityCount, c.search, "tabActivity", Modifier.weight(1f)) {
            onSelect(Tab.Activity)
        }
        NavItem("Grants", Icons.Rounded.Key, Icons.Outlined.Key, selected == Tab.Grants, grantCount, c.accent, "tabGrants", Modifier.weight(1f)) {
            onSelect(Tab.Grants)
        }
        // A bypass that runs shows on the tab: a red dot over its icon.
        NavItem(
            "Autopilot", Icons.Rounded.Bolt, Icons.Outlined.Bolt, selected == Tab.Autopilot, if (mode == AutopilotMode.BYPASS) 1 else 0, c.danger,
            "tabAutopilot", Modifier.weight(1f), dotOnly = true,
        ) { onSelect(Tab.Autopilot) }
        NavItem("Settings", Icons.Rounded.Settings, Icons.Outlined.Settings, selected == Tab.Settings, 0, c.accent, "tabSettings", Modifier.weight(1f)) {
            onSelect(Tab.Settings)
        }
    }
}

/**
 * The Autopilot mode at a glance, beside the Activity title: its icon in its colour and its name (the time left during
 * a bypass, on red). It opens the quick switcher.
 */
@Composable
fun ModePill(settings: AutopilotSettings?, onClick: () -> Unit) {
    val c = LocalColors.current
    val mode = settings?.mode ?: AutopilotMode.MANUAL
    val tint = modeTint(mode, c)
    val bypass = mode == AutopilotMode.BYPASS
    val wash by animateColorAsState(if (bypass) c.danger else tint.copy(alpha = 0.16f), label = "modeWash")
    val fg by animateColorAsState(if (bypass) Color.White else tint, label = "modeFg")
    Row(
        Modifier
            .height(HeaderControlHeight)
            .clip(CircleShape)
            .background(wash)
            .pressable(shape = CircleShape, label = "Autopilot", onClick = onClick)
            .semantics(mergeDescendants = true) { contentDescription = "Autopilot: ${AutopilotText.name(mode)}" }
            .testTag("modePill")
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val until = settings?.bypassUntil
        if (bypass && until != null) {
            PulseDot(Color.White, size = 6.dp)
            Spacer(Modifier.width(7.dp))
            BypassTime(until, fg)
        } else {
            Icon(modeIcon(mode), null, Modifier.size(18.dp), tint = fg)
            Spacer(Modifier.width(6.dp))
            RText(AutopilotText.name(mode), RType.sans(14.5f, FontWeight.SemiBold), fg, maxLines = 1)
        }
    }
}

/** The time a bypass has left. Its clock lives here, and only while a bypass runs, so the header around it stays still. */
@Composable
private fun BypassTime(until: Long, color: Color) {
    val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
    RText(AutopilotText.clock(until, now), RType.mono(13f, FontWeight.SemiBold), color, maxLines = 1)
}

/** The height of everything beside a tab's title: the pills there. */
val HeaderControlHeight = 40.dp

/** The pill beside a tab's title (Integrations, New grant): as tall as the mode pill. */
@Composable
fun HeaderPill(title: String, glyph: Glyph, tag: String, onClick: () -> Unit) {
    CapsuleButton(title, Modifier.height(HeaderControlHeight).testTag(tag), compact = true, glyph = glyph, onClick = onClick)
}

@Composable
private fun NavItem(
    label: String,
    icon: ImageVector,
    outlined: ImageVector,
    selected: Boolean,
    count: Int,
    countColor: Color,
    tag: String,
    modifier: Modifier,
    dotOnly: Boolean = false,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    val pill by animateColorAsState(if (selected) c.accent.copy(alpha = if (c.dark) 0.24f else 0.16f) else Color.Transparent, label = "nav")
    Column(
        modifier
            .clip(RoundedCornerShape(16.dp))
            .pressable(shape = RoundedCornerShape(16.dp), onClick = onClick)
            .semantics { this.selected = selected }
            .testTag(tag),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(width = 58.dp, height = 30.dp).clip(RoundedCornerShape(15.dp)).background(pill), contentAlignment = Alignment.Center) {
            Icon(if (selected) icon else outlined, null, Modifier.size(22.dp), tint = if (selected) c.text else c.secondary)
            if (dotOnly) {
                if (count > 0) Box(Modifier.align(Alignment.TopEnd).padding(top = 3.dp, end = 14.dp).size(8.dp).background(countColor, CircleShape))
            } else {
                CountPill(count, countColor, Modifier.align(Alignment.TopEnd).offset(x = (-4).dp, y = (-3).dp).testTag("$tag:count"))
            }
        }
        Spacer(Modifier.height(4.dp))
        RText(label, RType.sans(12f, if (selected) FontWeight.SemiBold else FontWeight.Medium), if (selected) c.text else c.secondary, maxLines = 1)
    }
}
