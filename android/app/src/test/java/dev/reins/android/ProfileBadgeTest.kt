package dev.reins.android

import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.design.ProfileBadge
import dev.reins.android.design.ReinsTheme
import dev.reins.android.profiles.WorkProfiles
import dev.reins.core.ProfileKind
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ProfileBadgeTest {
    @get:Rule val rule = createComposeRule()

    @Test
    fun `work cards and rows carry the org badge`() {
        val acme = WorkProfiles.of(FakeRegistry.profile("work-1", ProfileKind.MANAGED, "Acme", "o1"))
        rule.setContent { ReinsTheme { ProfileBadge(active = acme) } }
        rule.onNodeWithTag("profileBadge").assertExists()
        rule.onNodeWithText("Acme").assertExists()
    }

    @Test
    fun `personal cards and rows carry none`() {
        rule.setContent { ReinsTheme { ProfileBadge(active = WorkProfiles.PERSONAL) } }
        rule.onNodeWithTag("profileBadge").assertDoesNotExist()
    }
}
