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
    @Test fun uncertainAndWithheldRowsAnnounceCautionInsteadOfAuthorizationSuccess() {
        core.activity = listOf("unknown", "succeeded_withheld", "started").mapIndexed { index, status ->
            TestData.entry((index + 1).toLong(), "send", status, 1u, decidedBy = "autopilot", grantId = "g1",
                label = TestData.label("gmail", "send", "Send an email to", "friend@example.com"))
        }
        launch(); awaitTag("entry:1")
        rule.onNodeWithContentDescription("Outcome unknown; do not repeat").assertExists()
        rule.onNodeWithContentDescription("Completed; response withheld; do not repeat").assertExists()
        rule.onNodeWithContentDescription("Attempted; awaiting outcome").assertExists()
        tap("entry:1"); awaitTag("entryOutcome")
        rule.onNodeWithTag("entryAutopilotTitle", useUnmergedTree = true).assertTextEquals("Outcome unknown · do not repeat")
    }
}
