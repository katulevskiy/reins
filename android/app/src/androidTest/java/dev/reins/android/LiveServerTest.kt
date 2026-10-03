package dev.rewarden.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.rewarden.android.platform.KeystoreKeyWrapper
import dev.rewarden.core.CoreException
import dev.rewarden.core.ForeignException
import dev.rewarden.core.GoogleTokenProvider
import dev.rewarden.core.Notifier
import dev.rewarden.core.PendingItem
import dev.rewarden.core.PendingKind
import dev.rewarden.core.RewardenCore
import java.io.File
import java.util.UUID
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeNotNull
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The phone half of `cargo run -p rewarden-e2e --example device_smoke`: the real core on a real Android runtime
 * signs in to a real server and approves an AI client's connection. Skipped unless the host passes its details:
 * `-Pandroid.testInstrumentationRunnerArguments.live.server=... live.email=...`
 * (and `adb reverse tcp:PORT tcp:PORT` so the emulator reaches the host's loopback).
 */
@RunWith(AndroidJUnit4::class)
class LiveServerTest {
    private val args = InstrumentationRegistry.getArguments()

    private object NoGoogle : GoogleTokenProvider {
        override suspend fun accessToken(): String = throw ForeignException.NeedsUserInteraction()
    }

    private object Quiet : Notifier {
        override fun itemPending(item: PendingItem) = Unit
        override fun itemResolved(id: String) = Unit
        override fun autoDecided(decision: dev.rewarden.core.AutoDecisionView) = Unit
        override fun autopilotChanged(event: dev.rewarden.core.AutopilotEvent) = Unit
    }

    @Test
    fun signInReceiveAndApproveAConnection() = runBlocking {
        val server = args.getString("live.server")
        val email = args.getString("live.email")
        val password = args.getString("live.password") ?: "correct horse battery staple"
        assumeNotNull(server, email, password)

        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val dir = File(context.cacheDir, "live-${UUID.randomUUID()}").also { it.mkdirs() }
        try {
            val core = RewardenCore(dir.absolutePath, KeystoreKeyWrapper("rewarden_live_${UUID.randomUUID()}"), NoGoogle, Quiet)
            val session = core.login(server!!, email!!, password!!, null)
            assertEquals(email, session.email)
            core.registerDevice(null)

            // The connection request arrives over the long poll.
            var item: PendingItem? = null
            val deadline = System.currentTimeMillis() + 120_000
            while (item == null && System.currentTimeMillis() < deadline) {
                item = core.sync(5u).firstOrNull { it.kind == PendingKind.PAIRING }
            }
            assertNotNull("no connection request arrived", item)
            val view = core.pairingView(item!!.id)
            val choices = view.choices.map { it.toInt() and 0xFF }
            // The host writes the number its browser shows (adb run-as ... files/live-code).
            val codeFile = File(context.filesDir, "live-code")
            val codeDeadline = System.currentTimeMillis() + 60_000
            while (!codeFile.exists() && System.currentTimeMillis() < codeDeadline) kotlinx.coroutines.delay(200)
            val code = codeFile.readText().trim().toInt()
            assertTrue("browser code $code not among $choices", code in choices)

            core.answerPairing(item.id, true, code.toUByte(), "Emulator")
            assertTrue(core.pending().none { it.id == item.id })
            assertEquals(listOf("Emulator"), core.connections().map { it.label })

            // Let the AI's follow-up request (Gmail is unavailable here) be fetched and answered.
            val until = System.currentTimeMillis() + 15_000
            while (System.currentTimeMillis() < until) {
                try {
                    core.sync(3u)
                } catch (e: CoreException.Network) {
                    break // the host shut its server down: nothing more to fetch
                }
            }
            core.close()
        } finally {
            dir.deleteRecursively()
        }
    }
}
