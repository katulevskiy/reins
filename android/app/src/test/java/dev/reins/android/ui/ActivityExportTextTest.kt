package dev.reins.android.ui

import dev.reins.android.ui.settings.ActivityExportText
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Test

class ActivityExportTextTest {
    @Test
    fun oneDecisionIsSingular() {
        assertEquals("Exported 1 decision", ActivityExportText.exported(1u, Locale.US))
    }

    @Test
    fun otherCountsArePlural() {
        assertEquals("Exported 2 decisions", ActivityExportText.exported(2u, Locale.US))
        assertEquals("Exported 0 decisions", ActivityExportText.exported(0u, Locale.US))
    }

    @Test
    fun largeCountsHaveSeparators() {
        assertEquals("Exported 5,000 decisions", ActivityExportText.exported(5000u, Locale.US))
    }
}
