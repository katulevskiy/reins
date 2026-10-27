package dev.reins.android

import android.os.Looper
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.ui.connections.NOT_A_VIEWER
import dev.reins.core.CoreException
import dev.reins.core.EnrollProgress
import java.time.Duration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * A phone the server does not show the AI apps and computers to (signed in, neither approving nor a viewer) says where
 * they are and offers to join as a second phone: it shows the code to compare, and once the approving phone added it,
 * the list.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ConnectionsHiddenFlowTest : FlowHarness() {
    private fun hidden() {
        core.connections = TestData.showcaseConnections()
        core.connectionsError = CoreException.NotAViewer()
        launch()
        tap("tabConnections")
        awaitTag("aisHidden")
    }

    private fun poll() {
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(2_100))
        settle()
    }

    @Test
    fun aRefusedListSaysWhereItIsInsteadOfNone() {
        hidden()
        assertTrue(has("computersHidden"))
        assertEquals(2, rule.onAllNodes(hasText(NOT_A_VIEWER)).fetchSemanticsNodes().size)
        assertFalse("no empty-state offer", has("connectEmpty") || has("connectComputerEmpty"))
        assertFalse("nothing listed", has("connection:c1") || has("connection:d1"))
    }

    @Test
    fun joiningShowsTheCodeAndThenTheList() {
        core.enrollScript.addAll(listOf(EnrollProgress.Waiting, EnrollProgress.Compare("482 193"), EnrollProgress.Enrolled))
        hidden()
        tap("join:aisHidden")
        tap("viewerJoinStart")
        awaitCore { core.enrollBegins.isNotEmpty() }
        awaitTag("viewerJoinWaiting")
        poll()
        assertTrue(has("viewerJoinWaiting"))
        poll()
        awaitTag("viewerJoinCode")
        rule.onNodeWithTag("viewerJoinCode").assertTextEquals("482 193")
        poll()
        awaitTag("viewerJoinDone")
        tap("viewerJoinClose")
        awaitTag("connection:c1")
        assertFalse(has("aisHidden") || has("computersHidden"))
        assertEquals(0, core.enrollCancels.get())
    }

    @Test
    fun closingBeforeTheEndWithdrawsTheRequest() {
        hidden()
        tap("join:aisHidden")
        tap("viewerJoinStart")
        awaitTag("viewerJoinWaiting")
        pressBack()
        awaitCore { core.enrollCancels.get() == 1 }
        assertTrue(has("aisHidden"))
    }

    @Test
    fun aDeniedRequestSaysSoAndCanBeAskedAgain() {
        core.enrollScript.add(EnrollProgress.Denied)
        hidden()
        tap("join:aisHidden")
        tap("viewerJoinStart")
        awaitTag("viewerJoinWaiting")
        poll()
        awaitTag("viewerJoinEnded")
        awaitText("The approving phone said no.")
        tap("viewerJoinRetry")
        awaitCore { core.enrollBegins.size == 2 }
    }
}
