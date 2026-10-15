package dev.reins.android.platform

import android.app.NotificationManager
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.TestData
import java.util.concurrent.atomic.AtomicInteger
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
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
        assertEquals("Read 3 emails", posted.single().extras.getCharSequence("android.title").toString())
        assertEquals("Claude", posted.single().extras.getCharSequence("android.text").toString())
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
    fun `a heads-up reads as the ask, its target in italic, the command in mono and the request's icon beside it`() {
        notifier.createChannels()
        val item = TestData.pending("req1", "write", 1u, label = "Claude Code", service = "desktop", op = "ask", quick = true)
            .copy(label = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins"))
        notifier.itemPending(item)
        val posted = shadowOf(manager).allNotifications.single()
        val title = posted.extras.getCharSequence("android.title")!!
        assertEquals("Push 3 commits to katulevskiy/reins", title.toString())
        val spans = (title as android.text.Spanned).getSpans(0, title.length, android.text.style.StyleSpan::class.java)
        assertEquals("one italic run, the target", 1, spans.size)
        assertEquals(android.graphics.Typeface.ITALIC, spans[0].style)
        assertEquals("katulevskiy/reins", title.subSequence(title.getSpanStart(spans[0]), title.getSpanEnd(spans[0])).toString())
        assertEquals("Claude Code", posted.extras.getCharSequence("android.text").toString())
        assertTrue("the integration's icon with the action on its corner", posted.getLargeIcon() != null)

        notifier.itemPending(TestData.pending("req2", "write", 1u, service = "desktop", op = "ask").copy(label = TestData.label("terminal", "run", "Run", "rm -rf build && make", code = true)))
        val run = shadowOf(manager).allNotifications.first { it.extras.getCharSequence("android.title").toString() == "Run a command" }
        val text = run.extras.getCharSequence("android.text") as android.text.Spanned
        assertEquals("rm -rf build && make", text.toString())
        assertEquals(1, text.getSpans(0, text.length, android.text.style.TypefaceSpan::class.java).size)
        assertEquals("Claude", run.extras.getCharSequence("android.subText").toString())
    }

    @Test
    fun `permission requests say so`() {
        notifier.createChannels()
        notifier.itemPending(TestData.pending("req3", "grant", 1u))
        val title = shadowOf(manager).allNotifications.single().extras.getCharSequence("android.title").toString()
        assertEquals("Ask for access", title)
    }

    @Test
    fun `an upload notifies with what it is and opens its sheet`() {
        notifier.createChannels()
        notifier.itemPending(TestData.blobItem())
        val posted = shadowOf(manager).allNotifications.single()
        assertEquals("Share a file", posted.extras.getCharSequence("android.title").toString())
        assertEquals("Claude", posted.extras.getCharSequence("android.text").toString())
        val intent = shadowOf(posted.contentIntent).savedIntent
        assertEquals(AppNotifier.ACTION_OPEN_ITEM, intent.action)
        assertEquals("blob", intent.getStringExtra(AppNotifier.EXTRA_KIND))
        assertEquals("blob_0123456789abcdef", intent.getStringExtra(AppNotifier.EXTRA_ID))
    }

    @Test
    fun `a routine request can be approved or denied right from the notification, once the phone is unlocked`() {
        notifier.createChannels()
        notifier.itemPending(TestData.pending("req1", "search", 2u, headline = "Claude gets the 2 emails found.", quick = true))
        val posted = shadowOf(manager).allNotifications.single()
        val actions = posted.actions.orEmpty()
        assertEquals(listOf("Deny", "Approve"), actions.map { it.title.toString() })
        assertEquals(false, actions[0].isAuthenticationRequired)
        assertEquals(true, actions[1].isAuthenticationRequired)
        val approve = shadowOf(actions[1].actionIntent).savedIntent
        assertEquals(ApprovalActionReceiver.ACTION_APPROVE, approve.action)
        assertEquals("req1", approve.getStringExtra(AppNotifier.EXTRA_ID))
        val big = posted.extras.getCharSequence("android.bigText").toString()
        assertEquals("in:inbox\nClaude gets the 2 emails found.", big)
    }

    @Test
    fun `what is asked every time or needs a closer look only opens the sheet`() {
        notifier.createChannels()
        notifier.itemPending(TestData.pending("req1", "grant", 1u))
        assertEquals(0, shadowOf(manager).allNotifications.single().actions.orEmpty().size)
    }
}
