package dev.reins.android

import androidx.compose.ui.test.*
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ActivityOutcomeFlowTest : FlowHarness() {
    @Test fun savedPermissionsHaveDistinctVisibleAndAccessibleAttribution() {
        core.activity = listOf("account_permission", "temporary_permission").mapIndexed { index, actor ->
            TestData.entry((index + 1).toLong(), "send", "sent", 1u, decidedBy = actor,
                label = TestData.label("gmail", "send", "Send an email to", "friend@example.com"))
        }
        launch(); awaitTag("entry:1")
        rule.onAllNodesWithContentDescription("Permission").assertCountEquals(2)
        tap("entry:1"); awaitTag("entryOutcome")
        rule.onNodeWithTag("entryAutopilotTitle", useUnmergedTree = true).assertTextEquals("Allowed by account permission")
        rule.onNodeWithText("Allowed by Autopilot", substring = true).assertDoesNotExist()
        pressBack(); awaitTag("entry:2"); tap("entry:2"); awaitTag("entryOutcome")
        rule.onNodeWithTag("entryAutopilotTitle", useUnmergedTree = true).assertTextEquals("Allowed by temporary permission")
        rule.onNodeWithText("Allowed by you", substring = true).assertDoesNotExist()
    }
    @Test fun uncertainAndWithheldRowsAnnounceCautionInsteadOfAuthorizationSuccess() {
        // As the core folds them: the decision row carries its request's execution; a record with nothing else is
        // the request's one row while it runs.
        val unknown = TestData.execution(dev.reins.core.ExecutionState.UNKNOWN, listOf(9L))
        val running = TestData.execution(dev.reins.core.ExecutionState.PENDING, listOf(3L))
        core.activity = listOf("sent" to unknown, "succeeded_withheld" to null, "started" to running).mapIndexed { index, (status, execution) ->
            TestData.entry((index + 1).toLong(), "send", status, 1u, decidedBy = "autopilot", grantId = "g1",
                label = TestData.label("gmail", "send", "Send an email to", "friend@example.com"), execution = execution)
        }
        launch(); awaitTag("entry:1")
        rule.onNodeWithContentDescription("Outcome unknown; do not repeat").assertExists()
        rule.onNodeWithContentDescription("Completed; response withheld; do not repeat").assertExists()
        rule.onNodeWithContentDescription("In progress").assertExists()
        rule.onNodeWithText("Awaiting outcome", substring = true).assertDoesNotExist()
        tap("entry:1"); awaitTag("entryOutcome")
        rule.onNodeWithTag("entryAutopilotTitle", useUnmergedTree = true).assertTextEquals("Outcome unknown · do not repeat")
    }
    @Test fun aRequestCarriedOutFineIsOneRowWithNoExtraTag() {
        val done = TestData.execution(dev.reins.core.ExecutionState.DONE, listOf(1L), startedAt = 1_700_000_000)
        core.activity = listOf(
            TestData.entry(2, "write", "sent", 1u, service = "desktop", op = "ssh",
                label = TestData.label("terminal", "run", "Run", "ssh exnomic", code = true), execution = done),
        )
        launch(); awaitTag("entry:2")
        rule.onNodeWithContentDescription("Approved").assertExists()
        rule.onNodeWithContentDescription("Outcome unknown; do not repeat").assertDoesNotExist()
        rule.onNodeWithContentDescription("In progress").assertDoesNotExist()
        rule.onNodeWithText("Awaiting outcome", substring = true).assertDoesNotExist()
    }
}
