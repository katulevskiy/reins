package dev.reins.android

import androidx.compose.ui.test.assertContentDescriptionContains
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.work.NetworkType
import androidx.work.WorkManager
import androidx.work.testing.WorkManagerTestInitHelper
import dev.reins.android.autopilot.ModelUpdateGate
import dev.reins.android.autopilot.NetworkConditions
import dev.reins.android.autopilot.WorkModelDownloads
import dev.reins.android.autopilot.WorkModelUpdates
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.FeedbackProvider
import androidx.compose.ui.test.performTouchInput
import dev.reins.core.AutopilotMode
import dev.reins.core.CoreException
import dev.reins.core.ModelState
import dev.reins.core.Preset
import dev.reins.core.Verdict
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import dev.reins.android.platform.AuthResult
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Autopilot end to end over the fake core: modes, the model, profiles, "Try it", connections, approvals, activity. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class AutopilotFlowTest : FlowHarness() {
    private val heard = RecordingFeedback()
    private val now = 1_700_000_100L

    @Before
    fun listen() {
        dev.reins.android.TestNativeKeys.install()
        FeedbackProvider.observer = heard
    }

    @After
    fun stopListening() {
        FeedbackProvider.observer = null
        NetworkConditions.override = null
    }

    /** A tag inside a row that reads as one (its parts are merged for accessibility). */
    private fun awaitInside(tag: String) {
        rule.waitUntil(10_000) {
            settle()
            rule.onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
    }

    private fun network(online: Boolean = true, unmetered: Boolean = true, powerSave: Boolean = false, dataSaver: Boolean = false) {
        NetworkConditions.override = { ModelUpdateGate.Conditions(online, unmetered, powerSave, dataSaver) }
    }

    private fun installModel() {
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u)
    }

    private fun openAutopilot() {
        launch()
        tap("tabTrust")
        awaitTag("mode:MANUAL")
    }

    // ---- modes ------------------------------------------------------------------------------------------------------

    @Test
    fun theModeButtonOpensTheQuickMenuAndMoreOpensAutopilot() {
        installModel()
        launch()
        awaitTag("modePill")
        rule.onNodeWithContentDescription("Autopilot: Manual").assertExists()
        tap("modePill")
        awaitTag("autopilotQuick")
        assertFalse("the full page waits for More", has("modeHero"))
        tap("quickMore")
        awaitTag("modeHero")
        rule.onNodeWithTag("mode:MANUAL").assertIsSelected()
        assertFalse(has("autopilotQuick"))
    }

    @Test
    fun theQuickMenuSetsAModeInOneTapAndCloses() {
        installModel()
        launch()
        tap("modePill")
        heard.clear()
        tap("quick:AUTO")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals(Triple(null, AutopilotMode.AUTO, null), core.modeCalls.single())
        assertTrue(heard.played(Event.AutopilotOn))
        awaitGone("autopilotQuick")
        tap("modePill")
        tap("quick:LOCKDOWN")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals("Lockdown only takes power away: no confirmation", AutopilotMode.LOCKDOWN, core.modeCalls.last().second)
    }

    @Test
    fun bypassFromTheQuickMenuOpensTheSheetAndIsHeld() {
        installModel()
        launch()
        tap("modePill")
        tap("quick:BYPASS")
        awaitTag("bypassSheet")
        awaitGone("autopilotQuick")
        assertTrue("the shared sheet, with its own lengths", has("bypassLength:custom"))
        assertTrue("nothing changes before the hold", core.modeCalls.isEmpty())
        tap("bypassLength:15")
        hold("confirmBypass")
        awaitCore { core.bypassCalls.isNotEmpty() }
        assertEquals(null to 15u, core.bypassCalls.single())
        assertEquals("checked from the quick menu too", 1, prompts.get())
        awaitGone("bypassSheet")
        tap("modePill")
        // The running bypass is the Bypass button itself: no separate Stop.
        awaitText("Tap to stop")
        assertFalse(has("quickStopBypass"))
        tap("quick:BYPASS")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals("back to the mode it interrupted", AutopilotMode.MANUAL, core.modeCalls.last().second)
    }

    @Test
    fun theMenusBypassOpensTheSheetAndCancelChangesNothing() {
        installModel()
        launch()
        tap("modePill")
        tap("quick:BYPASS")
        awaitTag("bypassSheet")
        assertTrue(core.modeCalls.isEmpty())
        tap("cancelBypass")
        awaitGone("bypassSheet")
        assertTrue(core.modeCalls.isEmpty())
        tap("modePill")
        tap("quick:BYPASS")
        hold("confirmBypass")
        awaitCore { core.bypassCalls.isNotEmpty() }
        assertEquals("1 hour unless another is picked", null to 60u, core.bypassCalls.single())
        assertEquals(1, prompts.get())
    }

    @Test
    fun theModeSheetSaysWhatEachModeDoes() {
        installModel()
        launch()
        tap("modePill")
        awaitTag("autopilotQuick")
        // One sheet, every mode with its line (Next, Activity 03).
        for (line in listOf("You decide", "Easy ones decided", "Risky ones wait", "Everything denied", "Everything allowed")) awaitText(line)
        tap("quick:LOCKDOWN")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals(AutopilotMode.LOCKDOWN, core.modeCalls.single().second)
        awaitGone("autopilotQuick")
    }

    @Test
    fun aRunningBypassTurnsThePillIntoACountdown() {
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 14 * 60 + 32)
        launch()
        awaitTag("modePill")
        awaitText("14:32")
    }

    @Test
    fun aBypassWithoutAnEndShowsNoCountdown() {
        core.apGlobal = FakeCore.ApRow(bypassUntil = Long.MAX_VALUE, bypassSince = now - 2 * 86_400 - 60)
        launch()
        awaitTag("modePill")
        rule.onNodeWithContentDescription("Autopilot: Bypass").assertExists()
        assertFalse("no countdown to a time that never comes", has("quickBypassLeft"))
        tap("tabTrust")
        awaitTag("bypassUntilOff")
        rule.onNodeWithTag("bypassLine", useUnmergedTree = true).assertTextContains("On until you stop it · 2 days so far")
        tap("stopBypass")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals(Triple(null, AutopilotMode.MANUAL, null), core.modeCalls.single())
        assertEquals("stopping needs no check", 0, prompts.get())
    }

    /** Switching back and forth leaves one card chosen and neither stuck pressed (dimmed) nor disabled. */
    @Test
    fun manualAndAutoSwitchBackAndForthAndStayClean() {
        installModel()
        openAutopilot()
        for ((i, mode) in listOf(AutopilotMode.AUTO, AutopilotMode.MANUAL, AutopilotMode.AUTO, AutopilotMode.MANUAL).withIndex()) {
            tap("mode:${mode.name}")
            awaitCore { core.modeCalls.size == i + 1 && container.state.autopilot.value?.mode == mode }
            settle()
            val other = if (mode == AutopilotMode.AUTO) AutopilotMode.MANUAL else AutopilotMode.AUTO
            rule.onNodeWithTag("mode:${mode.name}").assertIsSelected().assertIsEnabled()
            rule.onNodeWithTag("mode:${other.name}").assertIsNotSelected().assertIsEnabled()
        }
        assertEquals(listOf(AutopilotMode.AUTO, AutopilotMode.MANUAL, AutopilotMode.AUTO, AutopilotMode.MANUAL), core.modeCalls.map { it.second })
    }

    @Test
    fun pickingAutoSetsTheGlobalModeAndFeelsLikeMoreAutonomy() {
        installModel()
        openAutopilot()
        heard.clear()
        tap("mode:AUTO")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals(Triple(null, AutopilotMode.AUTO, null), core.modeCalls.single())
        assertTrue(heard.played(Event.AutopilotOn))
        awaitCore { container.state.autopilot.value?.mode == AutopilotMode.AUTO }
        tap("mode:MANUAL")
        awaitCore { core.modeCalls.size == 2 }
        assertTrue(heard.played(Event.AutopilotOff))
    }

    @Test
    fun bypassAsksHowLongThenCountsDownAndStops() {
        installModel()
        openAutopilot()
        tap("mode:BYPASS")
        awaitTag("bypassSheet")
        assertTrue("nothing changes before the hold", core.modeCalls.isEmpty())
        rule.onNodeWithTag("bypassLength:60").assertIsSelected()
        tap("bypassLength:15")
        heard.clear()
        hold("confirmBypass")
        awaitCore { core.bypassCalls.isNotEmpty() }
        assertEquals(null to 15u, core.bypassCalls.single())
        assertEquals("the owner is checked first", listOf("Turn on Bypass for every AI" to "For 15 min"), promptTexts.toList())
        assertTrue(heard.played(Event.BypassOn))
        awaitTag("bypassClock")
        rule.onNodeWithTag("bypassClock").assertTextContains("15:00")
        heard.clear()
        tap("stopBypass")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals("back to the mode it interrupted", Triple(null, AutopilotMode.MANUAL, null), core.modeCalls.last())
        assertTrue(heard.played(Event.BypassOff))
        awaitGone("bypassClock")
    }

    @Test
    fun aBypassWithoutTheOwnersCheckChangesNothing() {
        installModel()
        authResult = dev.reins.android.platform.AuthResult.Cancelled
        openAutopilot()
        tap("mode:BYPASS")
        heard.clear()
        hold("confirmBypass")
        awaitCore { prompts.get() == 1 }
        settle()
        assertTrue("nothing reaches the core", core.bypassCalls.isEmpty() && core.modeCalls.isEmpty())
        assertEquals("nothing shows, not even for a moment", AutopilotMode.MANUAL, container.state.autopilot.value?.mode)
        assertFalse("and nothing sounds", heard.played(Event.BypassOn))
        assertFalse(has("bypassClock"))

        // No screen lock at all: it says so, and still nothing changes.
        authResult = dev.reins.android.platform.AuthResult.Unavailable
        tap("mode:BYPASS")
        tap("bypassLength:custom")
        rule.onNodeWithTag("bypassFor:untilOff").performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.OnClick)
        hold("confirmBypassFor")
        awaitTag("autopilotError")
        awaitText("screen lock", substring = true)
        assertEquals(2, prompts.get())
        assertEquals("Until you turn it off", promptTexts.last().second)
        assertTrue(core.bypassCalls.isEmpty())
        assertEquals(AutopilotMode.MANUAL, container.state.autopilot.value?.mode)
    }

    @Test
    fun aBypassOfYourOwnLengthIsPickedUnderCustom() {
        installModel()
        openAutopilot()
        tap("mode:BYPASS")
        tap("bypassLength:custom")
        awaitTag("bypassForSheet")
        // The sheet slides up first.
        rule.mainClock.advanceTimeBy(600)
        // Its middle is the stepper: the row is picked by what a screen reader does.
        rule.onNodeWithTag("bypassFor:own").performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.OnClick)
        tap("bypassStepper+")
        rule.waitUntil(10_000) {
            settle()
            rule.onAllNodes(androidx.compose.ui.test.hasTestTag("bypassCount") and androidx.compose.ui.test.hasText("4"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        tap("closeSheet")
        awaitGone("bypassForSheet")
        rule.onNodeWithTag("bypassLength:custom").assertIsSelected()
        awaitText("4 days")
        hold("confirmBypass")
        awaitCore { core.bypassCalls.isNotEmpty() }
        assertEquals(null to 4u * 1_440u, core.bypassCalls.single())
    }

    @Test
    fun aBypassUntilTurnedOffIsWarnedHeldAndShowsHowLongItRan() {
        installModel()
        openAutopilot()
        tap("mode:BYPASS")
        tap("bypassLength:custom")
        tap("bypassFor:untilOff")
        awaitTag("untilOffWarning")
        awaitText("Everything is approved without asking until you stop it.")
        awaitText("Hold to bypass until off")
        assertTrue(core.bypassCalls.isEmpty())
        hold("confirmBypassFor")
        awaitCore { core.bypassCalls.isNotEmpty() }
        assertEquals("no end", null to null, core.bypassCalls.single())
        awaitTag("bypassUntilOff")
        rule.onNodeWithTag("bypassLine", useUnmergedTree = true).assertTextContains("On until you stop it", substring = true)
        assertTrue((container.state.autopilot.value?.bypassUntil ?: 0) == Long.MAX_VALUE)
        tap("stopBypass")
        awaitGone("bypassUntilOff")
    }

    @Test
    fun lockdownIsOneTapAndEndingItAsksTheOwner() {
        openAutopilot()
        heard.clear()
        tap("mode:LOCKDOWN")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals("taking power away needs no check", 0, prompts.get())
        assertEquals(AutopilotMode.LOCKDOWN, core.modeCalls.single().second)
        assertTrue(heard.played(Event.LockdownOn))
        awaitTag("endLockdown")
        // Ending it loosens: without the owner's pass Lockdown stays.
        authResult = AuthResult.Cancelled
        tap("endLockdown")
        awaitCore { prompts.get() == 1 }
        settle()
        assertEquals(1, core.modeCalls.size)
        authResult = AuthResult.Success
        tap("endLockdown")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals(2, prompts.get())
        assertEquals(AutopilotMode.MANUAL, core.modeCalls.last().second)
    }

    @Test
    fun withoutTheModelAutoSaysSoAndTheModeIsManual() {
        openAutopilot()
        awaitText("Needs the model")
        rule.onNodeWithTag("mode:MANUAL").assertIsSelected()
        awaitTag("getScore")
        awaitText("after download")
        assertFalse("the score card takes the strip's place", has("statScored"))
    }

    @Test
    fun theWeekIsCountedOnThePage() {
        installModel()
        openAutopilot()
        awaitTag("statScored")
        rule.onNodeWithTag("statScored").assertTextContains("142", substring = true)
        rule.onNodeWithTag("statMatched").assertTextContains("94%", substring = true)
        rule.onNodeWithTag("statKinds").assertTextContains("2", substring = true)
        rule.onNodeWithTag("profileLine:personal", useUnmergedTree = true).assertTextContains("64 decisions", substring = true)
    }

    // ---- the model --------------------------------------------------------------------------------------------------

    private fun runDownloadJob() {
        val infos = WorkManager.getInstance(context).getWorkInfosForUniqueWork(WorkModelDownloads.NAME).get()
        WorkManagerTestInitHelper.getTestDriver(context)!!.setAllConstraintsMet(infos.single().id)
    }

    @Test
    fun theModelDownloadsOnWifiAndShowsWhenItIsReady() {
        openAutopilot()
        tap("getModel")
        awaitText("Waiting for Wi-Fi")
        val request = WorkManager.getInstance(context).getWorkInfosForUniqueWork(WorkModelDownloads.NAME).get().single()
        assertEquals(NetworkType.UNMETERED, request.constraints.requiredNetworkType)
        assertEquals("the core downloads nothing before the network is there", 0, core.downloads.get())
        runDownloadJob()
        awaitCore { core.downloads.get() == 1 }
        awaitInside("modelInstalled")
        tap("openModel")
        awaitTag("deleteModel")
    }

    @Test
    fun mobileDataCanBeAllowed() {
        openAutopilot()
        tap("openModel")
        tap("wifiOnly")
        awaitCore { !core.wifiOnly }
        tap("closeSheet")
        awaitGone("modelSheet")
        tap("getModel")
        awaitText("Waiting for a connection")
        val request = WorkManager.getInstance(context).getWorkInfosForUniqueWork(WorkModelDownloads.NAME).get().single()
        assertEquals(NetworkType.CONNECTED, request.constraints.requiredNetworkType)
    }

    @Test
    fun aDownloadThatFailsItsCheckSaysSoAndCanBeRetried() {
        core.downloadFailure = CoreException.Invalid("checksum mismatch")
        core.downloadError = "sha-256 of model.onnx does not match the pinned hash"
        openAutopilot()
        tap("getModel")
        awaitText("Waiting for Wi-Fi")
        runDownloadJob()
        awaitTag("getModelError")
        awaitText("did not match", substring = true)
        awaitText("Try again")
    }

    @Test
    fun aWaitingDownloadCanBeCancelled() {
        openAutopilot()
        tap("getModel")
        awaitTag("pauseDownload")
        tap("pauseDownload")
        awaitTag("getModel")
        assertEquals(0, core.downloads.get())
    }

    @Test
    fun theModelCanBeDeleted() {
        installModel()
        openAutopilot()
        tap("openModel")
        tap("deleteModel")
        awaitText("Delete the model?")
        rule.onNodeWithText("Delete").performClick()
        awaitCore { core.model.state == ModelState.NOT_INSTALLED }
        awaitTag("getModel")
        assertTrue(heard.played(Event.Revoked))
    }

    @Test
    fun theModelRowShowsItsVersionAndWhenItWasUpdatedWithNoUpdateButton() {
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u, update = true, updatedAt = now)
        openAutopilot()
        awaitInside("modelInstalled")
        rule.onNodeWithTag("modelVersion", useUnmergedTree = true).assertTextEquals("v1")
        awaitText("Updated today")
        awaitTag("statScored")
        assertFalse("a newer model comes by itself: nothing to tap", has("updateModel"))
        assertFalse(has("modelUpdateWaiting"))
        tap("openModel")
        awaitTag("deleteModel")
        assertFalse(has("updateModelDetails"))
        assertFalse(showsText("A newer model is available", substring = true))
    }

    @Test
    fun whileTheUpdateDownloadsTheRowShowsHowFarAndTheOlderModelStillScores() {
        core.model = TestData.modelStatus(ModelState.DOWNLOADING, downloaded = 120_000_000u, size = 412_000_000u, update = true)
        openAutopilot()
        awaitText("29%")
        awaitTag("statScored")
        assertFalse("no offer to get the model while the older one still works", has("getScore"))
    }

    @Test
    fun theAppSchedulesQuietUpdatesOnWifiWithBatteryAndStorageToSpare() {
        installModel()
        openAutopilot()
        val work = WorkManager.getInstance(context)
        awaitCore {
            work.getWorkInfosForUniqueWork(WorkModelUpdates.NOW).get().isNotEmpty() &&
                work.getWorkInfosForUniqueWork(WorkModelUpdates.DAILY).get().isNotEmpty()
        }
        val infos = work.getWorkInfosForUniqueWork(WorkModelUpdates.NOW).get() + work.getWorkInfosForUniqueWork(WorkModelUpdates.DAILY).get()
        infos.forEach {
            assertEquals(NetworkType.UNMETERED, it.constraints.requiredNetworkType)
            assertTrue(it.constraints.requiresBatteryNotLow())
            assertTrue(it.constraints.requiresStorageNotLow())
        }
        assertTrue("nothing is downloaded before Wi-Fi", core.updateCalls.isEmpty())
    }

    @Test
    fun updateAutomaticallyIsOnAndCanBeTurnedOff() {
        installModel()
        openAutopilot()
        tap("openModel")
        awaitTag("autoUpdate")
        tap("autoUpdate")
        awaitCore { core.autoUpdateCalls.toList() == listOf(false) }
        assertFalse(core.autoUpdate)
        assertEquals(false, container.state.autopilot.value?.autoUpdate)
    }

    @Test
    fun checkNowIsBehindMoreAndUpdatesTheModel() {
        network()
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u, update = true)
        openAutopilot()
        tap("openModel")
        awaitTag("modelMore")
        assertFalse("More starts closed", has("checkNow"))
        tap("modelMore")
        tap("checkNow")
        awaitTag("checkResult")
        rule.onNodeWithTag("checkResult", useUnmergedTree = true).assertTextEquals("Updated")
        assertEquals("the user's check, not the automatic one", listOf(false), core.updateCalls.toList())
        assertEquals("2", core.model.version)
    }

    @Test
    fun checkNowSaysWhenTheModelIsUpToDate() {
        network(powerSave = true, dataSaver = true)
        installModel()
        openAutopilot()
        tap("openModel")
        tap("modelMore")
        tap("checkNow")
        awaitTag("checkResult")
        rule.onNodeWithTag("checkResult", useUnmergedTree = true).assertTextEquals("Up to date")
        assertEquals("Battery Saver and Data Saver do not stop the user's check", listOf(false), core.updateCalls.toList())
    }

    @Test
    fun checkNowOnMobileDataWaitsForWifiInTheBackground() {
        network(unmetered = false)
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u, update = true)
        openAutopilot()
        tap("openModel")
        tap("modelMore")
        tap("checkNow")
        awaitTag("checkResult")
        rule.onNodeWithTag("checkResult", useUnmergedTree = true).assertTextEquals("Waiting for Wi-Fi")
        assertTrue("nothing downloads on mobile data", core.updateCalls.isEmpty())
        val waiting = WorkManager.getInstance(context).getWorkInfosForUniqueWork(WorkModelUpdates.NOW).get()
        assertTrue(waiting.isNotEmpty())
        waiting.forEach { assertEquals(NetworkType.UNMETERED, it.constraints.requiredNetworkType) }
    }

    @Test
    fun aCheckNowThatFailsSaysWhyAndKeepsTheModel() {
        network()
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u, update = true)
        core.updateFailure = CoreException.Service("the newer model failed its self-check")
        openAutopilot()
        tap("openModel")
        tap("modelMore")
        tap("checkNow")
        awaitTag("checkResult")
        rule.onNodeWithTag("checkResult", useUnmergedTree = true).assertTextContains("failed its self-check", substring = true)
        assertEquals(ModelState.INSTALLED, core.model.state)
        assertTrue(has("deleteModel"))
    }

    // ---- profiles ---------------------------------------------------------------------------------------------------

    @Test
    fun aProfileShowsItsClassesAndTakesAPreset() {
        installModel()
        openAutopilot()
        tap("profile:personal")
        awaitTag("class:github/write/push")
        rule.onNodeWithTag("classStatus:github/write/push", useUnmergedTree = true).assertTextContains("Auto")
        rule.onNodeWithTag("classStatus:gmail/read", useUnmergedTree = true).assertTextContains("Paused")
        rule.onNodeWithTag("classStatus:desktop/ask/command", useUnmergedTree = true).assertTextContains("Asks")
        rule.onNodeWithTag("classStatus:telegram/send", useUnmergedTree = true).assertTextContains("Locked by you")
        rule.onNodeWithTag("learningLine", useUnmergedTree = true).assertTextContains("Learning your preferences · 64 answers")
        rule.onNodeWithTag("statMemory").assertTextContains("64", substring = true)
        rule.onNodeWithTag("statKinds").assertTextContains("/4", substring = true)
        tap("preset:CAUTIOUS")
        awaitCore { core.presets.isNotEmpty() }
        assertEquals("personal" to Preset.CAUTIOUS, core.presets.single())
        awaitText("Approves when 98% sure, denies when 95% sure")
    }

    @Test
    fun customThresholdsAreSetWithSlidersAndReplayTheLastDays() {
        installModel()
        openAutopilot()
        tap("profile:personal")
        tap("preset:CUSTOM")
        awaitCore { core.presets.isNotEmpty() }
        assertEquals("personal" to Preset.CUSTOM, core.presets.single())
        awaitTag("thresholds")
        rule.onNodeWithTag("approveAt:value", useUnmergedTree = true).assertTextContains("95%")
        awaitTag("replayLine")
        rule.onNodeWithTag("replayLine").assertTextContains("Last 30 days at these:", substring = true)
        core.replay = dev.reins.core.ThresholdReplay(30u, 51u, 2u, 0u)
        rule.onNodeWithTag("approveAt").performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.SetProgress) { it(0.876f) }
        awaitCore { core.thresholdCalls.isNotEmpty() }
        assertEquals(Triple("personal", 0.88f, 0.90f), core.thresholdCalls.last())
        rule.onNodeWithTag("approveAt:value", useUnmergedTree = true).assertTextContains("88%")
        awaitText("51 decided", substring = true)
        rule.onNodeWithTag("denyAt").performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.SetProgress) { it(0.5f) }
        awaitCore { core.thresholdCalls.size == 2 }
        assertEquals("never below the 80% floor", 0.80f, core.thresholdCalls.last().third)
    }

    @Test
    fun trustingAClassByHandWarnsFirst() {
        installModel()
        openAutopilot()
        tap("profile:personal")
        tap("class:gmail/read")
        tap("lock:off:gmail/read")
        awaitText("Trust Gmail · read now?", substring = true)
        assertTrue(core.classLocks.isEmpty())
        rule.onNodeWithText("Trust").performClick()
        awaitCore { core.classLocks.isNotEmpty() }
        assertEquals(Triple("personal", "gmail/read", false), core.classLocks.single())
        tap("lock:on:gmail/read")
        awaitCore { core.classLocks.size == 2 }
        assertEquals(true, core.classLocks.last().third)
    }

    @Test
    fun aProfileCanBeCreatedMadeDefaultResetAndDeleted() {
        openAutopilot()
        tap("newProfile")
        awaitTag("profileName")
        rule.onNodeWithTag("profileName").performTextReplacement("Side project")
        tap("icon:🚀")
        tap("saveProfile")
        awaitCore { core.profileCalls.contains("create:Side project") }
        awaitTag("makeDefault")
        tap("makeDefault")
        awaitCore { core.profiles.first { it.name == "Side project" }.isDefault }
        awaitTag("defaultTag")
        tap("resetProfile")
        rule.onNodeWithText("Forget").performClick()
        awaitCore { core.profileCalls.any { it.startsWith("reset:") } }
        tap("deleteProfile")
        rule.onNodeWithText("Delete").performClick()
        awaitCore { core.profileCalls.any { it.startsWith("delete:") } }
        awaitTag("newProfile")
        assertFalse(core.profiles.any { it.name == "Side project" })
    }

    // ---- Try it -----------------------------------------------------------------------------------------------------

    @Test
    fun tryItShowsTheVerdictTheNeighboursAndTheReason() {
        installModel()
        openAutopilot()
        tap("openTryIt")
        awaitTag("situation")
        tap("evaluate")
        awaitTag("verdict")
        rule.onNodeWithTag("verdictWord").assertTextContains("Approve")
        awaitText("Push to a branch · dkat/laya")
        val (profile, situation) = core.evaluations.single()
        assertEquals("personal", profile)
        assertTrue(situation.startsWith("connection: Claude Code"))
    }

    @Test
    fun anExampleFillsTheRequestAndAnotherProfileCanBeTried() {
        installModel()
        openAutopilot()
        tap("openTryIt")
        tap("example:Read bank emails")
        tap("tryProfile:work")
        rule.onNodeWithTag("situation").performTextReplacement("connection: x\nservice: vault\noperation: Get a password")
        tap("evaluate")
        awaitTag("verdictWord")
        rule.onNodeWithTag("verdictWord").assertTextContains("Deny")
        assertEquals("work", core.evaluations.single().first)
    }

    // ---- a connection's own mode and profile -----------------------------------------------------------------------

    @Test
    fun aConnectionGetsItsOwnModeAndProfile() {
        installModel()
        launch()
        tap("tabConnections")
        tap("connection:c1")
        awaitTag("connectionMode")
        tap("connMode:AUTO")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals(Triple("c1", AutopilotMode.AUTO, null), core.modeCalls.single())
        rule.onNodeWithTag("connMode:AUTO").assertIsSelected()
        tap("learnsInto")
        tap("connProfile:work")
        awaitCore { core.assigned.isNotEmpty() }
        assertEquals("c1" to "work", core.assigned.single())
        tap("connMode:follow")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals(Triple("c1", null, null), core.modeCalls.last())
    }

    @Test
    fun aConnectionBypassThatTheCoreRefusesSaysWhy() {
        installModel()
        core.modeFailure = CoreException.Invalid("a connection paired less than 10 minutes ago cannot be put in bypass")
        launch()
        tap("tabConnections")
        tap("connection:c1")
        tap("connMode:BYPASS")
        awaitTag("bypassSheet")
        awaitText("Bypass Claude?")
        hold("confirmBypass")
        awaitCore { prompts.get() == 1 }
        assertEquals("Turn on Bypass for Claude", promptTexts.single().first)
        awaitTag("connAutopilotError")
        awaitText("10 minutes", substring = true)
        assertTrue(heard.played(Event.Error))
    }

    @Test
    fun aConnectionBypassNeedsTheOwnersCheck() {
        installModel()
        authResult = dev.reins.android.platform.AuthResult.Cancelled
        launch()
        tap("tabConnections")
        tap("connection:c1")
        tap("connMode:BYPASS")
        hold("confirmBypass")
        awaitCore { prompts.get() == 1 }
        settle()
        assertTrue(core.bypassCalls.isEmpty())
        assertEquals(null, container.state.autopilot.value?.connections?.firstOrNull { it.connectionId == "c1" }?.bypassUntil)
        authResult = dev.reins.android.platform.AuthResult.Success
        tap("connMode:BYPASS")
        hold("confirmBypass")
        awaitCore { core.bypassCalls.isNotEmpty() }
        assertEquals("c1" to 60u, core.bypassCalls.single())
    }

    @Test
    fun aConnectionBypassShowsItsTimeAndStops() {
        installModel()
        core.apConnections["c1"] = FakeCore.ApRow(mode = AutopilotMode.AUTO, bypassUntil = now + 600)
        launch()
        tap("tabConnections")
        tap("connection:c1")
        awaitTag("stopConnectionBypass")
        rule.onNodeWithTag("connectionModeLine", useUnmergedTree = true).assertTextContains("10 min left", substring = true)
        tap("stopConnectionBypass")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals("back to its own mode", Triple("c1", AutopilotMode.AUTO, null), core.modeCalls.single())
    }

    // ---- Overnight ----------------------------------------------------------------------------------------------------

    @Test
    fun overnightAsksHowLongIsHeldCheckedAndStopsToTheModeItInterrupted() {
        installModel()
        core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.AUTO)
        openAutopilot()
        tap("mode:OVERNIGHT")
        awaitTag("overnightSheet")
        rule.onNodeWithTag("overnightTitle").assertTextContains("Overnight for every AI?")
        assertTrue("nothing changes before the hold", core.modeCalls.isEmpty())
        rule.onNodeWithTag("overnightLength:60").assertIsSelected()
        tap("overnightLength:15")
        heard.clear()
        hold("confirmOvernight")
        awaitCore { core.overnightCalls.isNotEmpty() }
        assertEquals(null to 15u, core.overnightCalls.single())
        assertEquals("the core asks the owner, in its own words", listOf("Turn on Overnight for every AI" to "Start Overnight for 15 min"), promptTexts.toList())
        assertTrue("through the phone's own prompt", core.overnightOwners.single() is dev.reins.android.platform.OwnerPrompt)
        assertTrue(heard.played(Event.AutopilotOn))
        awaitTag("overnightClock")
        rule.onNodeWithTag("overnightClock").assertTextContains("15:00")
        rule.onNodeWithTag("heroMode").assertTextContains("Overnight", substring = true)
        tap("stopOvernight")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals("back to the mode it interrupted", Triple(null, AutopilotMode.AUTO, null), core.modeCalls.last())
        assertEquals("stopping needs no check", 1, prompts.get())
        awaitGone("overnightClock")
    }

    @Test
    fun overnightWithoutTheOwnersCheckChangesNothing() {
        installModel()
        authResult = dev.reins.android.platform.AuthResult.Cancelled
        openAutopilot()
        tap("mode:OVERNIGHT")
        heard.clear()
        hold("confirmOvernight")
        awaitCore { prompts.get() == 1 }
        settle()
        assertTrue("the core was asked, with an owner check", core.overnightOwners.single() is dev.reins.android.platform.OwnerPrompt)
        assertTrue("and it refused", core.overnightCalls.isEmpty() && core.modeCalls.isEmpty())
        assertEquals(AutopilotMode.MANUAL, container.state.autopilot.value?.mode)
        assertFalse(has("overnightClock"))
        assertFalse("a cancel says nothing", has("autopilotError"))
        assertFalse(heard.played(Event.AutopilotOn))

        // No screen lock at all: it says so, and still nothing changes.
        authResult = dev.reins.android.platform.AuthResult.Unavailable
        tap("mode:OVERNIGHT")
        hold("confirmOvernight")
        awaitTag("autopilotError")
        awaitText("screen lock", substring = true)
        assertTrue(core.overnightCalls.isEmpty())
        assertEquals(AutopilotMode.MANUAL, container.state.autopilot.value?.mode)
    }

    @Test
    fun anAIsOwnOvernightUnderAGlobalOneIsCheckedToo() {
        installModel()
        // A running Overnight shows only Stop; any other length is a new start, and every start asks the owner.
        // Refused, the running one stays as it was.
        core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.OVERNIGHT, overnightUntil = now + 600, overnightSince = now - 60, overnightReturn = AutopilotMode.AUTO)
        authResult = dev.reins.android.platform.AuthResult.Cancelled
        launch()
        tap("tabConnections")
        tap("connection:c1")
        tap("connMode:OVERNIGHT")
        hold("confirmOvernight")
        awaitCore { prompts.get() == 1 }
        settle()
        assertTrue(core.overnightCalls.isEmpty())
        assertEquals(now + 600, core.apGlobal.overnightUntil)
    }

    @Test
    fun overnightUntilTurnedOffShowsHowLongItRan() {
        installModel()
        openAutopilot()
        tap("mode:OVERNIGHT")
        tap("overnightLength:custom")
        tap("overnightFor:untilOff")
        awaitTag("overnightUntilOffWarning")
        awaitText("Hold to start until off")
        hold("confirmOvernightFor")
        awaitCore { core.overnightCalls.isNotEmpty() }
        assertEquals("no end", null to null, core.overnightCalls.single())
        awaitTag("overnightUntilOff")
        rule.onNodeWithTag("overnightLine", useUnmergedTree = true).assertTextContains("On until you stop it", substring = true)
        assertEquals(Long.MAX_VALUE, container.state.autopilot.value?.overnightUntil)
    }

    @Test
    fun ifSomethingIsTooRiskyIsChosenInTheOvernightSheet() {
        installModel()
        openAutopilot()
        tap("mode:OVERNIGHT")
        awaitText("If something is too risky:")
        rule.onNodeWithTag("overnightRisky:WAIT").assertIsSelected()
        tap("overnightRisky:DENY")
        awaitCore { core.whenRiskyCalls.isNotEmpty() }
        assertEquals(listOf(dev.reins.core.OvernightRisky.DENY), core.whenRiskyCalls.toList())
        assertEquals("the owner is asked", "Change Overnight" to "Overnight: deny what is too risky", promptTexts.single())
        awaitText("Decides like Auto. Too risky is denied.")
        assertTrue("a setting, not a start", core.overnightCalls.isEmpty())

        // Back to Wait is asked too; refused, it stays.
        authResult = dev.reins.android.platform.AuthResult.Cancelled
        tap("overnightRisky:WAIT")
        awaitCore { prompts.get() == 2 }
        settle()
        assertEquals(dev.reins.core.OvernightRisky.DENY, core.whenRisky)
        rule.onNodeWithTag("overnightRisky:DENY").assertIsSelected()
        assertTrue(core.overnightOwners.all { it is dev.reins.android.platform.OwnerPrompt })
    }

    @Test
    fun withoutAModelThatRatesRiskOvernightsHelpSaysItDecidesLikeAuto() {
        installModel()
        openAutopilot()
        tap("mode:OVERNIGHT")
        tap("help:Overnight")
        awaitTag("overnightHelp")
        awaitText("Your model doesn't rate risk yet", substring = true)
        tap("cancelOvernight")
        awaitGone("overnightSheet")
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u, risk = true)
        kotlinx.coroutines.runBlocking { container.refreshAutopilot() }
        awaitCore { container.state.autopilot.value?.model?.risk == true }
        tap("mode:OVERNIGHT")
        tap("help:Overnight")
        awaitText("weighed by their risk", substring = true)
    }

    @Test
    fun overnightFromTheQuickMenuIsItsOwnButtonAndStopsThere() {
        installModel()
        launch()
        tap("modePill")
        tap("quick:OVERNIGHT")
        awaitTag("overnightSheet")
        awaitGone("autopilotQuick")
        hold("confirmOvernight")
        awaitCore { core.overnightCalls.isNotEmpty() }
        assertEquals(null to 60u, core.overnightCalls.single())
        assertEquals("checked from the quick menu too", "Start Overnight for 1 h", promptTexts.single().second)
        assertTrue(core.overnightOwners.single() is dev.reins.android.platform.OwnerPrompt)
        awaitGone("overnightSheet")
        rule.onNodeWithContentDescription("Autopilot: Overnight").assertExists()
        tap("modePill")
        awaitText("Tap to stop")
        tap("quick:OVERNIGHT")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals("back to the mode it interrupted", Triple(null, AutopilotMode.MANUAL, null), core.modeCalls.last())
    }

    @Test
    fun aRunningOvernightTurnsThePillIntoACountdown() {
        core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.OVERNIGHT, overnightUntil = now + 8 * 3_600, overnightSince = now - 60, overnightReturn = AutopilotMode.AUTO)
        launch()
        awaitTag("modePill")
        rule.onNodeWithContentDescription("Autopilot: Overnight").assertExists()
        awaitText("8:00:00")
    }

    @Test
    fun anOvernightPostsItsNotificationWhoseStopEndsIt() {
        // Without an end, so the notice does not depend on the phone's own clock.
        core.apGlobal = FakeCore.ApRow(mode = AutopilotMode.OVERNIGHT, overnightUntil = Long.MAX_VALUE, overnightSince = now - 60, overnightReturn = AutopilotMode.AUTO)
        launch()
        awaitTag("modePill")
        rule.onNodeWithContentDescription("Autopilot: Overnight").assertExists()
        val manager = context.getSystemService(android.app.NotificationManager::class.java)
        awaitCore { org.robolectric.Shadows.shadowOf(manager).getNotification(dev.reins.android.platform.AppNotifier.OVERNIGHT_ID) != null }
        val n = org.robolectric.Shadows.shadowOf(manager).getNotification(dev.reins.android.platform.AppNotifier.OVERNIGHT_ID)
        assertEquals("Overnight on · until you stop it", n.extras.getCharSequence(android.app.Notification.EXTRA_TITLE).toString())
        // Its Stop, started on another phone: back to the default, never to something looser.
        kotlinx.coroutines.runBlocking { container.stopOvernights() }
        assertEquals(Triple(null, null, null), core.modeCalls.last())
        awaitCore { org.robolectric.Shadows.shadowOf(manager).getNotification(dev.reins.android.platform.AppNotifier.OVERNIGHT_ID) == null }
    }

    @Test
    fun theMenusOvernightOpensItsSheet() {
        installModel()
        launch()
        tap("modePill")
        tap("quick:OVERNIGHT")
        awaitTag("overnightSheet")
        assertTrue(core.modeCalls.isEmpty())
        hold("confirmOvernight")
        awaitCore { core.overnightCalls.isNotEmpty() }
        assertEquals(1, prompts.get())
        assertTrue(core.overnightOwners.single() is dev.reins.android.platform.OwnerPrompt)
    }

    @Test
    fun aConnectionsOwnOvernightIsCheckedShowsItsTimeAndStops() {
        installModel()
        core.apConnections["c1"] = FakeCore.ApRow(mode = AutopilotMode.MANUAL)
        launch()
        tap("tabConnections")
        tap("connection:c1")
        tap("connMode:OVERNIGHT")
        awaitTag("overnightSheet")
        awaitText("Overnight for Claude?")
        hold("confirmOvernight")
        awaitCore { core.overnightCalls.isNotEmpty() }
        assertEquals("c1" to 60u, core.overnightCalls.single())
        assertEquals("Turn on Overnight for Claude" to "Start Overnight for 1 h", promptTexts.single())
        assertTrue(core.overnightOwners.single() is dev.reins.android.platform.OwnerPrompt)
        awaitTag("stopConnectionOvernight")
        rule.onNodeWithTag("connectionOvernightLine", useUnmergedTree = true).assertTextContains("1 h left", substring = true)
        tap("stopConnectionOvernight")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals("back to its own mode", Triple("c1", AutopilotMode.MANUAL, null), core.modeCalls.last())
    }

    // ---- approvals ----------------------------------------------------------------------------------------------------

    @Test
    fun theApprovalScreenShowsAutopilotsSuggestionAndWhy() {
        installModel()
        core.suggestions["req1"] = TestData.suggestion("req1", novel = true)
        openRequest(TestData.searchView())
        awaitTag("suggestion")
        // A pill with Autopilot's share; a screen reader hears what it means.
        rule.onNodeWithTag("suggestion").assertContentDescriptionContains("Autopilot would allow", substring = true).assertTextContains("97%", substring = true)
        assertFalse(has("getAutopilot"))
        assertFalse(has("suggestionDetail"))
        tap("suggestion")
        awaitTag("suggestionDetail")
        awaitTag("neighbour:2")
        awaitText("never approved", substring = true)
        tap("closeOverlay")
        awaitGone("suggestionDetail")
    }

    @Test
    fun withoutAModelTheApprovalScreenOffersTheDownload() {
        openRequest(TestData.searchView())
        assertFalse(has("suggestion"))
        awaitTag("getAutopilot")
        tap("downloadModel")
        awaitTag("modelProgress")
    }

    @Test
    fun withAModelButNoSuggestionTheApprovalScreenIsAsBefore() {
        installModel()
        openRequest(TestData.searchView())
        assertFalse(has("suggestion"))
        assertFalse(has("getAutopilot"))
    }

    @Test
    fun aWaitingCardOffersAutopilotsPick() {
        val r1 = TestData.pending("r1", quick = true)
        val r2 = TestData.pending("r2", "send", 1u, label = "Codex", conn = "c2", quick = true)
        val r3 = TestData.pending("r3", "read", label = "Cursor", conn = "c3", quick = true)
        val r4 = TestData.pending("r4", "write", 1u, label = "Hermes", conn = "c4", service = "github", op = "repo_delete", opTitle = "Delete a repository")
        val r5 = TestData.pending("r5", label = "Gemini", conn = "c5", quick = true)
        core.suggestions["r1"] = TestData.suggestion("r1")
        core.suggestions["r2"] = TestData.suggestion("r2", verdict = Verdict.DENY, pApprove = 0.03f, pDeny = 0.92f)
        core.suggestions["r3"] = TestData.suggestion("r3", verdict = Verdict.ASK, pApprove = 0.62f, pDeny = 0.1f)
        core.suggestions["r4"] = TestData.suggestion("r4")
        dev.reins.android.platform.Foreground.autoPopup = false
        // The stack shows its top card's pick; each one is put on top in turn.
        fun onTop(item: dev.reins.core.PendingItem) {
            core.pending = listOf(item)
            // Off the main thread: an answer still going out holds the refresh, and it needs the main thread.
            Thread { kotlinx.coroutines.runBlocking { container.refreshPending() } }.start()
        }
        core.pending = listOf(r1)
        launch()
        awaitTag("allow:r1")
        rule.onNodeWithTag("allow:r1").assertTextEquals("Allow", "97%")
        onTop(r3)
        awaitTag("unsure:r3")
        // Asked every time: never a one-tap Allow, whatever Autopilot thinks.
        onTop(r4)
        awaitTag("alwaysAsks:r4")
        assertFalse(has("allow:r4"))
        // No suggestion at all: Review alone.
        onTop(r5)
        awaitTag("review:r5")
        assertFalse(has("allow:r5") || has("deny:r5") || has("unsure:r5"))
        onTop(r1)
        tap("allow:r1")
        awaitCore { core.quickApprovals.contains("r1") }
        assertEquals("Allow checks the owner first", 1, prompts.get())
        assertTrue("approved once, no standing grant", core.approvals.isEmpty())
        onTop(r2)
        awaitTag("deny:r2")
        tap("deny:r2")
        awaitCore { core.denials.contains("r2") }
        assertEquals("Deny needs no check", 1, prompts.get())
    }

    /** A flick right takes Autopilot's pick (its owner check first), a flick left denies; short of 35 % it springs back. */
    @Test
    fun flickingTheTopCardAnswersIt() {
        core.suggestions["r1"] = TestData.suggestion("r1")
        core.suggestions["r2"] = TestData.suggestion("r2")
        core.pending = listOf(TestData.pending("r1", quick = true), TestData.pending("r2", "read", label = "Cursor", conn = "c3", quick = true))
        dev.reins.android.platform.Foreground.autoPopup = false
        launch()
        awaitTag("allow:r1")
        rule.onNodeWithTag("pending:r1").performTouchInput { swipeRight(startX = centerX, endX = centerX + width * 0.2f, durationMillis = 400) }
        settle()
        assertTrue("a short drag answers nothing", core.quickApprovals.isEmpty() && core.denials.isEmpty())
        rule.onNodeWithTag("pending:r1").performTouchInput { swipeRight(startX = left + 10f, endX = right, durationMillis = 300) }
        awaitCore { core.quickApprovals.contains("r1") }
        assertEquals("the owner's check, as Allow", 1, prompts.get())
        awaitTag("allow:r2")
        rule.onNodeWithTag("pending:r2").performTouchInput { swipeLeft(startX = right - 10f, endX = left, durationMillis = 300) }
        awaitCore { core.denials.contains("r2") }
        assertEquals(1, prompts.get())
    }
    @Test
    fun theApprovalScreenShowsTheRiskAndThatOvernightHeldIt() {
        installModel()
        core.suggestions["req1"] = TestData.suggestion(
            "req1",
            mode = AutopilotMode.OVERNIGHT,
            risk = TestData.risk(dev.reins.core.RiskLevel.CATASTROPHIC, "deletes the default branch main", held = true),
        )
        openRequest(TestData.searchView())
        awaitTag("risk")
        rule.onNodeWithTag("riskLine", useUnmergedTree = true).assertTextContains("Risk: catastrophic · deletes the default branch main")
        rule.onNodeWithTag("riskHeld").assertTextContains("Held by Overnight")
    }

    @Test
    fun withoutARiskTheApprovalScreenShowsNone() {
        installModel()
        core.suggestions["req1"] = TestData.suggestion("req1")
        openRequest(TestData.searchView())
        awaitTag("suggestion")
        assertFalse(has("risk"))
    }

    // ---- activity -----------------------------------------------------------------------------------------------------

    @Test
    fun activityShowsOvernightsDecisionsAndTheirRisk() {
        core.activity = listOf(
            TestData.entry(5, "write", "denied", 1u, decidedBy = "overnight", autopilot = TestData.note(AutopilotMode.OVERNIGHT, risk = TestData.risk(dev.reins.core.RiskLevel.CATASTROPHIC, "force push to main", held = true))),
            TestData.entry(4, "write", "released", 1u, decidedBy = "overnight", autopilot = TestData.note(AutopilotMode.OVERNIGHT, risk = TestData.risk(dev.reins.core.RiskLevel.BENIGN, "a feature branch"))),
            TestData.entry(2, "read", "released", 1u),
        )
        launch()
        awaitTag("entry:5")
        // In the list (inside each row's one tap target): only the risk that says something.
        fun inRow(tag: String) = rule.onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        assertTrue(inRow("rowRisk:5"))
        assertTrue(
            rule.onAllNodes(androidx.compose.ui.test.hasTestTag("riskLine") and androidx.compose.ui.test.hasText("Risk: catastrophic · force push to main"), useUnmergedTree = true)
                .fetchSemanticsNodes().isNotEmpty(),
        )
        assertFalse("a low risk is left to the entry's page", inRow("rowRisk:4"))
        assertFalse(inRow("rowRisk:2"))
        assertTrue("its marker", rule.onAllNodes(androidx.compose.ui.test.hasText("Overnight"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
        assertEquals(dev.reins.android.ui.activity.Outcome.Overnight, dev.reins.android.ui.activity.outcomeOf(core.activity[1]))
        assertEquals(dev.reins.android.ui.activity.Outcome.OvernightDenied, dev.reins.android.ui.activity.outcomeOf(core.activity[0]))
        assertEquals("Denied by Overnight", dev.reins.android.ui.activity.outcomeLine(core.activity[0]))
        tap("entry:4")
        awaitTag("entryAutopilot")
        rule.onNodeWithTag("entryAutopilotTitle", useUnmergedTree = true).assertTextContains("Allowed by Overnight")
        awaitTag("risk")
        rule.onNodeWithTag("riskLine", useUnmergedTree = true).assertTextContains("Risk: low · a feature branch")
        assertFalse(has("riskHeld"))
    }

    @Test
    fun aRequestOvernightDeniedSaysSoAndThatItHeldIt() {
        core.activity = listOf(
            TestData.entry(5, "write", "denied", 1u, decidedBy = "overnight", autopilot = TestData.note(AutopilotMode.OVERNIGHT, risk = TestData.risk(dev.reins.core.RiskLevel.CATASTROPHIC, "force push to main", held = true))),
        )
        launch()
        tap("entry:5")
        awaitTag("entryAutopilot")
        rule.onNodeWithTag("entryAutopilotTitle", useUnmergedTree = true).assertTextContains("Denied by Overnight")
        rule.onNodeWithTag("riskHeld").assertTextContains("Held by Overnight")
    }

    private fun automaticEntries() {
        core.activity = listOf(
            TestData.entry(3, "write", "released", 1u, decidedBy = "autopilot", autopilot = TestData.note()),
            TestData.entry(2, "read", "released", 1u),
            TestData.entry(1, "send", "denied", 1u, decidedBy = "lockdown"),
        )
    }

    @Test
    fun theAutomaticFilterShowsOnlyWhatWasDecidedForYou() {
        automaticEntries()
        launch()
        awaitTag("entry:2")
        tap("filters")
        tap("filter:who")
        tap("who:autopilot")
        awaitGone("entry:2")
        assertTrue(has("entry:3"))
        // Lockdown is its own: Who · Autopilot is what Autopilot decided.
        assertFalse(has("entry:1"))
        tap("filter:who")
        tap("who:lockdown")
        awaitTag("entry:1")
        tap("filtersClear")
        awaitTag("entry:2")
    }

    @Test
    fun theFiltersAreBehindTheirButton() {
        core.activity = listOf(TestData.entry(1), TestData.entry(2, outcome = "denied"))
        launch()
        awaitTag("entry:1")
        assertFalse(has("filter:outcome"))
        tap("filters")
        tap("filter:outcome")
        tap("outcome:denied")
        awaitGone("entry:1")
        assertTrue(has("entry:2"))
    }

    @Test
    fun thisWasWrongTeachesAutopilot() {
        automaticEntries()
        launch()
        tap("entry:3")
        awaitTag("entryAutopilot")
        rule.onNodeWithTag("entryAutopilotTitle").assertTextContains("Allowed by Autopilot", substring = true)
        tap("thisWasWrong")
        awaitText("Should this have been denied?")
        rule.onNodeWithText("Deny next time").performClick()
        awaitCore { core.corrections.isNotEmpty() }
        assertEquals(3L to Verdict.DENY, core.corrections.single())
        awaitTag("corrected")
    }

    @Test
    fun aLockdownDenialShowsWhoDecidedButCannotBeCorrected() {
        automaticEntries()
        launch()
        tap("entry:1")
        awaitTag("entryAutopilot")
        rule.onNodeWithTag("entryAutopilotTitle").assertTextContains("Denied by Lockdown")
        assertFalse(has("thisWasWrong"))
    }

    @Test
    fun reportFromANotificationOpensTheEntry() {
        automaticEntries()
        launch(
            android.content.Intent(context, MainActivity::class.java)
                .setAction(dev.reins.android.platform.AppNotifier.ACTION_OPEN_ACTIVITY)
                .putExtra(dev.reins.android.platform.AppNotifier.EXTRA_ACTIVITY_ID, 3L),
        )
        awaitTag("entryAutopilot")
    }

    // ---- sounds ------------------------------------------------------------------------------------------------------

    @Test
    fun autopilotSoundsHaveTheirOwnSwitch() {
        launch()
        tap("tabSettings")
        tap("openSounds")
        tap("autopilotSounds")
        awaitCore { !container.feedbackStore.current.autopilotSounds }
        tap("autopilotSounds")
        awaitCore { container.feedbackStore.current.autopilotSounds }
    }
}
