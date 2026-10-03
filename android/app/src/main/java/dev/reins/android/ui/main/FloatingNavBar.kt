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
import dev.reins.android.design.CountPill
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.ui.nav.Tab

/** The bottom bar: a floating capsule with the two tabs, and a separate round button for settings. */
@Composable
fun FloatingNavBar(
    selected: Tab,
    activityCount: Int,
    grantCount: Int,
    onSelect: (Tab) -> Unit,
    onSettings: () -> Unit,
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
        Box(
            Modifier
                .size(60.dp)
                .glass(c, CircleShape, 8.dp)
                .pressable(shape = CircleShape, label = "Settings", onClick = onSettings)
                .testTag("openSettings"),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Rounded.Settings, "Settings", Modifier.size(26.dp), tint = c.text) }
    }
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
