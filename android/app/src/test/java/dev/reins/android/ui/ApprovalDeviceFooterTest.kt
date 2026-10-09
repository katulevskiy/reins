package dev.reins.android.ui

import dev.reins.android.ui.settings.approvalDeviceFooter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ApprovalDeviceFooterTest {
    @Test
    fun `the footer says how requests reach this phone`() {
        assertTrue(approvalDeviceFooter(false, true, true).startsWith("Only one phone"))
        assertTrue(approvalDeviceFooter(true, false, true).startsWith("Push notifications are not set up in this build"))
        // A server without push: the phone has to be open, whatever this build can do.
        assertEquals(
            "This server sends no push notifications: requests arrive only while Reins is open.",
            approvalDeviceFooter(true, true, false),
        )
        assertTrue(approvalDeviceFooter(true, true, true).startsWith("Requests reach this phone by push"))
        // An older server says nothing: no claim either way beyond what the build can do.
        assertTrue(approvalDeviceFooter(true, true, null).startsWith("Requests reach this phone by push"))
    }
}
