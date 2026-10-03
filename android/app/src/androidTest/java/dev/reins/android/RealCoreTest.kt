package dev.reins.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.firebase.FirebaseApp
import dev.reins.android.core.MainSafeCore
import dev.reins.android.platform.KeystoreKeyWrapper
import dev.reins.core.CoreException
import dev.reins.core.ForeignException
import dev.reins.core.GoogleTokenProvider
import dev.reins.core.Notifier
import dev.reins.core.PendingItem
import dev.reins.core.ReinsCore
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

/** The real Rust library, real JNA loading and the real Android Keystore on a device; no UI involved. */
@RunWith(AndroidJUnit4::class)
class RealCoreTest {
    private val context get() = InstrumentationRegistry.getInstrumentation().targetContext
    private val dirs = mutableListOf<File>()

    private object NoGoogle : GoogleTokenProvider {
        override suspend fun accessToken(): String = throw ForeignException.NeedsUserInteraction()
    }

    private object NoNotifications : Notifier {
        override fun itemPending(item: PendingItem) = Unit
        override fun itemResolved(id: String) = Unit
        override fun autoDecided(decision: dev.reins.core.AutoDecisionView) = Unit
        override fun autopilotChanged(event: dev.reins.core.AutopilotEvent) = Unit
    }

    private fun newCore(): ReinsCore {
        val dir = File(context.cacheDir, "core-${UUID.randomUUID()}").also { dirs += it; it.mkdirs() }
        return ReinsCore(dir.absolutePath, KeystoreKeyWrapper("reins_test_${UUID.randomUUID()}"), NoGoogle, NoNotifications)
    }

    @After
    fun cleanUp() {
        dirs.forEach { it.deleteRecursively() }
    }

    @Test
    fun aFreshCoreOpensItsEncryptedStoreAndIsSignedOut() = runBlocking {
        val core = newCore()
        assertNull(core.session())
        assertTrue(core.pending().isEmpty())
        assertTrue(core.grants().isEmpty())
        assertTrue(core.activity(10u).isEmpty())
    }

    @Test
    fun theStoreSurvivesReopeningWithTheSameKeystoreKey() = runBlocking {
        val dir = File(context.cacheDir, "core-${UUID.randomUUID()}").also { dirs += it; it.mkdirs() }
        val alias = "reins_test_${UUID.randomUUID()}"
        ReinsCore(dir.absolutePath, KeystoreKeyWrapper(alias), NoGoogle, NoNotifications).close()
        val reopened = ReinsCore(dir.absolutePath, KeystoreKeyWrapper(alias), NoGoogle, NoNotifications)
        assertNull(reopened.session())
    }

    @Test
    fun aBadServerAddressIsRejectedBeforeAnyNetworkTraffic() = runBlocking {
        val core = newCore()
        try {
            core.login("http://evil.example.com", "me@example.com", "pw", null)
            fail("plain http to a public host must be refused")
        } catch (e: CoreException.Invalid) {
            assertTrue(e.reason.contains("https"))
        }
    }

    @Test
    fun anUnreachableServerIsANetworkErrorNotACrash() = runBlocking {
        val core = newCore()
        try {
            core.login("http://127.0.0.1:1", "me@example.com", "pw", null)
            fail("nothing listens on port 1")
        } catch (e: CoreException) {
            assertTrue(e is CoreException.Network || e is CoreException.Server)
        }
    }

    @Test
    fun mainSafeCoreNeverRunsCoreCodeOnTheMainThread() = runBlocking {
        val thread = java.util.concurrent.atomic.AtomicReference<String>()
        val core = MainSafeCore { newCore().also { thread.set(Thread.currentThread().name) } }
        // Called from the main dispatcher, as a ViewModel would.
        withContext(Dispatchers.Main) { assertNull(core.session()) }
        assertTrue("the core was built on ${thread.get()}", !thread.get().equals("main"))
    }

    @Test
    fun theKeystoreWrapperRoundTripsAndDetectsTampering() {
        val wrapper = KeystoreKeyWrapper("reins_test_${UUID.randomUUID()}")
        val secret = ByteArray(32) { it.toByte() }
        val wrapped = wrapper.wrap(secret)
        assertTrue(wrapped.size > secret.size)
        assertArrayEquals(secret, wrapper.unwrap(wrapped))
        assertTrue("fresh IV every time", !wrapper.wrap(secret).contentEquals(wrapped))
        wrapped[wrapped.lastIndex] = (wrapped.last().toInt() xor 1).toByte()
        try {
            wrapper.unwrap(wrapped)
            fail("a modified blob must not unwrap")
        } catch (e: ForeignException.Failed) {
            assertTrue(!e.reason.contains("00010203"))
        }
    }

    @Test
    fun firebaseIsInitializedExactlyWhenTheBuildHadGoogleServices() {
        assertEquals(BuildConfig.HAS_FIREBASE, FirebaseApp.getApps(context).isNotEmpty())
    }
}
