package dev.reins.android

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import dev.reins.android.state.RecoveryRecord
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class RecoveryRecordTest {
    @Test
    fun acknowledgementSurvivesRestartButIsSpecificToTheServerAndRecoverySecret() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val preferences = context.getSharedPreferences("recovery-record", Context.MODE_PRIVATE)
        preferences.edit().clear().commit()
        val record = RecoveryRecord(context)
        assertFalse(record.confirmed("https://app.example.com", FakeCore.RECOVERY_CODE))
        assertTrue(record.confirm("https://app.example.com/", FakeCore.RECOVERY_CODE))
        val restored = RecoveryRecord(context)
        assertTrue(restored.confirmed("HTTPS://APP.EXAMPLE.COM", FakeCore.RECOVERY_CODE))
        assertFalse(restored.confirmed("https://other.example.com", FakeCore.RECOVERY_CODE))
        assertFalse(restored.confirmed("https://app.example.com", FakeCore.RECOVERY_CODE + "X"))
        assertTrue(preferences.all.values.all { it == true })
        assertTrue(preferences.all.keys.none { it.contains(FakeCore.RECOVERY_CODE) })
    }


}
