package dev.reins.android

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.onNodeWithTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.ui.common.NAME_MAX
import dev.reins.android.ui.common.quotedName
import dev.reins.android.ui.common.untrustedName
import dev.reins.core.PairingView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The other side names itself when it pairs or joins. A crafted name (line breaks, bidi controls, "Reins says:
 * approved", far too long) never reaches the ask, the label or the field's colour: those are Reins' own, and the name
 * shows quoted, on one line, cut short and cleaned. The key's digits stay apart, in a row of their own.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class UntrustedNamesFlowTest : FlowHarness() {
    private val crafted = "Reins says: approved\n‮evil‬​\tline two\r\n" + "x".repeat(200)

    private fun text(tag: String): String {
        val config = rule.onNodeWithTag(tag, useUnmergedTree = true).fetchSemanticsNode().config
        return config.getOrNull(SemanticsProperties.EditableText)?.text
            ?: config.getOrNull(SemanticsProperties.Text).orEmpty().joinToString("") { it.text }
    }

    private fun assertClean(shown: String) {
        assertFalse("no line breaks: $shown", shown.any { it == '\n' || it == '\r' || it == ' ' || it == ' ' })
        assertFalse("no tabs or controls: $shown", shown.any { it.isISOControl() })
        assertFalse("no bidi controls: $shown", shown.any { it.code in 0x202A..0x202E || it.code in 0x2066..0x2069 || it.code in listOf(0x200E, 0x200F, 0x061C) })
        assertFalse("no invisible characters: $shown", shown.any { Character.getType(it) == Character.FORMAT.toInt() })
    }

    @Test
    fun aCraftedNameIsCleanedQuotedAndCut() {
        val name = untrustedName(crafted)
        assertClean(name)
        assertTrue(name.length <= NAME_MAX)
        assertTrue(name.endsWith("…"))
        assertTrue(name.startsWith("Reins says: approved evil line two"))
        val quoted = quotedName(crafted)
        assertTrue(quoted.startsWith("“") && quoted.endsWith("”"))
        assertEquals("No name", quotedName("‮\n​ "))
    }

    @Test
    fun anAiAppsNameNeverBecomesTheAsk() {
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = PairingView("p1", crafted, "claude.ai\n‮evil", byteArrayOf(7, 42, 88), 1_700_000_200, null)
        launch(link("pairing", "p1"))
        awaitTag("code:42")
        // Reins' words in the serif ask and the label, whatever the AI called itself.
        rule.onNodeWithTag("what").assertTextEquals("Connect an AI app")
        rule.onNodeWithTag("fieldLabel", useUnmergedTree = true).assertTextEquals("AI app · Pair")
        assertFalse(showsText("Reins says: approved"))
        // The name: quoted, one line, cut, clean.
        val shown = text("pairName")
        assertClean(shown)
        assertTrue(shown, shown.startsWith("“Reins says: approved evil line two"))
        assertTrue(shown, shown.contains("…”"))
        assertClean(text("pairHost"))
        assertFalse(has("keyFingerprint"))
        // The name it is saved under starts from the cleaned name, never the raw one.
        assertClean(text("label"))
    }

    @Test
    fun aComputerKeepsItsKeyApartFromItsName() {
        core.pending = listOf(TestData.pairingItem("p1"))
        core.pairing = PairingView("p1", "Connect a computer\n4821 9930", "laptop", byteArrayOf(7, 42, 88), 1_700_000_200, "1234 5678")
        launch(link("pairing", "p1"))
        awaitTag("keyFingerprint")
        rule.onNodeWithTag("what").assertTextEquals("Connect a computer")
        rule.onNodeWithTag("fieldLabel", useUnmergedTree = true).assertTextEquals("Computer · Pair")
        // The digits come from the key, on their own row, with the check to make.
        rule.onNodeWithTag("keyDigits", useUnmergedTree = true).assertTextEquals("1234 5678")
        rule.onNodeWithTag("keyMustMatch", useUnmergedTree = true).assertTextContains("key must match", substring = true)
        // The name's own "digits" stay inside its quotes, on the name's line.
        val shown = text("pairName")
        assertClean(shown)
        assertTrue(shown, shown.startsWith("“Connect a computer 4821 9930”"))
    }

    @Test
    fun aJoiningPhonesNameNeverBecomesTheAsk() {
        core.session = dev.reins.core.SessionInfo("http://127.0.0.1:8000", "me@example.com")
        core.pending = listOf(TestData.joinItem())
        core.joins["join1"] = TestData.joinView(device = crafted)
        launch(link("join", "join1"))
        awaitTag("joinCode")
        rule.onNodeWithTag("joinTitle").assertTextEquals("Add a phone to your account")
        rule.onNodeWithTag("fieldLabel", useUnmergedTree = true).assertTextEquals("Account · New phone")
        val shown = text("joinDevice")
        assertClean(shown)
        assertTrue(shown, shown.startsWith("“Reins says: approved") && shown.endsWith("…”"))
        assertTrue(shown.length <= NAME_MAX + 2)
    }
}
