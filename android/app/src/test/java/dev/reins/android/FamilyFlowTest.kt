package dev.reins.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.profiles.ProfileSwitch
import dev.reins.android.profiles.ProfilesProvider
import dev.reins.android.profiles.RegistryFactory
import dev.reins.android.profiles.WorkCoreFactory
import dev.reins.android.profiles.WorkProfiles
import dev.reins.android.ui.settings.AddFamilyAccountViewModel
import dev.reins.android.ui.settings.AddFamilyStep
import dev.reins.android.ui.settings.FamilyViewModel
import dev.reins.android.ui.settings.leavesIn
import dev.reins.core.FamilyLeaveAsk
import dev.reins.core.FamilyView
import dev.reins.core.ProfileKind
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** The Family profile (Decisions v2 · P7): joining with an invitation, leaving, a guardian's consent, the badge. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class FamilyFlowTest : FlowHarness() {
    private val registry = FakeRegistry()
    private val kid = FakeCore()

    @Before
    fun profiles() {
        ProfilesProvider.registry = RegistryFactory { registry }
        ProfilesProvider.workCore = WorkCoreFactory { _, _ -> kid }
        ProfileSwitch.restart = {}
    }

    @After
    fun personalAgain() {
        WorkProfiles.active = WorkProfiles.PERSONAL
        WorkProfiles.hasWork = false
    }

    private fun lees(minor: Boolean, guardian: Boolean = false, leaves: List<FamilyLeaveAsk> = emptyList()) = FamilyView(
        orgId = "f1",
        name = "The Lees",
        minor = minor,
        guardian = guardian,
        guardians = listOf("mom@example.com"),
        noticeDays = 3u,
        leave = null,
        minorProposed = false,
        sees = listOf("Requests from your agents in this profile", "What guardians decided", "Not your Personal profile"),
        leaves = leaves,
    )

    private fun waitFor(condition: () -> Boolean) {
        repeat(200) {
            shadowOf(android.os.Looper.getMainLooper()).idle()
            if (condition()) return
            Thread.sleep(10)
        }
        assertTrue("timed out", condition())
    }

    private val link = "reins-family://reins.example/" + "A".repeat(43)

    @Test
    fun joinsWithAnInviteLink() {
        kid.family = lees(minor = true)
        kid.ssoEmail = "sam.lee@gmail.com"
        val vm = AddFamilyAccountViewModel(container)
        var opened: String? = null
        vm.start(link) { opened = it }
        waitFor { opened != null }
        // An ordinary sign-in on the family profile's own core: a new account, no domain hint.
        assertTrue(kid.ssoBegins.isNotEmpty())
        assertTrue(FakeCore.shared.ssoBegins.isEmpty())
        container.ssoSignIn.deliver("com.reins2fa.app://sso-callback?code=c&state=${FakeCore.SSO_STATE}")
        waitFor { vm.ui.value.step is AddFamilyStep.Added || vm.ui.value.error != null }
        val added = (vm.ui.value.step as AddFamilyStep.Added).profile
        assertEquals(listOf(link), kid.familyJoins.toList())
        assertTrue(added.family)
        assertEquals("The Lees", added.label)
        assertTrue(registry.list().any { it.id == added.id && it.family && it.kind == ProfileKind.MANAGED })
        // The Personal core never took part.
        assertTrue(FakeCore.shared.familyJoins.isEmpty())
    }

    @Test
    fun anAccountThatIsNotNewIsRefusedAndItsProfileRemoved() {
        kid.family = null
        val vm = AddFamilyAccountViewModel(container)
        var opened: String? = null
        vm.start(link) { opened = it }
        waitFor { opened != null }
        container.ssoSignIn.deliver("com.reins2fa.app://sso-callback?code=c&state=${FakeCore.SSO_STATE}")
        waitFor { vm.ui.value.error != null }
        assertEquals("Use a new address for your family account.", vm.ui.value.error)
        assertEquals(1, registry.list().size)
        assertEquals(1, registry.removed.size)
        // Not an invitation at all: nothing starts.
        val other = AddFamilyAccountViewModel(container)
        other.start("https://example.com") {}
        assertEquals("This isn't a family invitation.", other.ui.value.error)
    }

    @Test
    fun minorLeaveShowsWaitingForConsent() {
        kid.family = lees(minor = true)
        val vm = FamilyViewModel(kid)
        vm.load()
        waitFor { vm.ui.value.view != null }
        vm.leave()
        waitFor { vm.ui.value.view?.leave != null }
        val leave = vm.ui.value.view!!.leave!!
        assertTrue(leave.needsConsent)
        assertNull(leave.effectiveAt)
        // An adult's leave counts down the notice instead.
        kid.family = lees(minor = false)
        vm.load()
        waitFor { vm.ui.value.view?.minor == false }
        vm.leave()
        waitFor { vm.ui.value.view?.leave != null }
        val adult = vm.ui.value.view!!.leave!!
        assertTrue(!adult.needsConsent)
        assertEquals("Leaves in 3 days", leavesIn(adult.effectiveAt!!, adult.requestedAt))
        // A guardian allows a minor's request.
        kid.family = lees(minor = false, guardian = true, leaves = listOf(FamilyLeaveAsk("u-sam", "sam@example.com", true, 1, null)))
        vm.allow("u-sam")
        waitFor { kid.familyConsents.isNotEmpty() }
        assertEquals(listOf("u-sam"), kid.familyConsents.toList())
    }

    @Test
    fun familyCardsShowTheBadge() {
        val family = FakeRegistry.profile("work-f1", ProfileKind.MANAGED, "The Lees", "f1").copy(family = true)
        val work = FakeRegistry.profile("work-a1", ProfileKind.MANAGED, "Acme", "a1")
        assertTrue(WorkProfiles.of(family).family)
        assertTrue(!WorkProfiles.of(work).family)
        assertEquals("The Lees", WorkProfiles.badge(WorkProfiles.of(family)))
        // Settings shows the family's page and the way to add one.
        registry.profiles += family
        WorkProfiles.active = WorkProfiles.of(family)
        kid.family = lees(minor = true)
        FakeCore.shared.family = lees(minor = true)
        FakeCore.shared.orgsOffered = true
        launch()
        tap("tabSettings")
        awaitTag("addFamilyAccount")
        awaitTag("familyRow")
    }
}
