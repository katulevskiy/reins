package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.design.Platforms
import dev.reins.android.design.autoIcon
import dev.reins.android.design.iconName
import dev.reins.android.design.shownIcon
import dev.reins.android.ui.activity.askedWho
import dev.reins.android.ui.activity.askerLine
import org.junit.Assert.assertEquals
import org.junit.Test

/** Which icon a connection shows, what the Icon row calls it, and who asked, in Activity. */
class ConnectionIconsTest {
    @Test
    fun `the pick wins, else what the core says Auto shows`() {
        val mac = TestData.computer("d1", "M2A", platform = "macos")
        assertEquals("macos", shownIcon(mac))
        assertEquals("blob", shownIcon(mac.copy(icon = "blob")))
        assertEquals("claude", shownIcon(mac.copy(icon = "claude")))
        assertEquals("linux", shownIcon(TestData.computer("d2", "honeypot", platform = "linux")))
        assertEquals("a computer of unknown platform", "blob", shownIcon(TestData.computer("d3", "box", platform = null)))
        assertEquals("grok", shownIcon(TestData.connection("c2", "Grok")))
        assertEquals("blob", shownIcon(TestData.connection("c3", "notes-bot")))
        // The core's word counts, not the name's: a computer named after an AI still shows its platform.
        assertEquals("windows", shownIcon(TestData.computer("d4", "claude box", platform = "windows")))
    }

    @Test
    fun `older data without an automatic icon falls back to the name`() {
        assertEquals("claude", autoIcon(TestData.legacyConnection("c1", "My Claude")))
        assertEquals("blob", autoIcon(TestData.legacyConnection("c1", "notes-bot")))
        assertEquals("openai", shownIcon(TestData.legacyConnection("c1", "notes-bot", icon = "openai")))
    }

    @Test
    fun `the icon row names the pick`() {
        assertEquals("Auto", iconName(null))
        assertEquals("Blobatar", iconName("blob"))
        assertEquals("macOS", iconName("macos"))
        assertEquals("Windows", iconName("windows"))
        assertEquals("Linux", iconName("linux"))
        assertEquals("Grok", iconName("grok"))
        assertEquals(listOf("macos", "windows", "linux"), Platforms.all.map { it.key })
    }

    @Test
    fun `an activity row says which agent asked from which computer`() {
        val entry = TestData.entry(1).copy(connectionLabel = "M2A", connectionId = "d1")
        assertEquals("no agent known", "M2A", askerLine(entry))
        assertEquals("Claude Code · M2A", askerLine(entry.copy(request = TestData.asked("ls", agent = "Claude Code"))))
        assertEquals("M2A", askerLine(entry.copy(request = TestData.asked("ls", agent = "  "))))
        assertEquals("the agent is the connection", "Claude", askerLine(TestData.entry(2).copy(request = TestData.asked("ls", agent = "Claude"))))
    }

    @Test
    fun `the entry's who names the computer rather than its host`() {
        val entry = TestData.entry(1).copy(connectionLabel = "M2A", connectionId = "d1")
        assertEquals("Claude Code · M2A", askedWho(entry.copy(request = TestData.asked("ls", agent = "Claude Code")), "127.0.0.1"))
        assertEquals("M2A · 127.0.0.1", askedWho(entry.copy(request = TestData.asked("ls")), "127.0.0.1"))
        assertEquals("M2A", askedWho(entry.copy(request = TestData.asked("ls")), null))
    }
}
