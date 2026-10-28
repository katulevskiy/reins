package dev.reins.android

import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.profiles.KeystoreDeviceKeys
import dev.reins.android.profiles.OrgSigning
import dev.reins.android.profiles.ProfilesProvider
import dev.reins.android.profiles.WorkProfiles
import dev.reins.android.push.PushPayload
import dev.reins.android.ui.org.OrgReviewsViewModel
import dev.reins.core.OrgReviewKind
import dev.reins.core.OrgReviewView
import dev.reins.core.OrgView
import dev.reins.core.ProfileKind
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** Work reviews (O2): an org reviewer approves or denies on the phone, signed with the key enrolled to the org. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class OrgReviewsTest : FlowHarness() {
    private val keys = FakeDeviceKeys()
    private val acme = OrgView("o1", "Acme", "business", listOf("Security admin"), true, emptyList())
    private val work = WorkProfiles.of(FakeRegistry.profile("work-1", ProfileKind.MANAGED, "Acme", "o1"))
    private val signedMessages = mutableListOf<String>()

    private fun review(id: String, kind: OrgReviewKind = OrgReviewKind.REQUEST, own: Boolean = false) = OrgReviewView(
        id = id,
        kind = kind,
        orgName = "Acme",
        requester = "dana@acme.com",
        connection = "Claude",
        title = if (kind == OrgReviewKind.REQUEST) "Send an email" else "Policy",
        lines = listOf(if (kind == OrgReviewKind.REQUEST) "Sends an email to ext@other.com" else "Activate policy revision 3"),
        requires = "Security",
        level = "L2",
        zone = "external",
        digest = "d-$id",
        snapshot = "snap",
        expiresAt = 1_700_000_900,
        own = own,
        needs = emptyList(),
        progress = "",
        step = null,
        urgent = false,
        reason = null,
    )

    private var registryBefore: dev.reins.android.profiles.RegistryFactory? = null

    @Before
    fun workProfile() {
        registryBefore = ProfilesProvider.registry
        ProfilesProvider.registry = dev.reins.android.profiles.RegistryFactory {
            FakeRegistry().apply { profiles += FakeRegistry.profile("work-1", ProfileKind.MANAGED, "Acme", "o1") }
        }
        ProfilesProvider.deviceKeys = keys
        OrgSigning.message = { org, r, approve, at ->
            "$org|${r.id}|$approve|$at".toByteArray().also { signedMessages += String(it) }
        }
        core.org = acme
        core.orgReviews = listOf(review("or_1"), review("ch_1", OrgReviewKind.CHANGE, own = true))
        WorkProfiles.active = work
    }

    @After
    fun personalAgain() {
        WorkProfiles.active = WorkProfiles.PERSONAL
        registryBefore?.let { ProfilesProvider.registry = it }
        ProfilesProvider.deviceKeys = KeystoreDeviceKeys()
        OrgSigning.message = OrgSigning.CORE
        core.org = null
        core.orgReviews = emptyList()
        core.orgDecisions.clear()
        core.orgDecideError = null
    }

    @Test
    fun listsReviewsForTheWorkProfile() {
        launch()
        tap("tabSettings")
        awaitTag("orgReviewsRow")
        awaitText("2")
        tap("orgReviewsRow")
        awaitTag("orgReview:or_1")
        assertTrue(showsText("Sends an email to ext@other.com"))
        assertTrue(showsText("requires Security"))
        assertTrue(has("orgApprove:or_1"))
        assertTrue(has("orgDeny:or_1"))
        // Your own change is confirmed, not approved.
        assertTrue(has("orgConfirm:ch_1"))
        assertFalse(has("orgApprove:ch_1"))
    }

    @Test
    fun approveSignsAndSends() {
        launch()
        tap("tabSettings")
        tap("orgReviewsRow")
        tap("orgApprove:or_1")
        awaitCore { core.orgDecisions.isNotEmpty() }
        val (id, approve, at, signature) = core.orgDecisions.single()
        assertEquals("or_1", id)
        assertEquals(true, approve)
        // The enrolled key signed exactly the core's review message for this answer, and that signature was sent.
        assertEquals(listOf("o1|or_1|true|$at"), signedMessages)
        assertEquals("o1|or_1|true|$at", String(keys.signed.single()))
        assertEquals(Base64.encodeToString(byteArrayOf(0x30, 0x44), Base64.NO_WRAP), signature)
        awaitGone("orgReview:or_1")
        // Deny signs a deny.
        core.orgReviews = core.orgReviews + review("or_2")
        val vm = OrgReviewsViewModel(core, work, keys, now = { 42L })
        vm.load()
        waitFor { vm.ui.value.reviews?.any { it.id == "or_2" } == true }
        vm.answer(vm.ui.value.reviews!!.first { it.id == "or_2" }, approve = false)
        waitFor { core.orgDecisions.size == 2 }
        assertEquals(listOf("or_2", false, 42L), core.orgDecisions.last().take(3))
        assertEquals("o1|or_2|false|42", signedMessages.last())
    }

    @Test
    fun personalProfileShowsNoReviews() {
        WorkProfiles.active = WorkProfiles.PERSONAL
        core.orgsOffered = true
        launch()
        tap("tabSettings")
        awaitTag("addWorkAccount")
        assertFalse(has("orgReviewsRow"))
        // The view model asks nothing of a Personal core.
        val vm = OrgReviewsViewModel(core, WorkProfiles.PERSONAL, keys, now = { 1L })
        vm.load()
        waitFor { vm.ui.value.reviews != null }
        assertEquals(emptyList<OrgReviewView>(), vm.ui.value.reviews)
        assertTrue(keys.signed.isEmpty())
    }

    @Test
    fun anOrgReviewPushOpensTheReviewsOfItsProfile() {
        val payload = PushPayload.parse(mapOf("t" to "orgreview", "id" to "or_1"))!!
        assertEquals("orgreview", payload.kind)
        kotlinx.coroutines.runBlocking { dev.reins.android.push.PushHandler.of(container).handle(payload) }
        // Nothing for the core to fetch: the notification opens the reviews.
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
