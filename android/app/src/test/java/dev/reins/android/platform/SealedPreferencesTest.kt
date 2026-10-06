package dev.reins.android.platform

import androidx.test.core.app.ApplicationProvider
import android.content.Context
import dev.reins.android.TestNativeKeys
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SealedPreferencesTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()
    @Before fun keys() {
        TestNativeKeys.install()
        context.getSharedPreferences("sealed_test", Context.MODE_PRIVATE).edit().clear().commit()
    }
    @Test fun sensitiveMetadataIsCiphertextAndCannotBeMovedToAnotherPreference() {
        val sealed = SealedPreferences(context, "sealed_test")
        sealed.edit().putString("activity", "Alice's private laptop and vault").commit()
        assertEquals("Alice's private laptop and vault", sealed.getString("activity", null))
        val disk = context.getSharedPreferences("sealed_test", Context.MODE_PRIVATE)
        val ciphertext = disk.getString("activity", null)!!
        assertFalse(ciphertext.contains("Alice"))
        disk.edit().putString("integration", ciphertext).commit()
        assertNull(sealed.getString("integration", null))
        disk.edit().putString("activity", "corrupted").commit()
        assertNull(sealed.getString("activity", null))
    }
    @Test fun legacyPlaintextIsRemovedBeforeAnAccountCanUseTheCache() {
        val disk = context.getSharedPreferences("sealed_test", Context.MODE_PRIVATE)
        disk.edit().putString("activity", "another account's activity").commit()
        val sealed = SealedPreferences(context, "sealed_test")
        assertNull(sealed.getString("activity", null))
        assertFalse(disk.contains("activity"))
    }
    @Test fun accountStatusDoesNotFollowASecondLogin() {
        val status = dev.reins.android.state.DeviceStatusStore(context)
        status.clear()
        status.selectAccount(dev.reins.core.SessionInfo("https://reins.example", "alice@example.com"))
        status.setSeenActivityId(321)
        status.setApprovalDevice(true)
        status.selectAccount(dev.reins.core.SessionInfo("https://reins.example", "bob@example.com"))
        assertEquals(0L, status.seenActivityId())
        assertFalse(status.isApprovalDevice())
        val disk = context.getSharedPreferences("device_status", Context.MODE_PRIVATE)
        assertFalse(disk.all.values.any { it.toString().contains("example.com") })
    }
}
