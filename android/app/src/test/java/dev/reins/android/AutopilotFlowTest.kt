package dev.rewarden.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.work.NetworkType
import androidx.work.WorkManager
import androidx.work.testing.WorkManagerTestInitHelper
import dev.rewarden.android.autopilot.WorkModelDownloads
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.FeedbackProvider
import dev.rewarden.core.AutopilotMode
import dev.rewarden.core.CoreException
import dev.rewarden.core.ModelState
import dev.rewarden.core.Preset
import dev.rewarden.core.Verdict
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
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
        FeedbackProvider.observer = heard
    }

    @After
    fun stopListening() {
        FeedbackProvider.observer = null
    }

    private fun installModel() {
        core.model = TestData.modelStatus(ModelState.INSTALLED, size = 412_000_000u)
    }

    private fun openAutopilot() {
        launch()
        tap("openSettings")
        tap("openAutopilot")
        awaitTag("mode:MANUAL")
    }

    // ---- modes ------------------------------------------------------------------------------------------------------

    @Test
    fun theHeaderPillShowsTheModeAndOpensAutopilot() {
        installModel()
        launch()
        awaitTag("modePill")
        awaitText("Assisted")
        tap("modePill")
        awaitTag("modeHero")
        rule.onNodeWithTag("heroMode").assertTextContains("Assisted")
    }

    @Test
    fun aRunningBypassTurnsThePillIntoACountdown() {
        core.apGlobal = FakeCore.ApRow(bypassUntil = now + 14 * 60 + 32)
        launch()
        awaitTag("modePill")
        awaitText("14:32")
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
        awaitTag("bypassDialog")
        assertTrue("nothing changes before the warning is accepted", core.modeCalls.isEmpty())
        tap("bypass:30")
        heard.clear()
        tap("confirmBypass")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals(Triple(null, AutopilotMode.BYPASS, 30u), core.modeCalls.single())
        assertTrue(heard.played(Event.BypassOn))
        awaitTag("bypassClock")
        rule.onNodeWithTag("bypassClock").assertTextContains("30:00")
        heard.clear()
        tap("stopBypass")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals("back to the mode it interrupted", Triple(null, AutopilotMode.ASSISTED, null), core.modeCalls.last())
        assertTrue(heard.played(Event.BypassOff))
        awaitGone("bypassClock")
    }

    @Test
    fun lockdownIsConfirmedFirst() {
        openAutopilot()
        tap("mode:LOCKDOWN")
        awaitText("Lock down?")
        assertTrue(core.modeCalls.isEmpty())
        heard.clear()
        rule.onNodeWithText("Lock down").performClick()
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals(AutopilotMode.LOCKDOWN, core.modeCalls.single().second)
        assertTrue(heard.played(Event.LockdownOn))
        awaitTag("endLockdown")
        tap("endLockdown")
        awaitCore { core.modeCalls.size == 2 }
        assertEquals(AutopilotMode.MANUAL, core.modeCalls.last().second)
    }

    @Test
    fun withoutTheModelAssistedAndAutoSaySoAndTheModeIsManual() {
        openAutopilot()
        awaitText("NEEDS MODEL")
        rule.onNodeWithTag("heroMode").assertTextContains("Manual")
        awaitText("Download the model first")
    }

    // ---- the model --------------------------------------------------------------------------------------------------

    private fun runDownloadJob() {
        val infos = WorkManager.getInstance(context).getWorkInfosForUniqueWork(WorkModelDownloads.NAME).get()
        WorkManagerTestInitHelper.getTestDriver(context)!!.setAllConstraintsMet(infos.single().id)
    }

    @Test
    fun theModelDownloadsOnWifiAndShowsWhenItIsReady() {
        openAutopilot()
        tap("downloadModel")
        awaitText("Waiting for Wi-Fi")
        val request = WorkManager.getInstance(context).getWorkInfosForUniqueWork(WorkModelDownloads.NAME).get().single()
        assertEquals(NetworkType.UNMETERED, request.constraints.requiredNetworkType)
        assertEquals("the core downloads nothing before the network is there", 0, core.downloads.get())
        runDownloadJob()
        awaitCore { core.downloads.get() == 1 }
        awaitTag("modelInstalled")
        awaitTag("deleteModel")
    }

    @Test
    fun mobileDataCanBeAllowed() {
        openAutopilot()
        tap("wifiOnly")
        awaitCore { !core.wifiOnly }
        tap("downloadModel")
        awaitText("Waiting for a connection")
        val request = WorkManager.getInstance(context).getWorkInfosForUniqueWork(WorkModelDownloads.NAME).get().single()
        assertEquals(NetworkType.CONNECTED, request.constraints.requiredNetworkType)
    }

    @Test
    fun aDownloadThatFailsItsCheckSaysSoAndCanBeRetried() {
        core.downloadFailure = CoreException.Invalid("checksum mismatch")
        core.downloadError = "sha-256 of model.onnx does not match the pinned hash"
        openAutopilot()
        tap("downloadModel")
        awaitText("Waiting for Wi-Fi")
        runDownloadJob()
        awaitTag("modelError")
        awaitText("did not match", substring = true)
        awaitText("Try again")
    }

    @Test
    fun aWaitingDownloadCanBeCancelled() {
        openAutopilot()
        tap("downloadModel")
        awaitTag("cancelDownload")
        tap("cancelDownload")
        awaitTag("downloadModel")
        assertEquals(0, core.downloads.get())
    }

    @Test
    fun theModelCanBeDeleted() {
        installModel()
        openAutopilot()
        tap("deleteModel")
        awaitText("Delete the model?")
        rule.onNodeWithText("Delete").performClick()
        awaitCore { core.model.state == ModelState.NOT_INSTALLED }
        awaitTag("downloadModel")
        assertTrue(heard.played(Event.Revoked))
    }

    // ---- profiles ---------------------------------------------------------------------------------------------------

    @Test
    fun aProfileShowsItsClassesAndTakesAPreset() {
        installModel()
        openAutopilot()
        tap("profile:personal")
        awaitTag("class:github/write/push")
        rule.onNodeWithTag("classStatus:github/write/push", useUnmergedTree = true).assertTextContains("Approves on its own")
        rule.onNodeWithTag("classStatus:desktop/ask/command", useUnmergedTree = true).assertTextContains("15 more decisions to unlock")
        tap("preset:CAUTIOUS")
        awaitCore { core.presets.isNotEmpty() }
        assertEquals("personal" to Preset.CAUTIOUS, core.presets.single())
        awaitText("Approves when 98% sure, denies when 95% sure")
    }

    @Test
    fun unlockingAClassByHandWarnsFirst() {
        installModel()
        openAutopilot()
        tap("profile:personal")
        tap("class:gmail/read")
        tap("lock:off:gmail/read")
        awaitText("Let Auto approve", substring = true)
        assertTrue(core.classLocks.isEmpty())
        rule.onNodeWithText("Unlock").performClick()
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
        tap("openSettings")
        tap("connection:c1")
        awaitTag("connectionMode")
        tap("connMode:AUTO")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals(Triple("c1", AutopilotMode.AUTO, null), core.modeCalls.single())
        awaitText("Its own mode")
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
        tap("openSettings")
        tap("connection:c1")
        tap("connMode:BYPASS")
        awaitTag("bypassDialog")
        tap("confirmBypass")
        awaitTag("connAutopilotError")
        awaitText("10 minutes", substring = true)
        assertTrue(heard.played(Event.Error))
    }

    @Test
    fun aConnectionBypassShowsItsTimeAndStops() {
        installModel()
        core.apConnections["c1"] = FakeCore.ApRow(mode = AutopilotMode.AUTO, bypassUntil = now + 600)
        launch()
        tap("openSettings")
        tap("connection:c1")
        awaitTag("stopConnectionBypass")
        rule.onNodeWithTag("connectionModeLine").assertTextContains("10 min left", substring = true)
        tap("stopConnectionBypass")
        awaitCore { core.modeCalls.isNotEmpty() }
        assertEquals("back to its own mode", Triple("c1", AutopilotMode.AUTO, null), core.modeCalls.single())
    }

    // ---- approvals ----------------------------------------------------------------------------------------------------

    @Test
    fun theApprovalScreenShowsAutopilotsSuggestionAndWhy() {
        installModel()
        core.suggestions["req1"] = TestData.suggestion("req1", novel = true)
        openRequest(TestData.searchView())
        awaitTag("suggestion")
        rule.onNodeWithTag("suggestionHeadline", useUnmergedTree = true).assertTextContains("Autopilot would approve · 97%")
        assertFalse(has("suggestionDetail"))
        tap("suggestionToggle")
        awaitTag("suggestionDetail")
        awaitTag("neighbour:2")
        awaitText("never approved", substring = true)
    }

    @Test
    fun withoutASuggestionTheApprovalScreenIsAsBefore() {
        openRequest(TestData.searchView())
        assertFalse(has("suggestion"))
    }

    @Test
    fun aWaitingRequestCarriesTheSuggestionLine() {
        core.pending = listOf(TestData.pending("req1", suggestion = "Autopilot would deny · 92%"))
        dev.rewarden.android.platform.Foreground.autoPopup = false
        launch()
        awaitTag("pending:req1")
        awaitText("Autopilot would deny · 92%")
    }

    // ---- activity -----------------------------------------------------------------------------------------------------

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
        awaitTag("filter:automatic")
        rule.onNodeWithTag("filter:automatic").assertTextContains("Automatic · 2")
        awaitTag("entry:2")
        tap("filter:automatic")
        awaitGone("entry:2")
        assertTrue(has("entry:3"))
        assertTrue(has("entry:1"))
        tap("filter:all")
        awaitTag("entry:2")
    }

    @Test
    fun withoutAutomaticDecisionsThereIsNoFilter() {
        core.activity = listOf(TestData.entry(1))
        launch()
        awaitTag("entry:1")
        assertFalse(has("filter:automatic"))
    }

    @Test
    fun thisWasWrongTeachesAutopilot() {
        automaticEntries()
        launch()
        tap("entry:3")
        awaitTag("entryAutopilot")
        rule.onNodeWithTag("entryAutopilotTitle").assertTextContains("Approved by Autopilot")
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
                .setAction(dev.rewarden.android.platform.AppNotifier.ACTION_OPEN_ACTIVITY)
                .putExtra(dev.rewarden.android.platform.AppNotifier.EXTRA_ACTIVITY_ID, 3L),
        )
        awaitTag("entryAutopilot")
    }

    // ---- sounds ------------------------------------------------------------------------------------------------------

    @Test
    fun autopilotSoundsHaveTheirOwnSwitch() {
        launch()
        tap("openSettings")
        tap("openSounds")
        tap("autopilotSounds")
        awaitCore { !container.feedbackStore.current.autopilotSounds }
        tap("autopilotSounds")
        awaitCore { container.feedbackStore.current.autopilotSounds }
    }
}
