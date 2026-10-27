package dev.reins.android

import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** This phone's signing key on a connection's page: one quiet row under More, with a (?) that says what it is for. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class SigningKeyFlowTest : FlowHarness() {
    @Before
    fun connections() {
        core.connections = listOf(TestData.connection("c1", "Claude"))
    }

    @Test
    fun theSigningKeyIsUnderMoreWithItsExplanation() {
        core.signingKey = "4821 9930 1274"
        launch()
        tap("tabConnections")
        tap("connection:c1")
        awaitTag("connectionName")
        assertFalse("not on the page until More opens", has("signingKey"))
        tap("connectionMore")
        awaitTag("signingKey")
        rule.waitUntil(5_000) { settle(); showsText("4821 9930 1274") }
        rule.onNodeWithTag("signingKeyValue", useUnmergedTree = true).assertTextContains("4821 9930 1274")
        assertFalse(showsText("Agents and your computer use this to check what this phone signs."))
        tap("help:Signing key")
        awaitText("Agents and your computer use this to check what this phone signs.")
        // Read once, however often More opens.
        tap("connectionMore")
        tap("connectionMore")
        awaitTag("signingKey")
        assertEquals(1, core.signingKeyCalls.get())
    }
}
