package dev.reins.android

import androidx.compose.ui.test.assertIsFocused

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.ui.connections.calledItself
import dev.reins.android.ui.connections.lastUsed
import dev.reins.android.ui.pairing.sameClientLine
import dev.reins.android.ui.pairing.startingName
import dev.reins.core.PairingView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Any connection can be renamed from its page (the core keeps the name with the account and every screen shows it); the
 * page still says what it calls itself. A second account of an AI already connected starts with a name of its own
 * ("Claude 2") and the cursor in the name field.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ConnectionNamesFlowTest : FlowHarness() {
    @Before
    fun connections() {
        core.connections = listOf(
            TestData.connection("c1", "Claude").copy(ownLabel = "Claude", clientName = "Claude"),
            TestData.computer("d1", "M2A").copy(ownLabel = "M2A", clientName = "M2A (macOS)"),
        )
        core.renames.clear()
    }

    private fun rename(id: String, to: String) {
        tap("connectionRename")
        awaitTag("renameField")
        rule.onNodeWithTag("renameField").performTextReplacement(to)
        tap("renameSave")
        awaitGone("renameSheet")
    }

    @Test
    fun aConnectionIsRenamedAndClearedFromItsPage() {
        launch()
        tap("tabConnections")
        tap("connection:c1")
        awaitTag("connectionName")
        assertFalse("its own name: nothing to add", has("connectionCalledItself"))
        rename("c1", "  Claude (work) ")
        awaitCore { core.renames.contains("c1=Claude (work)") }
        rule.waitUntil(5_000) { settle(); showsText("Claude (work)") }
        rule.onNodeWithTag("connectionName").assertTextEquals("Claude (work)")
        awaitTag("connectionCalledItself")
        rule.onNodeWithTag("connectionCalledItself").assertTextEquals("Calls itself “Claude”")
        // The field opens with the name and the cursor in it; the clear button gives it its own name back.
        tap("connectionRename")
        awaitTag("renameField")
        rule.onNodeWithTag("renameField").assertTextEquals("Claude (work)").assertIsFocused()
        tap("renameClear")
        tap("renameSave")
        awaitCore { core.renames.last() == "c1=" }
        rule.waitUntil(5_000) { settle(); !has("connectionCalledItself") }
        rule.onNodeWithTag("connectionName").assertTextEquals("Claude")
    }

    @Test
    fun aComputerIsRenamedToo() {
        launch()
        tap("tabConnections")
        tap("connection:d1")
        rename("d1", "Work Mac")
        awaitCore { core.renames.contains("d1=Work Mac") }
        awaitTag("connectionCalledItself")
        rule.onNodeWithTag("connectionCalledItself").assertTextEquals("Calls itself “M2A (macOS)”")
    }

    @Test
    fun aSecondClaudeStartsAsClaude2WithTheCursorInTheName() {
        core.pending = listOf(TestData.pairingItem("p2"))
        core.pairing = PairingView("p2", "Claude", "claude.ai", byteArrayOf(7, 42, 88), 1_700_000_200, null, "Claude 2", 1u)
        launch(link("pairing", "p2"))
        awaitTag("pairSameClient")
        assertTrue(showsText("Claude is connected already · name this one"))
        rule.onNodeWithTag("label").assertTextEquals("Claude 2").assertIsFocused()
    }

    @Test
    fun aFirstConnectionStartsWithItsOwnName() {
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = PairingView("p1", "Claude", "claude.ai", byteArrayOf(7, 42, 88), 1_700_000_200, null)
        launch(link("pairing", "p1"))
        awaitTag("label")
        rule.onNodeWithTag("label").assertTextEquals("Claude")
        assertFalse(has("pairSameClient"))
    }

    @Test
    fun theWordsAreCleanAndQuiet() {
        val second = PairingView("p", "Cl‮aude\n", "claude.ai", byteArrayOf(1, 2, 3), 0, null, "Claude​ 2", 1u)
        assertFalse(startingName(second).any { it == '​' || it == '\n' })
        assertEquals("Claude 2", startingName(PairingView("p", "Claude", "claude.ai", byteArrayOf(), 0, null, "Claude 2", 1u)))
        assertEquals("Claude", startingName(PairingView("p", "Claude", "claude.ai", byteArrayOf(), 0, null)))
        assertEquals("Claude is connected already · name this one", sameClientLine(PairingView("p", "Claude", "claude.ai", byteArrayOf(), 0, null)))
        val claude = TestData.connection("c1", "Claude").copy(ownLabel = "Claude", clientName = "Claude")
        assertNull(calledItself(claude))
        assertEquals("Calls itself “Claude”", calledItself(claude.copy(name = "Mine", label = "Mine")))
        assertFalse(calledItself(claude.copy(name = "Mine", clientName = "Ev‮il\nname"))!!.any { it == '‮' || it == '\n' })
        assertEquals("Never", lastUsed(claude.copy(lastUsedAt = null)))
    }
}
