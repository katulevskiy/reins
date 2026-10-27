package dev.reins.android

import android.app.NotificationManager
import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AppNotifier
import dev.reins.android.profiles.DeviceKey
import dev.reins.android.profiles.DeviceKeys
import dev.reins.android.profiles.Posture
import dev.reins.android.profiles.ProfileSwitch
import dev.reins.android.profiles.ProfilesProvider
import dev.reins.android.profiles.RegistryFactory
import dev.reins.android.profiles.WorkCoreFactory
import dev.reins.android.profiles.WorkProfiles
import dev.reins.android.profiles.enrollment
import dev.reins.android.ui.settings.AddWorkAccountViewModel
import dev.reins.android.ui.settings.AddWorkStep
import dev.reins.core.AccountProfile
import dev.reins.core.AttestationKindInput
import dev.reins.core.CoreException
import dev.reins.core.OrgView
import dev.reins.core.PostureInput
import dev.reins.core.ProfileKind
import dev.reins.core.ProfileRegistryInterface
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** The profiles on this phone in memory, as the core's registry keeps them. */
class FakeRegistry : ProfileRegistryInterface {
    val profiles = mutableListOf(profile("personal", ProfileKind.PERSONAL, "Personal", null))
    val pending = mutableListOf<AccountProfile>()
    var activeId = "personal"
    var personalEmail: String? = null
    val removed = mutableListOf<String>()

    override fun list() = profiles.toList()
    override fun active() = profiles.first { it.id == activeId }
    override fun setActive(id: String) {
        if (profiles.none { it.id == id }) throw CoreException.Invalid("No such profile")
        activeId = id
    }
    override fun notePersonal(email: String?) {
        personalEmail = email
    }
    override fun beginManaged(serverUrl: String): AccountProfile =
        profile("work-0000000000${pending.size + 1}", ProfileKind.MANAGED, "Work", null).copy(serverUrl = serverUrl).also { pending += it }
    override fun confirmManaged(id: String, orgId: String, orgName: String, email: String): AccountProfile {
        if (email == personalEmail) throw CoreException.Invalid("This is your personal account. Sign in with your work account.")
        val p = pending.first { it.id == id }.copy(label = orgName, orgId = orgId, email = email)
        pending.removeAll { it.id == id }
        profiles += p
        return p
    }
    override fun remove(id: String) {
        removed += id
        pending.removeAll { it.id == id }
        profiles.removeAll { it.id == id }
    }

    companion object {
        fun profile(id: String, kind: ProfileKind, label: String, orgId: String?) = AccountProfile(
            id = id,
            kind = kind,
            label = label,
            orgId = orgId,
            email = null,
            serverUrl = null,
            dataDir = "/data/$id/core",
            keyAlias = if (kind == ProfileKind.PERSONAL) "" else id,
            channelPrefix = if (kind == ProfileKind.PERSONAL) "" else "$id.",
        )
    }
}

/** A device key without a Keystore: fixed bytes, and what was asked of it. */
class FakeDeviceKeys : DeviceKeys {
    val challenges = mutableListOf<String>()
    val signed = mutableListOf<ByteArray>()
    var chain = listOf("MIIleaf", "MIIroot")

    override fun create(alias: String, challenge: ByteArray): DeviceKey {
        challenges += String(challenge)
        return DeviceKey("MFkwEwYHKoZIzj0CAQ", hardware = true, chain = chain)
    }

    override fun sign(alias: String, data: ByteArray): ByteArray {
        signed += data
        return byteArrayOf(0x30, 0x44)
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class WorkProfileFlowTest : FlowHarness() {
    private val registry = FakeRegistry()
    private val keys = FakeDeviceKeys()
    private val work = FakeCore()
    private var restarted = 0

    @Before
    fun profiles() {
        ProfilesProvider.registry = RegistryFactory { registry }
        ProfilesProvider.workCore = WorkCoreFactory { _, _ -> work }
        ProfilesProvider.deviceKeys = keys
        ProfileSwitch.restart = { restarted++ }
    }

    @After
    fun personalAgain() {
        WorkProfiles.active = WorkProfiles.PERSONAL
        WorkProfiles.hasWork = false
    }

    private val acme = OrgView("o1", "Acme", "business", listOf("Approver"), false, listOf("acme"))

    @Test
    fun `personal keeps todays key alias and channels, a work profile gets its own`() {
        val personal = WorkProfiles.of(FakeRegistry.profile("personal", ProfileKind.PERSONAL, "Personal", null))
        val acmeProfile = WorkProfiles.of(FakeRegistry.profile("work-1", ProfileKind.MANAGED, "Acme", "o1"))
        assertEquals("reins_dek_wrap_v1", WorkProfiles.dekAlias(personal.keyAlias))
        assertEquals("reins_dek_wrap_v1.work-1", WorkProfiles.dekAlias(acmeProfile.keyAlias))
        assertNull(WorkProfiles.badge(personal))
        assertEquals("Acme", WorkProfiles.badge(acmeProfile))
        assertEquals("reins_org_device.work-1", WorkProfiles.orgKeyAlias("work-1"))
        assertEquals("reins/org-enroll/v1\nabc", String(WorkProfiles.enrollmentMessage("abc")))
    }

    @Test
    fun `org names are shown as untrusted names`() {
        val p = FakeRegistry.profile("work-1", ProfileKind.MANAGED, "Acme\u202Elanosrep", "o1")
        assertFalse(WorkProfiles.of(p).label.contains('\u202E'))
    }

    @Test
    fun `a work profile posts to its own channels, grouped under the org`() {
        val manager = context.getSystemService(NotificationManager::class.java)
        val personalId = AppNotifier.channelId(AppNotifier.Kind.Approvals, sound = true, vibrate = true)
        WorkProfiles.active = WorkProfiles.of(FakeRegistry.profile("work-1", ProfileKind.MANAGED, "Acme", "o1"))
        val workId = AppNotifier.channel(context, AppNotifier.Kind.Approvals, dev.reins.android.feedback.FeedbackSettings())
        assertNotEquals(personalId, workId)
        assertTrue(workId, workId.startsWith("work-1.approvals-v"))
        assertEquals("Acme", manager.getNotificationChannelGroup(manager.getNotificationChannel(workId).group).name)
        // Migrating channels leaves the work profile's alone.
        AppNotifier.migrateChannels(manager)
        assertTrue(manager.getNotificationChannel(workId) != null)
    }

    @Test
    fun `posture reports root signs, and an enrollment carries the key, its signature and the attestation chain`() {
        assertEquals(false, Posture.rooted("release-keys") { false })
        assertEquals(true, Posture.rooted("test-keys") { false })
        assertEquals(true, Posture.rooted("release-keys") { it == "/system/xbin/su" })
        val posture = PostureInput(true, "Android", "16", false, "1.0")
        val input = enrollment(keys, "work-1", "challenge-1", posture)
        assertEquals(listOf("reins/org-enroll/v1\nchallenge-1"), keys.challenges)
        assertEquals("reins/org-enroll/v1\nchallenge-1", String(keys.signed.single()))
        assertEquals(Base64.encodeToString(byteArrayOf(0x30, 0x44), Base64.NO_WRAP), input.signature)
        assertEquals(AttestationKindInput.ANDROID_KEY_ATTESTATION, input.attestations.single().kind)
        assertEquals("MIIleaf,MIIroot", input.attestations.single().data)
        keys.chain = emptyList()
        assertEquals(AttestationKindInput.NONE, enrollment(keys, "work-1", "c", posture).attestations.single().kind)
    }

    private fun addWork(org: OrgView?): AddWorkAccountViewModel {
        work.org = org
        work.ssoEmail = "dana@acme.com"
        val vm = AddWorkAccountViewModel(container)
        var opened: String? = null
        vm.start("acme.com") { opened = it }
        waitFor { opened != null }
        assertTrue(opened!!.contains("domain_hint=acme.com"))
        container.ssoSignIn.deliver("com.reins2fa.app://sso-callback?code=c&state=${FakeCore.SSO_STATE}")
        waitFor { vm.ui.value.step != AddWorkStep.Working || vm.ui.value.error != null }
        return vm
    }

    @Test
    fun `adding a work account signs it in through its org, enrolls the phone and offers the switch`() {
        val vm = addWork(acme)
        val step = vm.ui.value.step as AddWorkStep.Added
        assertEquals("Acme", step.profile.label)
        assertTrue(step.enrolled)
        assertEquals(listOf(listOf(BuildConfig.DEFAULT_SERVER, "acme.com")), work.ssoBeginWorks.toList())
        assertEquals("challenge-1", work.enrollments.single().first)
        assertTrue(registry.list().any { it.label == "Acme" && it.kind == ProfileKind.MANAGED })
        // The Personal core never took part.
        assertTrue(FakeCore.shared.ssoBeginWorks.isEmpty())
        assertTrue(FakeCore.shared.enrollments.isEmpty())
    }

    @Test
    fun `an account without an org is refused and its profile removed`() {
        val vm = addWork(null)
        assertEquals("This account isn't managed by an organization.", vm.ui.value.error)
        assertEquals(1, registry.list().size)
        assertEquals(1, registry.removed.size)
        assertTrue(work.enrollments.isEmpty())
    }

    @Test
    fun `settings offers a work account only where the server has organizations`() {
        launch()
        tap("tabSettings")
        awaitTag("devicesRow")
        assertFalse(has("addWorkAccount"))
        closeApp()
        FakeCore.shared.orgsOffered = true
        launch()
        tap("tabSettings")
        awaitTag("addWorkAccount")
    }

    private fun waitFor(condition: () -> Boolean) {
        repeat(200) {
            shadowOf(android.os.Looper.getMainLooper()).idle()
            if (condition()) return
            Thread.sleep(10)
        }
        assertTrue("timed out", condition())
    }
}
