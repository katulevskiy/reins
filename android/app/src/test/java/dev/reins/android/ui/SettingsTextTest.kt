package dev.reins.android.ui

import dev.reins.android.feedback.FeedbackSettings
import dev.reins.android.ui.settings.PushChip
import dev.reins.android.ui.settings.accountsValue
import dev.reins.android.ui.settings.connectionLine
import dev.reins.android.ui.settings.hostOf
import dev.reins.android.ui.settings.pushChip
import dev.reins.android.ui.settings.soundsValue
import dev.reins.android.ui.settings.vaultPasskeysValue
import dev.reins.core.ConnectionView
import org.junit.Assert.assertEquals
import org.junit.Test

class SettingsTextTest {
    @Test
    fun theNotificationChipFollowsTheApprovalPhoneAndPush() {
        // Another phone approves, or notifications are off: no chip (the card says what to do instead).
        assertEquals(PushChip.Hidden, pushChip(approvalDevice = false, notificationsEnabled = true, appPush = true, serverPush = true))
        assertEquals(PushChip.Hidden, pushChip(approvalDevice = true, notificationsEnabled = false, appPush = true, serverPush = true))
        // No push in this build or on the server: requests arrive only while Reins is open.
        assertEquals(PushChip.OnlyWhileOpen, pushChip(approvalDevice = true, notificationsEnabled = true, appPush = false, serverPush = true))
        assertEquals(PushChip.OnlyWhileOpen, pushChip(approvalDevice = true, notificationsEnabled = true, appPush = true, serverPush = false))
        // A server that does not say is taken to send push.
        assertEquals(PushChip.On, pushChip(approvalDevice = true, notificationsEnabled = true, appPush = true, serverPush = null))
        assertEquals(PushChip.On, pushChip(approvalDevice = true, notificationsEnabled = true, appPush = true, serverPush = true))
    }

    @Test
    fun rowValuesAreAFewWords() {
        assertEquals("None", accountsValue(0))
        assertEquals("1 account", accountsValue(1))
        assertEquals("3 accounts", accountsValue(3))
        assertEquals("Not set up", vaultPasskeysValue(null))
        assertEquals("None", vaultPasskeysValue(0))
        assertEquals("2", vaultPasskeysValue(2))
        assertEquals("On", soundsValue(FeedbackSettings()))
        assertEquals("Off", soundsValue(FeedbackSettings(master = false)))
        assertEquals("Sounds only", soundsValue(FeedbackSettings(haptics = false)))
    }

    @Test
    fun aConnectionSaysWhereFromAndWhen() {
        val ai = ConnectionView("c1", "Claude", "claude.ai", 1_700_000_000, 1_700_000_000 - 300, null, null)
        assertEquals("claude.ai · 5 min ago", connectionLine(ai, nowSeconds = 1_700_000_000))
        assertEquals("never used", connectionLine(ai.copy(lastUsedAt = null), nowSeconds = 1_700_000_000))
        val computer = ai.copy(keyFingerprint = "4821 9930", lastUsedAt = 1_700_000_000)
        assertEquals("computer · just now", connectionLine(computer, nowSeconds = 1_700_000_000))
    }

    @Test
    fun theCardShowsTheServersHost() {
        assertEquals("app.reins2fa.com", hostOf("https://app.reins2fa.com/"))
        assertEquals("127.0.0.1:8000", hostOf("http://127.0.0.1:8000"))
    }
}
