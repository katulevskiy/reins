package dev.reins.android.design

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import dev.reins.android.profiles.WorkProfiles

/**
 * The work profile's badge on cards and rows (06 §2: "Cards and Activity rows carry a profile badge"): the org's name
 * with a briefcase (a family's: the family glyph). Nothing in the Personal profile. [onField]: on an ask's coloured field.
 */
@Composable
fun ProfileBadge(modifier: Modifier = Modifier, onField: Boolean = false, active: WorkProfiles.Active = WorkProfiles.active) {
    val label = WorkProfiles.badge(active) ?: return
    val c = LocalColors.current
    Pill(label, if (onField) c.onField else c.violet, modifier.testTag("profileBadge"), glyph = if (active.family) Glyph.Family else Glyph.Briefcase)
}
