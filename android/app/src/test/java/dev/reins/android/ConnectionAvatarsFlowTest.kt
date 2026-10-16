package dev.reins.android

import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Connections show the icon the core says (a computer its platform's logo), and the Icon sheet changes the blobatar:
 * its shape, its colour, a shuffle, each kept by the core.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class ConnectionAvatarsFlowTest : FlowHarness() {
    @Before
    fun connections() {
        core.connections = TestData.showcaseConnections()
        core.icons.clear()
        core.blobatarCalls.clear()
    }

    /** A tag inside a row (merged into it for screen readers). */
    private fun hasDeep(tag: String): Boolean {
        settle()
        return rule.onAllNodes(androidx.compose.ui.test.hasTestTag(tag), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
    }

    private fun blobatarOf(id: String) = core.connections.single { it.id == id }.blobatar!!

    private fun openIconSheet(id: String) {
        launch()
        tap("tabConnections")
        tap("connection:$id")
        tap("connectionIcon")
        awaitTag("blobatarEditor")
    }

    @Test
    fun computersShowTheirPlatformAndTheOthersTheirLogoOrBlobatar() {
        launch()
        tap("tabConnections")
        awaitTag("connection:d1")
        assertTrue(hasDeep("avatar:d1:macos"))
        assertTrue(hasDeep("avatar:d2:linux"))
        assertTrue(hasDeep("avatar:c1:claude"))
        assertTrue(hasDeep("avatar:c2:grok"))
        assertTrue(hasDeep("avatar:c3:blob"))
        assertTrue("the Apple logo", rule.onAllNodes(hasContentDescription("macOS"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
        assertTrue("Tux", rule.onAllNodes(hasContentDescription("Linux"), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty())
        // A computer's page: its logo, and the platforms to pick from.
        tap("connection:d1")
        awaitTag("connectionAvatar:macos")
        rule.onNodeWithTag("connectionIcon").assertTextContains("Auto", substring = true)
        tap("connectionIcon")
        awaitTag("icon:macos")
        assertTrue(has("icon:windows") && has("icon:linux") && has("icon:blob") && has("icon:auto"))
        tap("icon:linux")
        awaitCore { core.icons["d1"] == "linux" }
        awaitTag("connectionAvatar:linux")
        rule.onNodeWithTag("connectionIcon").assertTextContains("Linux", substring = true)
    }

    @Test
    fun anAiIsNotOfferedThePlatforms() {
        openIconSheet("c1")
        assertTrue(has("icon:claude"))
        assertFalse(has("icon:macos"))
        assertFalse(has("icon:windows"))
    }

    @Test
    fun pickingAShapeKeepsItAndShowsTheBlobatar() {
        openIconSheet("d1")
        tap("shape:3")
        awaitCore { blobatarOf("d1").shape.toInt() == 3 }
        assertEquals(listOf("shape 3 colour 7 d1"), core.blobatarCalls.toList())
        assertEquals("d1", blobatarOf("d1").seed)
        assertEquals("kept colour", 7, blobatarOf("d1").colour.toInt())
        // It showed the Apple logo: now it shows the blobatar, so the change is seen.
        awaitCore { core.icons["d1"] == "blob" }
        awaitTag("connectionAvatar:blob")
        // The sheet stays open, with the new shape ringed.
        assertTrue(has("blobatarEditor"))
        rule.onNodeWithTag("shape:3").assertIsSelected()
    }

    @Test
    fun pickingAColourKeepsIt() {
        openIconSheet("c3")
        tap("colour:10")
        awaitCore { blobatarOf("c3").colour.toInt() == 10 }
        assertEquals(listOf("shape 5 colour 10 c3"), core.blobatarCalls.toList())
        // Already its blobatar (Auto shows it): the icon stays Auto.
        settle()
        assertFalse(core.icons.containsKey("c3"))
        rule.onNodeWithTag("colour:10").assertIsSelected()
        rule.onNodeWithTag("colour:0").assertIsNotSelected()
    }

    @Test
    fun shuffleGivesANewBlobatarAndShowsIt() {
        openIconSheet("c2")
        val before = blobatarOf("c2")
        tap("shuffleBlobatar")
        awaitCore { blobatarOf("c2") != before }
        assertEquals(listOf("shuffle c2"), core.blobatarCalls.toList())
        awaitCore { core.icons["c2"] == "blob" }
        awaitTag("connectionAvatar:blob")
        rule.onNodeWithTag("connectionIcon").assertTextContains("Blobatar", substring = true)
        // It persists: the list shows it after leaving.
        pressBack()
        pressBack()
        rule.waitUntil(10_000) { hasDeep("avatar:c2:blob") }
    }
}
