package dev.reins.android.ui.main

import androidx.compose.ui.semantics.selected
import dev.reins.android.ui.autopilot.modeIcon
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentSize
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
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.runtime.setValue
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
        // A bypass or an Overnight that runs shows on the tab: a dot in its colour over its icon.
        NavItem(
            "Autopilot", Icons.Rounded.Bolt, Icons.Outlined.Bolt, selected == Tab.Autopilot, if (AutopilotText.isTimed(mode)) 1 else 0,
            if (mode == AutopilotMode.OVERNIGHT) c.night else c.danger,
            "tabAutopilot", Modifier.weight(1f), dotOnly = true,
        ) { onSelect(Tab.Autopilot) }
        NavItem("Settings", Icons.Rounded.Settings, Icons.Outlined.Settings, selected == Tab.Settings, 0, c.accent, "tabSettings", Modifier.weight(1f)) {
            onSelect(Tab.Settings)
        }
    }
}

/** Where the header's mode pill is (root coordinates): the quick menu drops from under it. */
object ModePillAnchor {
    var bounds by androidx.compose.runtime.mutableStateOf<androidx.compose.ui.geometry.Rect?>(null)
}

/**
 * The Autopilot mode at a glance, beside the Activity title: its icon in its colour and its name (the time left during
 * a bypass, on red, or an Overnight, on night indigo). It opens the quick switcher.
 */
@Composable
fun ModePill(settings: AutopilotSettings?, onClick: () -> Unit) {
    val c = LocalColors.current
    val mode = settings?.mode ?: AutopilotMode.MANUAL
    val tint = modeTint(mode, c)
    val timed = AutopilotText.isTimed(mode)
    val wash by animateColorAsState(if (timed) tint else tint.copy(alpha = 0.16f), label = "modeWash")
    val fg by animateColorAsState(
        when {
            mode == AutopilotMode.OVERNIGHT -> dev.reins.android.ui.autopilot.onTimed(dev.reins.android.ui.autopilot.TimedKind.Overnight, c)
            timed -> Color.White
            else -> tint
        },
        label = "modeFg",
    )
    Row(
        Modifier
            .height(HeaderControlHeight)
            .onGloballyPositioned { ModePillAnchor.bounds = it.boundsInRoot() }
            .clip(CircleShape)
            .background(wash)
            .pressable(shape = CircleShape, label = "Autopilot", onClick = onClick)
            .semantics(mergeDescendants = true) { contentDescription = "Autopilot: ${AutopilotText.name(mode)}" }
            .testTag("modePill")
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val until = when (mode) {
            AutopilotMode.BYPASS -> settings?.bypassUntil
            AutopilotMode.OVERNIGHT -> settings?.overnightUntil
            else -> null
        }
        if (timed && until != null) {
            PulseDot(fg, size = 6.dp)
            Spacer(Modifier.width(7.dp))
            // Without an end there is nothing to count down: it reads as the mode.
            if (AutopilotText.isUntilOff(until)) RText(AutopilotText.name(mode), RType.sans(14.5f, FontWeight.SemiBold), fg, maxLines = 1) else BypassTime(until, fg)
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
    // Not clipped: the count sits over the pill's corner and may reach past it.
    Column(
        modifier
            .pressable(shape = RoundedCornerShape(16.dp), onClick = onClick)
            .semantics { this.selected = selected }
            .testTag(tag),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(width = 58.dp, height = 30.dp), contentAlignment = Alignment.Center) {
            Box(Modifier.matchParentSize().background(pill, RoundedCornerShape(15.dp)))
            Icon(if (selected) icon else outlined, null, Modifier.size(22.dp), tint = if (selected) c.text else c.secondary)
            if (dotOnly) {
                if (count > 0) Box(Modifier.align(Alignment.TopEnd).padding(top = 3.dp, end = 14.dp).size(8.dp).background(countColor, CircleShape))
            } else {
                // Its left edge over the icon's top right; wider counts grow to the right, past the pill.
                CountPill(
                    count,
                    countColor,
                    Modifier.align(Alignment.TopStart).offset(x = 31.dp, y = (-7).dp).wrapContentSize(unbounded = true, align = Alignment.TopStart).testTag("$tag:count"),
                    ring = c.glass,
                )
            }
        }
        Spacer(Modifier.height(4.dp))
        RText(label, RType.sans(12f, if (selected) FontWeight.SemiBold else FontWeight.Medium), if (selected) c.text else c.secondary, maxLines = 1)
    }
}
