package dev.reins.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.core.CommandPresentation
import org.junit.Assert.assertFalse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class CommandPresentationFlowTest : FlowHarness() {
    private val raw = "JAVA_HOME=/opt/jdk TOKEN='fixture-secret-value' ./gradlew :app:assembleDebug --no-daemon"
    private val presentation = CommandPresentation(
        "Run Gradle", "./gradlew :app:assembleDebug --no-daemon",
        listOf("JAVA_HOME=•••• (value hidden)", "TOKEN=•••• (value hidden)"),
        listOf("Gradle can execute project build scripts and plugins with this process’s permissions and credentials."),
    )
    private val label = TestData.label("terminal", "run", "Run", raw, code = true).copy(command = presentation)
    private fun view() = TestData.askView(detail = "Command:\n$raw\n\nIn: /repo\nRule: gradle").let {
        it.copy(ask = it.ask!!.copy(question = "Claude Code wants to run: $raw"))
    }

    @Test
    fun releaseBuildUsesTheVerifiedCoreGoldenContract() {
        val checkout = generateSequence(java.io.File(System.getProperty("user.dir"))) { it.parentFile }
            .first { java.io.File(it, "crates/reins-core/tests/fixtures/gradle-presentation.json").isFile }
        val expected = org.json.JSONObject(java.io.File(checkout, "crates/reins-core/tests/fixtures/gradle-presentation.json").readText())
        val actual = CommandClarityFixtures.presentation()
        assertEquals(expected.getString("heading"), actual.heading)
        assertEquals(expected.getString("actionable"), actual.actionable)
        for ((key, values) in listOf("setup" to actual.setup, "notices" to actual.notices)) {
            val array = expected.getJSONArray(key)
            assertEquals((0 until array.length()).map(array::getString), values)
        }
    }

    @Test
    fun reportedReleaseBuildShowsActionAndConsequencesBeforeGuardAndKeepsConfigurationFolded() {
        core.activity = listOf(CommandClarityFixtures.entry())
        core.entryApprovals[7L] = CommandClarityFixtures.view()
        launch()
        tap("entry:7")
        awaitTag("commandPresentation")
        rule.onNodeWithTag("detailTitle", useUnmergedTree = true).assertTextEquals("Approved: Build Android release and test APKs")
        rule.onNodeWithTag("askCommand", useUnmergedTree = true).assertTextEquals(CommandClarityFixtures.presentation().actionable)
        val commandTop = rule.onNodeWithTag("commandPresentation").fetchSemanticsNode().boundsInRoot.top
        val guardTop = rule.onNodeWithTag("guardRule").fetchSemanticsNode().boundsInRoot.top
        assertTrue("action and concrete effects precede guard details", commandTop < guardTop)
        assertFalse(has("askUncertain"))
        assertFalse(has("commandSetup"))
        assertFalse(has("fullRequestText"))
        assertFalse(showsText("fixture-secret-sentinel", substring = true))
        tap("commandTechnicalDetails")
        awaitTag("commandSetup")
        assertFalse(showsText("fixture-secret-sentinel", substring = true))
        tap("revealFullInvocation")
        rule.onNodeWithTag("fullRequestText", useUnmergedTree = true).assertTextEquals(CommandClarityFixtures.raw)
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun releaseBuildHeadingPrecedesWarningsAtLargeTextWithoutRevealingRawConfiguration() {
        val configuration = android.content.res.Configuration(context.resources.configuration).apply { fontScale = 1.6f }
        context.resources.updateConfiguration(configuration, context.resources.displayMetrics)
        val view = CommandClarityFixtures.view()
        core.approval = view
        core.pending = listOf(TestData.pending(view.requestId, "write", service = "desktop").copy(label = CommandClarityFixtures.label()))
        launch(link("request", view.requestId))
        awaitTag("askQuestion")
        rule.onNodeWithTag("askQuestion", useUnmergedTree = true).assertTextEquals("Build Android release and test APKs").assertIsDisplayed()
        assertTrue(rule.onNodeWithTag("askQuestion").fetchSemanticsNode().boundsInRoot.top < rule.onNodeWithTag("guardRule").fetchSemanticsNode().boundsInRoot.top)
        assertFalse(has("commandSetup"))
        assertFalse(showsText("fixture-secret-sentinel", substring = true))
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun compoundElevatedAndRedirectedActionsStayVisibleWhenTechnicalSetupIsCollapsed() {
        val actionable = "npm test && sudo rm -rf /srv/old $(cat /tmp/target) >> /tmp/cleanup.log 2>&1"
        val command = presentation.copy(heading = "Test and remove a directory", actionable = actionable,
            notices = listOf("Runs multiple commands.", "Requests elevated privileges with sudo.", "Appends output to /tmp/cleanup.log.",
                "Contains command substitution; inspect the original invocation.", "Runs multiple commands."))
        core.activity = listOf(TestData.entry(7, "write", "released", service = "desktop", op = "ask",
            label = label.copy(command = command), request = TestData.asked(actionable)))
        launch()
        tap("entry:7")
        awaitTag("commandPresentation")
        rule.onNodeWithTag("askCommand", useUnmergedTree = true).assertTextEquals(actionable)
        rule.onAllNodes(hasTestTag("commandNotice:4"), useUnmergedTree = true).assertCountEquals(0)
        assertTrue(showsText("Requests elevated privileges with sudo."))
        assertTrue(showsText("Appends output to /tmp/cleanup.log."))
        assertTrue(showsText("Contains command substitution; inspect the original invocation."))
        assertFalse(has("commandSetup"))
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun pendingShowsActionAndConcealsEnvironmentUntilExplicitInspection() {
        val view = view()
        core.pending = listOf(TestData.pending(view.requestId, "write", 1u, service = "desktop").copy(label = label))
        core.approval = view
        launch()
        awaitTag("pending:${view.requestId}")
        if (!has("sheet")) tap("pending:${view.requestId}")
        awaitTag("commandPresentation")
        rule.onNodeWithTag("askCommand", useUnmergedTree = true).assertTextEquals(presentation.actionable)
        assertFalse(showsText("fixture-secret-value", substring = true))
        tap("fullInvocation")
        awaitTag("fullInvocationText")
        rule.onNodeWithTag("fullInvocationText", useUnmergedTree = true).assertTextContains(raw, substring = true)
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun historyRetainsOriginalAndCopyButDoesNotRevealSecretValuesOnEntry() {
        core.activity = listOf(TestData.entry(7, "write", "released", service = "desktop", op = "ask", label = label,
            request = TestData.asked(raw, cwd = "/repo")))
        launch()
        tap("entry:7")
        awaitTag("commandPresentation")
        assertFalse(has("fullRequestText"))
        assertFalse(showsText("fixture-secret-value", substring = true))
        tap("revealFullInvocation")
        rule.onNodeWithTag("fullRequestText", useUnmergedTree = true).assertTextEquals(raw)
        assertTrue(has("copyFullRequest"))
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun retainedApprovalHistoryShowsCommandFactsOnceAndHasNoDecisionDock() {
        core.activity = listOf(TestData.entry(7, "write", "released", service = "desktop", op = "ask", label = label,
            request = TestData.asked(raw, cwd = "/repo")))
        core.entryApprovals[7L] = view().copy(waitUntil = null)
        launch()
        tap("entry:7")
        awaitTag("commandPresentation")
        rule.onAllNodes(hasTestTag("commandPresentation"), useUnmergedTree = true).assertCountEquals(1)
        assertFalse(has("dock"))
        assertFalse(has("approve"))
        assertFalse(has("fullRequestText"))
        assertTrue(core.approvals.isEmpty())
    }

    @Test
    fun explicitInspectionUsesTheOriginalEvenWhenLegacyDetailWasShortened() {
        val original = "TOKEN='" + "fixture".repeat(1400) + "' ./gradlew test --info"
        val view = view().let { it.copy(ask = it.ask!!.copy(detail = original.take(8_000))) }
        core.pending = listOf(TestData.pending(view.requestId, "write", 1u, service = "desktop").copy(
            label = label.copy(command = presentation.copy(original = original)),
        ))
        core.approval = view
        launch(link("request", view.requestId))
        awaitTag("commandPresentation")
        assertFalse(showsText("fixturefixture", substring = true))
        tap("fullInvocation")
        awaitTag("fullInvocationText")
        rule.onNodeWithTag("fullInvocationText", useUnmergedTree = true).assertTextEquals(original)
    }
}
