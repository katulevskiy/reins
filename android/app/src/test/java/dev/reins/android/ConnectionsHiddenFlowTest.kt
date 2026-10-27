package dev.reins.android

import androidx.compose.ui.test.hasText
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.ui.connections.NOT_A_VIEWER
import dev.reins.core.CoreException
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * A phone the server does not show the AI apps and computers to (signed in, neither approving nor enrolled as a
 * viewer) says where they are, with a (?), and never "connect one" as if there were none. Once the list comes, it shows.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ConnectionsHiddenFlowTest : FlowHarness() {
    @Test
    fun aRefusedListSaysWhereItIsInsteadOfNone() {
        core.connections = TestData.showcaseConnections()
        core.connectionsError = CoreException.NotAViewer()
        launch()
        tap("tabConnections")
        awaitTag("aisHidden")
        assertTrue(has("computersHidden"))
        assertTrue(rule.onAllNodes(hasText(NOT_A_VIEWER)).fetchSemanticsNodes().size == 2)
        assertFalse("no empty-state offer", has("connectEmpty") || has("connectComputerEmpty"))
        assertFalse("nothing listed", has("connection:c1") || has("connection:d1"))
        tap("help:aisHidden")
        rule.waitUntil(5_000) { rule.onAllNodes(hasText("is not one of them", substring = true)).fetchSemanticsNodes().isNotEmpty() }

        // Enrolled (or approving) later: the list replaces the note.
        core.connectionsError = null
        kotlinx.coroutines.runBlocking { container.refreshConnections() }
        awaitTag("connection:c1")
        assertFalse(has("aisHidden") || has("computersHidden"))
    }
}
