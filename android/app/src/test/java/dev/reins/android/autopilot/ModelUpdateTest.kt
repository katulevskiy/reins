package dev.reins.android.autopilot

import dev.reins.android.FakeCore
import dev.reins.android.TestData
import dev.reins.core.CoreException
import dev.reins.core.ModelState
import dev.reins.core.ModelUpdateOutcome
import dev.reins.core.SessionInfo
import androidx.work.NetworkType
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/** When the model may update by itself, and what a background run does with the core's answer. */
class ModelUpdateTest {
    private val wifi = ModelUpdateGate.Conditions(online = true, unmetered = true, powerSave = false, dataSaver = false)
    private val cellular = wifi.copy(unmetered = false)
    private val offline = ModelUpdateGate.Conditions(online = false, unmetered = false, powerSave = false, dataSaver = false)

    private val core = FakeCore()
    private var refreshes = 0

    @Before
    fun signedInWithAnOlderModel() {
        core.session = SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u, update = true)
    }

    private fun run(
        conditions: ModelUpdateGate.Conditions = wifi,
        downloadRunning: Boolean = false,
        manual: Boolean = false,
        wifiOnly: Boolean = true,
    ) = runBlocking { runModelUpdate(core, conditions, downloadRunning, manual, wifiOnly) { refreshes++ } }

    // ---- the gate ---------------------------------------------------------------------------------------------------

    @Test
    fun `an automatic update downloads on wifi only, off battery saver and data saver`() {
        assertTrue(ModelUpdateGate.allows(wifi))
        assertFalse("on mobile data it waits", ModelUpdateGate.allows(cellular))
        assertFalse("offline it waits", ModelUpdateGate.allows(offline))
        assertFalse("battery saver: it waits", ModelUpdateGate.allows(wifi.copy(powerSave = true)))
        assertFalse("data saver: it waits", ModelUpdateGate.allows(wifi.copy(dataSaver = true)))
        assertFalse(ModelUpdateGate.allows(wifi.copy(online = false)))
    }

    @Test
    fun `check now uses the network the first download may use`() {
        assertTrue(ModelUpdateGate.checkNowAllows(wifi, wifiOnly = true))
        assertTrue(ModelUpdateGate.checkNowAllows(wifi, wifiOnly = false))
        assertFalse("Wi-Fi only: mobile data waits", ModelUpdateGate.checkNowAllows(cellular, wifiOnly = true))
        assertTrue("mobile data allowed by the user", ModelUpdateGate.checkNowAllows(cellular, wifiOnly = false))
        assertFalse(ModelUpdateGate.checkNowAllows(offline, wifiOnly = false))
        assertTrue("the user asked: savers do not stop it", ModelUpdateGate.checkNowAllows(wifi.copy(powerSave = true, dataSaver = true), wifiOnly = true))
    }

    // ---- a background run -------------------------------------------------------------------------------------------

    @Test
    fun `nothing pending, the core is not asked to update`() {
        core.updatePending = false
        assertEquals(UpdateRun.Done, run())
        assertTrue(core.updateCalls.isEmpty())
    }

    @Test
    fun `signed out, nothing happens`() {
        core.session = null
        assertEquals(UpdateRun.Done, run())
        assertTrue(core.updateCalls.isEmpty())
    }

    @Test
    fun `pending on wifi, an automatic update, then autopilot is read again`() {
        assertEquals(UpdateRun.Done, run())
        assertEquals(listOf(true), core.updateCalls.toList())
        assertEquals(1, refreshes)
        assertFalse(core.model.update)
        assertEquals("2", core.model.version)
    }

    @Test
    fun `pending but not on wifi, try again later, nothing downloaded`() {
        listOf(cellular, offline).forEach {
            assertEquals(it.toString(), UpdateRun.Retry, run(it))
        }
        assertTrue(core.updateCalls.isEmpty())
    }

    @Test
    fun `battery saver or data saver ends the run without a retry, the daily run looks again`() {
        listOf(wifi.copy(powerSave = true), wifi.copy(dataSaver = true), cellular.copy(powerSave = true)).forEach {
            assertEquals(it.toString(), UpdateRun.Done, run(it))
        }
        assertTrue("nothing downloaded", core.updateCalls.isEmpty())
    }

    @Test
    fun `a run stopped by workmanager pauses the core's download`() {
        core.updateHangs = true
        runBlocking {
            val job = launch { runModelUpdate(core, wifi, downloadRunning = false) }
            while (core.updateCalls.isEmpty()) delay(5)
            job.cancelAndJoin()
        }
        assertEquals("what arrived is kept for the next run", 1, core.pauses)
    }

    @Test
    fun `check now honours download on wifi only`() {
        core.autoUpdate = false
        assertEquals("wifi only: mobile data waits", UpdateRun.Retry, run(cellular, manual = true, wifiOnly = true))
        assertTrue(core.updateCalls.isEmpty())
        assertEquals("allowed by the user: mobile data will do", UpdateRun.Done, run(cellular, manual = true, wifiOnly = false))
        assertEquals(listOf(false), core.updateCalls.toList())
        // What the waiting run asks WorkManager for.
        assertEquals(NetworkType.UNMETERED, WorkModelUpdates.network(manual = true, wifiOnly = true))
        assertEquals(NetworkType.CONNECTED, WorkModelUpdates.network(manual = true, wifiOnly = false))
        assertEquals("automatic runs always wait for wifi", NetworkType.UNMETERED, WorkModelUpdates.network(manual = false, wifiOnly = false))
    }

    @Test
    fun `the first download running, try again later`() {
        assertEquals(UpdateRun.Retry, run(downloadRunning = true))
        assertTrue(core.updateCalls.isEmpty())
    }

    @Test
    fun `a dropped connection or a busy host is retried`() {
        core.updateFailure = CoreException.Network("connection reset")
        assertEquals(UpdateRun.Retry, run())
        core.updateFailure = CoreException.Server(503u, "unavailable")
        assertEquals(UpdateRun.Retry, run())
        assertEquals(2, refreshes)
    }

    @Test
    fun `a model that fails its check is not retried and the old one stays`() {
        core.updateFailure = CoreException.Service("sha-256 of model.onnx does not match the pinned hash")
        assertEquals(UpdateRun.Done, run())
        assertEquals(ModelState.INSTALLED, core.model.state)
        assertTrue(core.model.update)
        core.updateFailure = CoreException.Invalid("the model is already being downloaded")
        assertEquals(UpdateRun.Done, run())
    }

    @Test
    fun `a check now that waited for wifi runs even with update automatically off`() {
        core.autoUpdate = false
        assertEquals("the automatic run has nothing to do", UpdateRun.Done, run())
        assertTrue(core.updateCalls.isEmpty())
        assertEquals(UpdateRun.Retry, run(cellular, manual = true))
        assertEquals(UpdateRun.Done, run(wifi.copy(powerSave = true), manual = true))
        assertEquals(listOf(false), core.updateCalls.toList())
        assertEquals("2", core.model.version)
    }

    @Test
    fun `check now says what it found in a few words`() {
        assertEquals("Up to date", CheckNow.outcome(ModelUpdateOutcome.UP_TO_DATE))
        assertEquals("Updated", CheckNow.outcome(ModelUpdateOutcome.UPDATED))
        assertEquals("Not available yet", CheckNow.outcome(ModelUpdateOutcome.UNAVAILABLE))
        assertNull(CheckNow.outcome(ModelUpdateOutcome.OFF))
        assertEquals("Waiting for Wi-Fi", CheckNow.WAITING)
    }
}
