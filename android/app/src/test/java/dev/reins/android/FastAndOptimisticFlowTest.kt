package dev.reins.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.platform.update.FakeInstaller
import dev.reins.android.platform.update.FakeUpdateServer
import dev.reins.android.platform.update.UpdateProvider
import dev.reins.android.state.SessionState
import dev.reins.android.state.StartupSnapshot
import dev.reins.core.CoreException
import dev.reins.core.SessionInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Opening without waiting, answering without waiting, and the install permission asked where Install was tapped. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-xxhdpi")
class FastAndOptimisticFlowTest : FlowHarness() {
    @Test
    fun theSnapshotKeepsOnlyAPlainSessionToStartFrom() {
        val snapshot = StartupSnapshot(context.getSharedPreferences("startup-test", 0))
        assertNull("nothing kept yet: wait for the core", snapshot.read())
        val info = SessionInfo("https://app.reins2fa.com", "me@example.com")
        snapshot.write(SessionState.SignedIn(info), plain = true)
        assertEquals(SessionState.SignedIn(info), snapshot.read())
        snapshot.write(SessionState.SignedIn(info), plain = false)
        assertNull("a recovery code, setup or unlock to do first: start from the core", snapshot.read())
        snapshot.write(SessionState.SignedOut, plain = false)
        assertEquals(SessionState.SignedOut, snapshot.read())
    }

    @Test
    fun anApprovalThatFailsInTheBackgroundComesBackWithAShortMessage() {
        core.approveFailure = CoreException.Network("offline")
        openRequest(TestData.searchView())
        tap("approve")
        awaitGone("approve")
        awaitTag("flash")
        assertTrue(showsText("Not approved:", substring = true))
        awaitTag("pending:req1")
    }

    @Test
    fun installTappedWithoutPermissionAsksRightThereAndFinishesOnReturn() {
        assumeTrue(BuildConfig.SELF_UPDATE)
        val installer = FakeInstaller().apply { allowed = false }
        UpdateProvider.installer = installer
        (UpdateProvider.fetcher as FakeUpdateServer).publish(29_834_567, versionName = "0.9.0")
        launch()
        tap("openSettings")
        tap("installUpdate")
        awaitTag("allowInstalls")
        tap("allowInstalls")
        awaitCore { installer.permissionScreens == 1 }
        // Back from Android's setting with it allowed: the install goes on by itself.
        installer.allowed = true
        scenario!!.onActivity { container.updates!!.onForeground() }
        awaitCore { installer.installed.isNotEmpty() }
    }
}
