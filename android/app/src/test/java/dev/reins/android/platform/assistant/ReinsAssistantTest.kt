package dev.reins.android.platform.assistant

import android.app.PendingIntent
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.FakeCore
import dev.reins.android.MainActivity
import dev.reins.android.TestData
import dev.reins.android.platform.AppNotifier
import dev.reins.android.profiles.WorkProfiles
import dev.reins.core.AutopilotMode
import dev.reins.core.ConnectionView
import dev.reins.core.CoreException
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import dev.reins.core.ReinsCoreInterface
import dev.reins.core.SessionInfo
import java.time.Instant
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.suspendCancellableCoroutine
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * Gemini can only take power away. These cover the mapping, the redaction and the refusals.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class ReinsAssistantTest {
    private class Probe {
        val opened = mutableListOf<String>()
        val denied = mutableListOf<String?>()
        var lockdowns = 0
        var tiles = 0
        var bypasses = 0
        var overnights = 0
        var autopilots = 0
        var locked = false
        var allowed = true
        var onBypass: suspend () -> Unit = {}
    }

    private fun signedIn(configure: FakeCore.() -> Unit = {}) = FakeCore().apply {
        session = SessionInfo("https://app.reins2fa.com", "me@example.com")
        configure()
    }

    private fun assistant(core: ReinsCoreInterface, probe: Probe = Probe(), timeoutMs: Long = 10_000) = probe to ReinsAssistant(
        core = core,
        isLocked = { probe.locked },
        isAllowed = { probe.allowed },
        openScreen = { id ->
            probe.opened += id
            pendingIntent()
        },
        afterDeny = { probe.denied += it },
        playLockdown = { probe.lockdowns += 1 },
        refreshTiles = { probe.tiles += 1 },
        stopBypasses = {
            probe.bypasses += 1
            probe.onBypass()
        },
        stopOvernights = { probe.overnights += 1 },
        refreshAutopilot = { probe.autopilots += 1 },
        dispatcher = kotlinx.coroutines.Dispatchers.Unconfined,
        timeoutMs = timeoutMs,
    )

    @Test
    fun `status names the mode and counts only requests`() = runBlocking {
        val core = signedIn {
            apGlobal = FakeCore.ApRow(mode = AutopilotMode.AUTO)
            pending = listOf(TestData.pending("r1"), TestData.pairingItem(), TestData.joinItem())
            connections = listOf(TestData.connection("c1"), TestData.connection("c2", "Grok"))
        }
        val status = assistant(core).second.getStatus()
        assertEquals("auto", status.mode)
        assertNull(status.until)
        assertEquals(1, status.waiting)
        assertEquals(2, status.connectedAis)
    }

    @Test
    fun `a timed bypass reports its end and an open ended overnight does not`() = runBlocking {
        val end = System.currentTimeMillis() / 1000 + 3_600
        val bypass = signedIn {
            apGlobal = FakeCore.ApRow(mode = AutopilotMode.MANUAL, bypassUntil = end, bypassSince = end - 60)
        }
        val bypassStatus = assistant(bypass).second.getStatus()
        assertEquals("bypass", bypassStatus.mode)
        assertEquals(Instant.ofEpochSecond(end), bypassStatus.until)

        val overnight = signedIn {
            apGlobal = FakeCore.ApRow(mode = AutopilotMode.OVERNIGHT, overnightUntil = Long.MAX_VALUE, overnightSince = 1)
        }
        val overnightStatus = assistant(overnight).second.getStatus()
        assertEquals("overnight", overnightStatus.mode)
        assertNull(overnightStatus.until)
    }

    @Test
    fun `waiting summaries are one line, newest first, and vault stays the operation`() = runBlocking {
        val long = "line\n" + "x".repeat(250)
        val core = signedIn {
            pending = listOf(
                TestData.pending("old", createdAt = 1, headline = "older"),
                TestData.pending("new", createdAt = 9, headline = long),
                TestData.pending("vault", createdAt = 5, service = "vault", headline = "Visa 4242 password", opTitle = "Unlock").copy(title = "secret title"),
                TestData.pending("pay", createdAt = 4, service = "payments", headline = "\$50 at Amazon", opTitle = "").copy(title = "card 4242"),
                TestData.pairingItem(),
            )
        }
        val (probe, assistant) = assistant(core)
        val page = assistant.listWaitingRequests(null)
        assertEquals(listOf("new", "vault", "pay", "old"), page.map { it.id })
        assertEquals("Search Gmail · 2", page[0].summary)
        assertFalse(page[0].summary.contains("line"))
        assertFalse(page[0].summary.contains("x"))
        assertEquals("Unlock", page[1].summary)
        assertFalse(page[1].summary.contains("Visa"))
        assertEquals("Payments", page[2].summary)
        assertFalse(page[2].summary.contains("Amazon"))
        assertEquals("Search Gmail · 2", page[3].summary)
        assertFalse(page[3].summary.contains("older"))
        assertEquals("Gmail", page[0].integration)
        assertEquals("Password vault", page[1].integration)
        assertTrue(probe.opened.isEmpty())
    }

    @Test
    fun `limits fall back when omitted and stop at the maximum`() = runBlocking {
        val core = signedIn {
            pending = (1..30).map { TestData.pending("r$it", createdAt = it.toLong(), headline = "h$it") }
            activity = (1..60).map { TestData.entry(it.toLong(), at = it.toLong()) }
        }
        val assistant = assistant(core).second
        assertEquals((30 downTo 21).map { "r$it" }, assistant.listWaitingRequests(null).map { it.id })
        assertEquals(10, assistant.listWaitingRequests(0).size)
        assertEquals(10, assistant.listWaitingRequests(-3).size)
        assertEquals(25, assistant.listWaitingRequests(100).size)
        assertEquals(listOf("r30", "r29"), assistant.listWaitingRequests(2).map { it.id })
        assertEquals(15, assistant.searchActivity(null, null, null, null).size)
        assertEquals(50, assistant.searchActivity(null, null, null, 80).size)
        assertEquals(15, assistant.searchActivity(null, null, null, -1).size)
    }

    @Test
    fun `activity redacts vault and payments and names who decided`() = runBlocking {
        val core = signedIn {
            activity = listOf(
                TestData.entry(1, outcome = "released", at = 10, grantId = "g1", service = "gmail", opTitle = "Read mail")
                    .copy(label = TestData.label("gmail", "read", "Read", "the invoice")),
                TestData.entry(2, outcome = "denied", at = 20, decidedBy = "lockdown"),
                TestData.entry(3, outcome = "error", at = 30, decidedBy = "autopilot"),
                TestData.entry(4, outcome = "released", at = 40, decidedBy = "bypass", service = "payments", opTitle = "Pay")
                    .copy(detail = "\$50 at Amazon"),
                TestData.entry(5, outcome = "released", at = 50, service = "vault", opTitle = "")
                    .copy(detail = "Bank password 1234"),
            )
        }
        val items = assistant(core).second.searchActivity(null, null, null, 20)
        assertEquals(listOf(50L, 40L, 30L, 20L, 10L), items.map { it.at.epochSecond })
        assertEquals("Read mail · 2", items[4].summary)
        assertFalse(items[4].summary.contains("invoice"))
        assertEquals("standing permission", items[4].decidedBy)
        assertEquals("approved", items[4].outcome)
        assertEquals("denied", items[3].outcome)
        assertEquals("lockdown", items[3].decidedBy)
        assertEquals("error", items[2].outcome)
        assertEquals("autopilot", items[2].decidedBy)
        assertEquals("Pay", items[1].summary)
        assertFalse(items[1].summary.contains("Amazon"))
        assertEquals("bypass", items[1].decidedBy)
        assertEquals("Password vault", items[0].summary)
        assertFalse(items[0].summary.contains("password"))
        assertTrue(core.activityQueries.isEmpty())
    }

    @Test
    fun `activity filters by agent and integration and never sends the words to the core`() = runBlocking {
        val since = Instant.ofEpochSecond(15)
        val core = signedIn {
            activity = listOf(
                TestData.entry(1, at = 10, opTitle = "Old", count = 1u).copy(
                    connectionLabel = "Claude",
                    service = "gmail",
                    detail = "secret body from:bank",
                ),
                TestData.entry(2, at = 30, opTitle = "Push", count = 1u).copy(connectionLabel = "Grok", service = "github"),
                TestData.entry(3, at = 40, opTitle = "Read Gmail messages", count = 3u).copy(connectionLabel = "Grok", service = "gmail"),
            )
        }
        val assistant = assistant(core).second
        val items = assistant.searchActivity(ai = "gro", integration = "git", since = since, limit = null)
        assertEquals(listOf("Push"), items.map { it.summary })
        assertEquals("", core.activityQueries.single().text)
        assertEquals(15L, core.activityQueries.single().since)
        val counted = assistant.searchActivity(null, "mail", null, null).single { it.summary.startsWith("Read") }
        assertEquals("Read Gmail messages · 3", counted.summary)
        assertTrue(core.activityQueries.all { it.text.isEmpty() })
        assertTrue(items.none { it.summary.contains("secret") || it.summary.contains("bank") })
    }

    @Test
    fun `deny writes the note and deny all leaves pairings`() = runBlocking {
        val core = signedIn {
            pending = listOf(
                TestData.pending("r1"),
                TestData.pending("r2"),
                TestData.pairingItem("pair"),
                TestData.joinItem("join"),
                TestData.pending("blob").copy(kind = PendingKind.BLOB),
            )
        }
        val (probe, assistant) = assistant(core)
        val note = "n".repeat(600)
        assertEquals("r1", assistant.denyRequest("r1", note).requestId)
        assertEquals(500, core.notedDenials.single().second?.length)
        assertEquals(listOf("r1"), probe.denied)
        assertEquals(1, probe.tiles)
        val count = assistant.denyAllWaiting()
        assertEquals(1, count.count)
        assertEquals(setOf("pair", "join", "blob"), core.pending.map { it.id }.toSet())
        assertEquals(listOf("r1", null), probe.denied)
        assertEquals(2, probe.tiles)
        assertTrue(core.notedDenials.none { it.first == "pair" || it.first == "join" || it.first == "blob" })
    }

    @Test
    fun `an unknown request or pairing is not denied`() = runBlocking {
        val core = signedIn { pending = listOf(TestData.pairingItem("pair")) }
        val (probe, assistant) = assistant(core)
        assertDenied("That request is no longer waiting.") { assistant.openRequest("missing") }
        assertDenied("That request is no longer waiting.") { assistant.denyRequest("pair", null) }
        assertTrue(core.notedDenials.isEmpty())
        assertTrue(probe.opened.isEmpty())
        assertTrue(probe.denied.isEmpty())
    }

    @Test
    fun `openRequest hands the id to the same intent a notification uses`() = runBlocking {
        val core = signedIn { pending = listOf(TestData.pending("req1", quick = true, waitUntil = 1_700_000_300)) }
        val (probe, assistant) = assistant(core)
        assistant.openRequest("req1")
        assertEquals(listOf("req1"), probe.opened)
        val row = assistant.listWaitingRequests(1).single()
        assertTrue(row.canApproveQuickly)
        assertEquals(Instant.ofEpochSecond(1_700_000_300), row.aiWaitsUntil)

        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val pending = AppNotifier.openRequest(context, "req1", WorkProfiles.PERSONAL)
        val shadow = shadowOf(pending)
        val intent = shadow.savedIntent
        assertEquals(AppNotifier.ACTION_OPEN_ITEM, intent.action)
        assertEquals(AppNotifier.KIND_REQUEST, intent.getStringExtra(AppNotifier.EXTRA_KIND))
        assertEquals("req1", intent.getStringExtra(AppNotifier.EXTRA_ID))
        assertEquals(MainActivity::class.java.name, intent.component?.className)
        assertTrue((shadow.flags and PendingIntent.FLAG_IMMUTABLE) != 0)
        assertEquals(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP, intent.flags and (Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP))
    }

    @Test
    fun `lockdown turns on and manual refuses to turn it off`() = runBlocking {
        val core = signedIn()
        val (probe, assistant) = assistant(core)
        val status = assistant.startLockdown()
        assertEquals("lockdown", status.mode)
        assertEquals(1, probe.lockdowns)
        assertEquals(1, probe.tiles)
        assertEquals(listOf(Triple(null, AutopilotMode.LOCKDOWN, null)), core.modeCalls.toList())
        assertEquals(0, probe.bypasses)
        assertEquals(0, probe.overnights)

        val message = assertThrows(androidx.appfunctions.AppFunctionDeniedException::class.java) {
            runBlocking { assistant.switchToManual() }
        }
        assertEquals("Lockdown is on; end it in Reins.", message.message)
        assertEquals(0, probe.bypasses)
        assertEquals(0, probe.overnights)
        assertEquals(listOf(Triple(null, AutopilotMode.LOCKDOWN, null)), core.modeCalls.toList())
    }

    @Test
    fun `manual stops bypass and overnight then sets manual`() = runBlocking {
        val end = System.currentTimeMillis() / 1000 + 3_600
        val core = signedIn {
            apGlobal = FakeCore.ApRow(mode = AutopilotMode.AUTO, bypassUntil = end, bypassSince = end - 10)
        }
        val (probe, assistant) = assistant(core)
        val status = assistant.switchToManual()
        assertEquals("manual", status.mode)
        assertEquals(1, probe.bypasses)
        assertEquals(1, probe.overnights)
        assertEquals(AutopilotMode.MANUAL, core.modeCalls.last().second)
        assertTrue(probe.autopilots >= 1)
        assertEquals(1, probe.tiles)
    }

    @Test
    fun `revoke drops a known permission and refuses an unknown one`() = runBlocking {
        val core = signedIn {
            grants = listOf(
                TestData.grant("g-new", ageSeconds = 10),
                TestData.grant("g-old", ageSeconds = 500),
                TestData.grant("g-vault", action = "read").copy(service = "vault", summary = "Bank password 1234", active = true),
                TestData.grant("g-dead", active = false),
            )
        }
        val assistant = assistant(core).second
        val open = assistant.listStandingPermissions()
        assertEquals(listOf("g-new", "g-old", "g-vault"), open.map { it.id })
        assertFalse(open.any { it.id == "g-dead" })
        assertEquals("read", open.first { it.id == "g-vault" }.allows)
        assertFalse(open.first { it.id == "g-vault" }.allows.contains("password"))
        assertEquals("Read Gmail", open.first { it.id == "g-new" }.allows)
        assertFalse(open.any { it.allows.contains("@bank") || it.allows.contains("corp") })
        assertEquals("g-vault", assistant.revokeStandingPermission("g-vault").permissionId)
        assertEquals(listOf("g-vault"), core.revokedGrants.toList())
        assertNotFound("That permission is not open.") { assistant.revokeStandingPermission("missing") }
        assertEquals(listOf("g-vault"), core.revokedGrants.toList())
    }

    @Test
    fun `connected agents are listed without a way to drop one`() = runBlocking {
        val core = signedIn {
            connections = listOf(TestData.connection("c1", "Claude").copy(lastUsedAt = 1_700_000_100))
        }
        val ai = assistant(core).second.listConnectedAis().single()
        assertEquals("c1", ai.id)
        assertEquals("Claude", ai.name)
        assertEquals(Instant.ofEpochSecond(1_700_000_100), ai.lastSeen)
        assertTrue(core.revokedConnections.isEmpty())
    }

    @Test
    fun `a locked phone still allows the emergency brake`() = runBlocking {
        val core = signedIn {
            pending = listOf(TestData.pending("r1"))
            grants = listOf(TestData.grant())
            connections = listOf(TestData.connection())
            activity = listOf(TestData.entry(1))
        }
        val (probe, assistant) = assistant(core)
        probe.locked = true
        assertEquals("Unlock the phone first", assertThrows(androidx.appfunctions.AppFunctionDeniedException::class.java) {
            runBlocking { assistant.listWaitingRequests(null) }
        }.message)
        listOf<() -> Unit>(
            { runBlocking { assistant.openRequest("r1") } },
            { runBlocking { assistant.denyRequest("r1", null) } },
            { runBlocking { assistant.denyAllWaiting() } },
            { runBlocking { assistant.searchActivity(null, null, null, null) } },
            { runBlocking { assistant.listStandingPermissions() } },
            { runBlocking { assistant.revokeStandingPermission("g1") } },
            { runBlocking { assistant.listConnectedAis() } },
        ).forEach { block ->
            assertEquals("Unlock the phone first", assertThrows(androidx.appfunctions.AppFunctionDeniedException::class.java, block).message)
        }
        assertTrue(core.notedDenials.isEmpty())
        assertTrue(core.revokedGrants.isEmpty())
        assertEquals("manual", assistant.getStatus().mode)
        assertEquals("lockdown", assistant.startLockdown().mode)
        core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.BYPASS, bypassUntil = System.currentTimeMillis() / 1000 + 60)
        assertEquals("manual", assistant.switchToManual().mode)
    }

    @Test
    fun `signed out is permission required before anything changes`() = runBlocking {
        val core = FakeCore().apply { pending = listOf(TestData.pending("r1")) }
        val assistant = assistant(core).second
        val missing = assertThrows(androidx.appfunctions.AppFunctionPermissionRequiredException::class.java) {
            runBlocking { assistant.denyRequest("r1", "no") }
        }
        assertEquals("Sign in to Reins first", missing.message)
        assertTrue(core.notedDenials.isEmpty())
        core.sessionError = CoreException.NotLoggedIn()
        core.sessionThrows = 1
        assertEquals("Sign in to Reins first", assertThrows(androidx.appfunctions.AppFunctionPermissionRequiredException::class.java) {
            runBlocking { assistant.getStatus() }
        }.message)
    }

    @Test
    fun `a switch that is off refuses before anything in the core changes`() = runBlocking {
        val core = signedIn {
            pending = listOf(TestData.pending("r1"))
        }
        val probe = Probe().apply { allowed = false }
        val assistant = assistant(core, probe).second
        suspend fun refused(block: suspend () -> Unit) {
            assertEquals(
                "Gemini is off in Reins",
                assertThrows(androidx.appfunctions.AppFunctionDeniedException::class.java) {
                    runBlocking { block() }
                }.message,
            )
        }
        refused { assistant.getStatus() }
        refused { assistant.denyRequest("r1", null) }
        refused { assistant.startLockdown() }
        refused { assistant.switchToManual() }
        assertTrue(core.notedDenials.isEmpty())
        assertTrue(core.modeCalls.isEmpty())
        assertEquals(0, probe.lockdowns)
        assertEquals(0, probe.tiles)
    }

    @Test
    fun `manual sets each looser agent to manual and leaves a lockdown agent`() = runBlocking {
        val core = signedIn {
            apGlobal = FakeCore.ApRow(mode = AutopilotMode.AUTO)
            apConnections["c-auto"] = FakeCore.ApRow(mode = AutopilotMode.AUTO)
            apConnections["c-night"] = FakeCore.ApRow(mode = AutopilotMode.OVERNIGHT, overnightUntil = Long.MAX_VALUE, overnightSince = 1)
            apConnections["c-lock"] = FakeCore.ApRow(mode = AutopilotMode.LOCKDOWN)
            apConnections["c-man"] = FakeCore.ApRow(mode = AutopilotMode.MANUAL)
        }
        val status = assistant(core).second.switchToManual()
        assertEquals("manual", status.mode)
        assertTrue(core.modeCalls.contains(Triple("c-auto", AutopilotMode.MANUAL, null)))
        assertTrue(core.modeCalls.contains(Triple("c-night", AutopilotMode.MANUAL, null)))
        assertTrue(core.modeCalls.none { it.first == "c-lock" || it.first == "c-man" })
        assertEquals(Triple(null, AutopilotMode.MANUAL, null), core.modeCalls.last())
    }

    @Test
    fun `a lockdown that starts during the stop is not overwritten`() = runBlocking {
        val core = signedIn { apGlobal = FakeCore.ApRow(mode = AutopilotMode.AUTO) }
        val (probe, assistant) = assistant(core)
        probe.onBypass = { core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.LOCKDOWN) }
        val message = assertThrows(androidx.appfunctions.AppFunctionDeniedException::class.java) {
            runBlocking { assistant.switchToManual() }
        }
        assertEquals("Lockdown is on; end it in Reins.", message.message)
        assertTrue(core.modeCalls.none { it.second == AutopilotMode.MANUAL })
        assertEquals(1, probe.bypasses)
        assertEquals(1, probe.overnights)
    }

    @Test
    fun `a mode write failure is not swallowed`() = runBlocking {
        val core = signedIn { apGlobal = FakeCore.ApRow(mode = AutopilotMode.AUTO) }
        core.modeFailure = CoreException.Invalid("the owner did not confirm")
        val message = assertThrows(androidx.appfunctions.AppFunctionInvalidArgumentException::class.java) {
            runBlocking { assistant(core).second.switchToManual() }
        }
        assertEquals("the owner did not confirm", message.message)
        assertTrue(core.modeCalls.isEmpty())
    }

    @Test
    fun `outcomes use the core's words and do not treat an unknown as approved`() = runBlocking {
        val core = signedIn {
            activity = listOf(
                TestData.entry(1, outcome = "sent", at = 1, opTitle = "Send", count = 1u),
                TestData.entry(2, outcome = "granted", at = 2, opTitle = "Grant", count = 1u),
                TestData.entry(3, outcome = "revoked", at = 3, opTitle = "Revoke", count = 1u),
                TestData.entry(4, outcome = "held", at = 4, opTitle = "Hold", count = 1u),
                TestData.entry(5, outcome = "undone", at = 5, opTitle = "Undo", count = 1u),
                TestData.entry(6, outcome = "started", at = 6, opTitle = "Start", count = 1u),
            )
        }
        val items = assistant(core).second.searchActivity(null, null, null, 20)
        assertEquals(listOf("started", "undone", "held", "revoked", "approved", "approved"), items.map { it.outcome })
    }

    @Test
    fun `a gate that flips stops a deny-all and is checked again before a result is returned`() = runBlocking {
        val core = signedIn {
            pending = listOf(TestData.pending("r1", createdAt = 1), TestData.pending("r2", createdAt = 2))
        }
        val probe = Probe()
        var denied = 0
        val script = CoreScript(core, onDeny = {
            denied += 1
            if (denied == 1) probe.locked = true
        })
        val count = assistant(script, probe).second.denyAllWaiting()
        assertEquals(1, count.count)
        assertEquals(listOf("r1"), core.notedDenials.map { it.first })
        assertTrue(core.pending.any { it.id == "r2" })

        probe.locked = false
        probe.allowed = true
        val reading = CoreScript(core, onPending = { probe.allowed = false })
        val message = assertThrows(androidx.appfunctions.AppFunctionDeniedException::class.java) {
            runBlocking { assistant(reading, probe).second.listWaitingRequests(null) }
        }
        assertEquals("Gemini is off in Reins", message.message)
    }

    @Test
    fun `the gemini switch is stored per reins profile`() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val store = AssistantStore(context)
        assertFalse(store.enabled("personal"))
        assertFalse(store.enabled("acme"))
        store.setEnabled("personal", true)
        assertTrue(store.enabled("personal"))
        assertFalse(store.enabled("acme"))
        store.setEnabled("acme", true)
        store.setEnabled("personal", false)
        assertFalse(store.enabled("personal"))
        assertTrue(store.enabled("acme"))
    }

    @Test
    fun `reconcile applies the value stored when its turn starts`() = runBlocking {
        var stored = true
        val applied = mutableListOf<Boolean>()
        val entered = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        val first = async {
            AssistantAccess.reconcileLatest({ stored }) {
                entered.complete(Unit)
                release.await()
                applied += it
            }
        }
        entered.await()
        stored = false
        val second = async {
            AssistantAccess.reconcileLatest({ stored }) { applied += it }
        }
        release.complete(Unit)
        first.await()
        second.await()
        assertEquals(listOf(true, false), applied)
    }

    /**
     * The enablement list is copied by hand from KSP. The generated asset is not in the unit-test asset
     * manager, and loading the generated service needs the API 36 sandbox, which does not start on this JDK.
     * The service class file is on the test classpath, and its constant pool holds each `FUNCTION_ID_*`.
     */
    @Test
    fun `saved function ids match the generated inventory`() {
        val bytes = javaClass.classLoader.getResourceAsStream(
            "dev/reins/android/platform/assistant/ReinsAppFunctionService.class",
        )?.use { it.readBytes() } ?: error("Generated service class is not on the test classpath")
        val generated = Regex("dev\\.reins\\.android\\.platform\\.assistant\\.ReinsAppFunctions#[A-Za-z0-9]+")
            .findAll(String(bytes, Charsets.ISO_8859_1))
            .map { it.value }
            .toSet()
        assertEquals(AssistantAccess.FUNCTION_IDS.toSet(), generated)
    }

    @Test
    fun `a slow core becomes a timeout and other failures stay inside app function errors`() = runBlocking {
        val hung = CoreScript(signedIn(), onSession = { suspendCancellableCoroutine { } })
        val timeout = assertThrows(androidx.appfunctions.AppFunctionAppUnknownException::class.java) {
            runBlocking { assistant(hung, timeoutMs = 30).second.getStatus() }
        }
        assertEquals("Reins did not answer in time", timeout.message)

        val leaking = CoreScript(signedIn(), onConnections = { throw RuntimeException("token SECRET") })
        val wrapped = assertThrows(androidx.appfunctions.AppFunctionAppUnknownException::class.java) {
            runBlocking { assistant(leaking).second.listConnectedAis() }
        }
        assertFalse(wrapped.message.orEmpty().contains("SECRET"))

        val cancelled = CoreScript(signedIn(), onSession = { throw CancellationException("stopped") })
        val stop = assertThrows(CancellationException::class.java) {
            runBlocking { assistant(cancelled).second.getStatus() }
        }
        assertEquals("stopped", stop.message)
    }

    /** FakeCore is final. This forwards every call and replaces the few a test needs to fail. */
    private class CoreScript(
        private val inner: ReinsCoreInterface,
        private val onSession: (suspend () -> SessionInfo?)? = null,
        private val onConnections: (suspend () -> List<ConnectionView>)? = null,
        private val onPending: (suspend () -> Unit)? = null,
        private val onDeny: (suspend () -> Unit)? = null,
    ) : ReinsCoreInterface by inner {
        override suspend fun session(): SessionInfo? = onSession?.invoke() ?: inner.session()
        override suspend fun connections(): List<ConnectionView> = onConnections?.invoke() ?: inner.connections()
        override suspend fun pending(): List<PendingItem> {
            onPending?.invoke()
            return inner.pending()
        }
        override suspend fun denyWithNote(requestId: String, note: String?) {
            onDeny?.invoke()
            inner.denyWithNote(requestId, note)
        }
    }

    private fun pendingIntent(): PendingIntent {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        return PendingIntent.getBroadcast(context, 0, Intent("dev.reins.android.TEST"), PendingIntent.FLAG_IMMUTABLE)
    }

    private fun assertDenied(message: String, block: suspend () -> Unit) {
        val error = assertThrows(androidx.appfunctions.AppFunctionElementNotFoundException::class.java) { runBlocking { block() } }
        assertEquals(message, error.message)
    }

    private fun assertNotFound(message: String, block: suspend () -> Unit) = assertDenied(message, block)
}
