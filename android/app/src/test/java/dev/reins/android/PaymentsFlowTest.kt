package dev.reins.android

import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.core.LimitPeriod
import dev.reins.core.PurchaseChoice
import dev.reins.core.SpendLimitInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Payments: the purchase screen pays with what the user picked, after biometrics; the settings reach the core. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class PaymentsFlowTest : FlowHarness() {
    @Before
    fun resetPayments() {
        core.payments = null
        core.spending = TestData.spending()
        core.purchaseApprovals.clear()
        core.paymentsCalls.clear()
        core.addedLimits.clear()
        core.connectProviderError = null
    }

    @Test
    fun a_purchase_is_approved_with_the_address_and_method_picked_after_biometrics() {
        openRequest(TestData.purchaseView())
        awaitTag("receipt")
        assertTrue(showsText("$34.97"))
        tap("shipTo:work")
        tap("payWith:card:visa")
        authResult = AuthResult.Cancelled
        tap("approve")
        awaitCore { prompts.get() == 1 }
        assertTrue("nothing is approved without the screen lock", core.purchaseApprovals.isEmpty())
        authResult = AuthResult.Success
        tap("approve")
        awaitCore { core.purchaseApprovals.isNotEmpty() }
        assertEquals("req20" to PurchaseChoice("card:visa", "work", null), core.purchaseApprovals.single())
    }

    @Test
    fun approving_with_a_spend_limit_creates_it_for_this_store_with_a_virtual_card() {
        openRequest(TestData.purchaseView())
        awaitTag("receipt")
        // A vault card is never offered for a limit.
        tap("payWith:card:visa")
        assertTrue(!has("makeLimit"))
        tap("payWith:virtual_card")
        tap("makeLimit")
        awaitTag("limitEach")
        rule.onNodeWithTag("limitEach").performTextReplacement("40")
        rule.onNodeWithTag("limitDay").performTextReplacement("80")
        tap("approve")
        awaitCore { core.purchaseApprovals.isNotEmpty() }
        val limit = core.purchaseApprovals.single().second.limit
        assertEquals(
            SpendLimitInput("", listOf("amazon.com"), "virtual_card", "USD", 4_000, 8_000, LimitPeriod.DAY, 7 * 86_400L),
            limit,
        )
    }

    @Test
    fun paying_on_the_phone_opens_the_stores_checkout() {
        openRequest(TestData.purchaseView())
        awaitTag("receipt")
        tap("payWith:pay_on_phone")
        awaitText("Open checkout")
        tap("approve")
        awaitCore { core.purchaseApprovals.isNotEmpty() }
        val opened = generateSequence { nextStarted() }.firstOrNull { it.dataString?.startsWith("https://www.amazon.com/") == true }
        assertEquals("https://www.amazon.com/gp/buy/spc/handlers/display.html", opened?.dataString)
    }

    @Test
    fun payments_is_turned_on_and_privacy_dot_com_connected_from_integrations() {
        core.payments = TestData.paymentsOverview(enabled = false).copy(provider = null, limits = emptyList())
        launch()
        tap("integrations")
        tap("service:payments")
        awaitTag("paymentsEnable")
        core.payments = TestData.paymentsOverview().copy(provider = null, limits = emptyList())
        tap("paymentsEnable")
        awaitTag("connectPrivacy")
        assertTrue(core.serviceAdded.contains("payments" to ""))
        tap("connectPrivacy")
        awaitTag("privacyKey")
        rule.onNodeWithTag("privacyKey").performTextReplacement("pk_test_123")
        tap("privacySandbox")
        tap("privacyConnect")
        awaitCore { core.paymentsCalls.contains("provider privacy sandbox=true singleUse=false") }
        tap("toggle:card:amex")
        awaitCore { core.paymentsCalls.contains("method card:amex true") }
    }

    @Test
    fun spending_shows_a_foreign_charge_and_closes_an_open_card() {
        core.payments = TestData.paymentsOverview()
        launch()
        tap("integrations")
        tap("service:payments")
        tap("openSpending")
        awaitTag("mismatch:p1")
        assertTrue(showsText("$87.46"))
        tap("acknowledge:p1")
        awaitCore { core.paymentsCalls.contains("acknowledge p1") }
        tap("purchase:p3")
        tap("closeCard:p3")
        awaitCore { core.paymentsCalls.contains("close p3") }
        // Reported failed, paid at the store: it counts until the user says nothing was charged.
        tap("purchase:p0")
        tap("clear:p0")
        awaitCore { core.paymentsCalls.contains("clear p0") }
    }
}
