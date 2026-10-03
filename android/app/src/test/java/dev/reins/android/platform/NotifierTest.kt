package dev.rewarden.android.platform

import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.rewarden.android.TestData
import java.util.concurrent.atomic.AtomicInteger
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class NotifierTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val changes = AtomicInteger()
    private val notifier = AppNotifier(context) { changes.incrementAndGet() }
    private val manager = context.getSystemService(NotificationManager::class.java)

    @After
    fun tearDown() {
        Foreground.focused = false
    }

    @Test
    fun `a request notifies when the app is not in front and says who wants what`() {
        notifier.createChannels()
        Foreground.focused = false
        notifier.itemPending(TestData.pending("req1", "read", 3u))
        val posted = shadowOf(manager).allNotifications
        assertEquals(1, posted.size)
        assertEquals("Claude: Read 3 emails", posted.single().extras.getCharSequence("android.text").toString())
        assertEquals(1, changes.get())
    }

    @Test
    fun `nothing is posted while the app is on screen and focused, but the app still learns of the change`() {
        notifier.createChannels()
        Foreground.focused = true
        notifier.itemPending(TestData.pending("req1", "search", 2u))
        assertEquals(0, shadowOf(manager).allNotifications.size)
        assertEquals(1, changes.get())
    }

    @Test
    fun `a resolved item takes its notification away`() {
        notifier.createChannels()
        notifier.itemPending(TestData.pending("req1"))
        notifier.itemResolved("req1")
        assertEquals(0, shadowOf(manager).allNotifications.size)
    }

    @Test
    fun `permission requests say so`() {
        notifier.createChannels()
        notifier.itemPending(TestData.pending("req3", "grant", 1u))
        val title = shadowOf(manager).allNotifications.single().extras.getCharSequence("android.title").toString()
        assertEquals("Permission requested", title)
    }

    @Test
    fun `an upload notifies with what it is and opens its sheet`() {
        notifier.createChannels()
        notifier.itemPending(TestData.blobItem())
        val posted = shadowOf(manager).allNotifications.single()
        assertEquals("File to check", posted.extras.getCharSequence("android.title").toString())
        assertEquals("Claude: Share a file", posted.extras.getCharSequence("android.text").toString())
        val intent = shadowOf(posted.contentIntent).savedIntent
        assertEquals(AppNotifier.ACTION_OPEN_ITEM, intent.action)
        assertEquals("blob", intent.getStringExtra(AppNotifier.EXTRA_KIND))
        assertEquals("blob_0123456789abcdef", intent.getStringExtra(AppNotifier.EXTRA_ID))
    }
}
