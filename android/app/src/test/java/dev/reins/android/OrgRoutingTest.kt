package dev.reins.android

import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.profiles.KeystoreDeviceKeys
import dev.reins.android.profiles.OrgSigning
import dev.reins.android.profiles.ProfilesProvider
import dev.reins.android.profiles.WorkProfiles
import dev.reins.android.push.PushPayload
import dev.reins.android.ui.org.OrgReviewsViewModel
import dev.reins.android.ui.org.OrgRouting
import dev.reins.core.OrgPasskeyView
import dev.reins.core.OrgReviewKind
import dev.reins.core.OrgReviewView
import dev.reins.core.OrgView
import dev.reins.core.ProfileKind
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * Org routing on the review cards (O3): who else must decide, how far it got and where an escalation took it; and a web
 * passkey the member added in the browser, confirmed on this phone with the key enrolled to the org.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class OrgRoutingTest : FlowHarness() {
    private val keys = FakeDeviceKeys()
    private val acme = OrgView("o1", "Acme", "business", listOf("Engineer"), true, emptyList())
    private val work = WorkProfiles.of(FakeRegistry.profile("work-1", ProfileKind.MANAGED, "Acme", "o1"))
    private val signedMessages = mutableListOf<String>()
    private val hash = "ab12cd34" + "0".repeat(56)
    private val passkey = OrgPasskeyView("cred_A-1", 1_700_000_000, false, hash, "ab12-cd34")

    private fun review(
        id: String,
        needs: List<String> = emptyList(),
        progress: String = "",
        step: String? = null,
        urgent: Boolean = false,
        reason: String? = null,
    ) = OrgReviewView(
        id = id,
        kind = OrgReviewKind.REQUEST,
        orgName = "Acme",
        requester = "dana@acme.com",
        connection = "Claude",
        title = "Write to GitHub",
        lines = listOf("Writes to acme/prod-db"),
        requires = "On-call",
        level = "L2",
        zone = "prod",
        digest = "d-$id",
        snapshot = "snap",
        expiresAt = 1_700_000_900,
        own = false,
        needs = needs,
        progress = progress,
        step = step,
        urgent = urgent,
        reason = reason,
    )

    private var registryBefore: dev.reins.android.profiles.RegistryFactory? = null

    @Before
    fun workProfile() {
        registryBefore = ProfilesProvider.registry
        ProfilesProvider.registry = dev.reins.android.profiles.RegistryFactory {
            FakeRegistry().apply { profiles += FakeRegistry.profile("work-1", ProfileKind.MANAGED, "Acme", "o1") }
        }
        ProfilesProvider.deviceKeys = keys
        OrgSigning.passkeyMessage = { org, k, at ->
            "$org|${k.credentialId}|${k.spkiHash}|${k.createdAt}|$at".toByteArray().also { signedMessages += String(it) }
        }
        core.org = acme
        WorkProfiles.active = work
    }

    @After
    fun personalAgain() {
        WorkProfiles.active = WorkProfiles.PERSONAL
        registryBefore?.let { ProfilesProvider.registry = it }
        ProfilesProvider.deviceKeys = KeystoreDeviceKeys()
        OrgSigning.passkeyMessage = OrgSigning.CORE_PASSKEY
        core.org = null
        core.orgReviews = emptyList()
        core.orgPasskeys = emptyList()
        core.passkeyConfirmations.clear()
    }

    @Test
    fun cardShowsWhoElseMustDecide() {
        core.orgReviews = listOf(
            review(
                "or_1",
                needs = listOf("On-call: payments"),
                progress = "1 of 2",
                step = "Escalated to Security admin",
                urgent = true,
                reason = "Rotate the leaked key",
            ),
            review("or_2"),
        )
        launch()
        tap("tabSettings")
        tap("orgReviewsRow")
        awaitTag("orgReview:or_1")
        assertTrue(showsText("Needs On-call: payments · 1 of 2"))
        assertTrue(showsText("Escalated to Security admin"))
        assertTrue(showsText("Rotate the leaked key"))
        assertTrue(has("orgUrgent:or_1"))
        // One approval decides: no routing lines, no marker.
        awaitTag("orgReview:or_2")
        assertFalse(has("orgNeeds:or_2"))
        assertFalse(has("orgStep:or_2"))
        assertFalse(has("orgUrgent:or_2"))
        assertFalse(has("orgReason:or_2"))
        // The lines in a few words: several obligations, progress alone, and the agent's words kept to one line each.
        assertEquals("Needs On-call: payments, 2 of Finance", OrgRouting.needsLine(review("x", needs = listOf("On-call: payments", "2 of\nFinance"))))
        assertEquals("1 of 2", OrgRouting.needsLine(review("x", progress = "1 of 2")))
        assertNull(OrgRouting.needsLine(review("x", needs = listOf(" "))))
        assertNull(OrgRouting.stepLine(review("x", step = "")))
    }

    @Test
    fun passkeyConfirmIsSigned() {
        core.orgPasskeys = listOf(passkey, passkey.copy(credentialId = "cred_B", spkiHash = "f".repeat(64), fingerprint = "ffff-ffff"))
        launch()
        tap("tabSettings")
        tap("orgReviewsRow")
        awaitTag("orgPasskey:cred_A-1")
        assertTrue(showsText("ab12-cd34"))
        tap("orgPasskeyConfirm:cred_A-1")
        awaitCore { core.passkeyConfirmations.isNotEmpty() }
        val (id, approve, at, signature) = core.passkeyConfirmations.single()
        assertEquals("cred_A-1", id)
        assertEquals(true, approve)
        // The enrolled key signed exactly the core's message over the key hash and registration time the card showed.
        assertEquals(listOf("o1|cred_A-1|$hash|1700000000|$at"), signedMessages)
        assertEquals("o1|cred_A-1|$hash|1700000000|$at", String(keys.signed.single()))
        assertEquals(Base64.encodeToString(byteArrayOf(0x30, 0x44), Base64.NO_WRAP), signature)
        awaitGone("orgPasskey:cred_A-1")
        // Not me signs a refusal of the other one.
        val vm = OrgReviewsViewModel(core, work, keys, now = { 42L })
        vm.load()
        waitFor { vm.ui.value.passkeys.any { it.credentialId == "cred_B" } }
        // The confirmed one no longer waits.
        assertFalse(vm.ui.value.passkeys.any { it.credentialId == "cred_A-1" })
        vm.confirmPasskey(vm.ui.value.passkeys.single(), approve = false)
        waitFor { core.passkeyConfirmations.size == 2 }
        assertEquals(listOf("cred_B", false, 42L), core.passkeyConfirmations.last().take(3))
        assertEquals("o1|cred_B|${"f".repeat(64)}|1700000000|42", signedMessages.last())
        waitFor { vm.ui.value.passkeys.isEmpty() }
    }

    @Test
    fun aPasskeyPushOpensTheReviews() {
        val long = "A".repeat(342)
        val payload = PushPayload.parse(mapOf("t" to "orgpasskey", "id" to long))!!
        assertEquals(PushPayload.ORG_PASSKEY, payload.kind)
        assertNull(PushPayload.parse(mapOf("t" to "orgreview", "id" to long)))
        kotlinx.coroutines.runBlocking { dev.reins.android.push.PushHandler.of(container).handle(payload) }
        assertTrue(core.pushes.isEmpty())
        val manager = context.getSystemService(android.app.NotificationManager::class.java)
        val posted = shadowOf(manager).activeNotifications.single()
        val open = shadowOf(posted.notification.contentIntent).savedIntent
        assertEquals(dev.reins.android.platform.AppNotifier.ACTION_OPEN_ORG_REVIEWS, open.action)
    }

    private fun waitFor(condition: () -> Boolean) {
        repeat(300) {
            shadowOf(android.os.Looper.getMainLooper()).idle()
            if (condition()) return
            Thread.sleep(10)
        }
        assertTrue("timed out", condition())
    }
}
