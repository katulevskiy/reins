package dev.reins.android

import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.AuthResult
import dev.reins.core.ActivityExportFormat
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Settings > This phone > Export decision history: a row, a choice of format, the phone's screen lock, the core's
 * export and then Android's save picker (nothing is written to the app's own storage).
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class SettingsExportFlowTest : FlowHarness() {
    private fun openChoice() {
        core.exportRequests.clear()
        core.exportError = null
        launch()
        tap("tabSettings")
        tap("exportDecisionsRow")
        awaitTag("exportSheet")
    }

    @Test
    fun settingsHasTheRowWithItsSubtitle() {
        launch()
        tap("tabSettings")
        awaitTag("exportDecisionsRow")
        assertTrue(showsText("Export decision history"))
        assertTrue(showsText("Save this phone's Activity as a file"))
    }

    @Test
    fun theRowOpensTheChoiceWithTheWarningAndBothFormats() {
        openChoice()
        assertTrue(showsText("Includes up to the last 5,000 decisions on this phone.", substring = true))
        assertTrue(showsText("Message contents aren't included.", substring = true))
        assertTrue(showsText("The file isn't encrypted, so keep it somewhere safe.", substring = true))
        assertTrue(showsText("Spreadsheet (CSV)"))
        assertTrue(showsText("JSON"))
        assertTrue(has("exportCancel"))
        tap("exportCancel")
        awaitGone("exportSheet")
        assertTrue("cancelling exports nothing", core.exportRequests.isEmpty())
    }

    @Test
    fun choosingCsvAsksTheCoreForCsvAndOpensTheSavePicker() {
        core.activity = listOf(TestData.entry(1), TestData.entry(2))
        openChoice()
        tap("exportCsv")
        awaitCore { core.exportRequests.isNotEmpty() }
        assertEquals(listOf<Pair<ActivityExportFormat, dev.reins.core.ActivityQuery?>>(ActivityExportFormat.CSV to null), core.exportRequests.toList())
        assertEquals("the screen lock came first", 1, prompts.get())
        val picker = awaitPicker()
        assertEquals("reins-decisions.csv", picker.getStringExtra(Intent.EXTRA_TITLE))
        assertEquals("text/csv", picker.type)
    }

    @Test
    fun choosingJsonOpensTheSavePickerForJson() {
        core.activity = listOf(TestData.entry(1))
        openChoice()
        tap("exportJson")
        awaitCore { core.exportRequests.isNotEmpty() }
        assertEquals(ActivityExportFormat.JSON, core.exportRequests.first().first)
        assertEquals("application/json", awaitPicker().type)
    }

    @Test
    fun noDecisionsSaysSoAndOpensNoPicker() {
        core.activity = emptyList()
        openChoice()
        tap("exportCsv")
        awaitText("No decisions to export yet")
        assertEquals(1, core.exportRequests.size)
        assertFalse(has("exportSheet"))
        assertNull(peekCreateDocument())
    }

    @Test
    fun aRefusedScreenLockExportsNothing() {
        core.activity = listOf(TestData.entry(1))
        authResult = AuthResult.Cancelled
        openChoice()
        tap("exportCsv")
        awaitCore { prompts.get() == 1 }
        settle()
        assertTrue(core.exportRequests.isEmpty())
        assertNull(peekCreateDocument())
    }

    @Test
    fun aCoreFailureIsShownAsAnError() {
        core.activity = listOf(TestData.entry(1))
        openChoice()
        core.exportError = dev.reins.core.CoreException.Storage("disk full")
        tap("exportJson")
        awaitText("Storage problem", substring = true)
        assertNull(peekCreateDocument())
    }

    /** Waits for the save picker the app asks Android for (ACTION_CREATE_DOCUMENT) and returns its intent. */
    private fun awaitPicker(): Intent {
        var found: Intent? = null
        rule.waitUntil(10_000) {
            settle()
            found = peekCreateDocument()
            found != null
        }
        return found!!
    }

    /** The save picker the app asked Android for, if it did (ACTION_CREATE_DOCUMENT). */
    private fun peekCreateDocument(): Intent? {
        val started = nextStarted() ?: return null
        return started.takeIf { it.action == Intent.ACTION_CREATE_DOCUMENT }
    }
}
