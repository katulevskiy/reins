package dev.reins.android.platform

import android.content.Context
import android.content.Intent
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.glance.appwidget.testing.unit.runGlanceAppWidgetUnitTest
import androidx.glance.testing.unit.hasText
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.MainActivity
import dev.reins.android.TestData
import dev.reins.android.design.Field
import dev.reins.android.platform.widgets.AutopilotWidgetContent
import dev.reins.android.platform.widgets.AutopilotWidgetModel
import dev.reins.android.platform.widgets.WaitingWidgetContent
import dev.reins.android.platform.widgets.openIntent
import dev.reins.android.platform.widgets.waitingCount
import dev.reins.android.platform.widgets.latestWidgetModel
import dev.reins.android.platform.widgets.LatestWidgetContent
import dev.reins.android.platform.widgets.waitingWidgetModel
import dev.reins.core.AutopilotMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The home screen widgets (And-Outside 05): Waiting (the count on the top card, or the list), Mode and Lock down,
 * and Latest. Nothing is ever approved from a widget: it opens the request.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class WidgetsTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    @Test
    fun `the waiting model is the top card of the stack, or nothing`() {
        val empty = waitingWidgetModel(emptyList())
        assertEquals(0, empty.waiting)
        assertNull(empty.top)
        val model = waitingWidgetModel(listOf(TestData.pending("req1", "read", 3u, quick = true), TestData.pending("req2", "send", 1u)))
        assertEquals(2, model.waiting)
        assertEquals("2 waiting", waitingCount(model))
        assertEquals(listOf("req1", "req2"), model.rows.map { it.id })
        val top = model.top!!
        assertEquals("req1", top.id)
        assertEquals(Field.Read, top.field)
        assertEquals("Read 3 emails", top.ask)
    }

    @Test
    fun `the widget opens the request it shows`() {
        val model = waitingWidgetModel(listOf(TestData.pending("req1", "read", 3u, quick = true)))
        val intent = openIntent(context, model.top)
        assertEquals(MainActivity::class.java.name, intent.component?.className)
        assertEquals(AppNotifier.ACTION_OPEN_ITEM, intent.action)
        assertEquals("req1", intent.getStringExtra(AppNotifier.EXTRA_ID))
        // With nothing waiting it opens Reins.
        assertNull(openIntent(context, null).action)
    }

    @Test
    fun `the small waiting widget shows the count and the ask, never Approve`() = runGlanceAppWidgetUnitTest {
        setAppWidgetSize(DpSize(150.dp, 150.dp))
        setContext(context)
        val model = waitingWidgetModel(listOf(TestData.pending("req1", "read", 3u, quick = true), TestData.pending("req2", "send", 1u)))
        provideComposable { WaitingWidgetContent(model, Intent(context, MainActivity::class.java), DpSize(150.dp, 150.dp)) }
        onNode(hasText("2")).assertExists()
        onNode(hasText("Read 3 emails")).assertExists()
        onAllNodes(hasText("Approve")).assertCountEquals(0)
        onAllNodes(hasText("Allow")).assertCountEquals(0)
    }

    @Test
    fun `the wide waiting widget lists what waits, with who asks`() = runGlanceAppWidgetUnitTest {
        setAppWidgetSize(DpSize(300.dp, 160.dp))
        setContext(context)
        val model = waitingWidgetModel(listOf(TestData.pending("req1", "read", 3u, quick = true), TestData.pending("req2", "send", 1u)))
        provideComposable { WaitingWidgetContent(model, Intent(context, MainActivity::class.java), DpSize(300.dp, 160.dp)) }
        onNode(hasText("2 waiting")).assertExists()
        onNode(hasText("Read 3 emails")).assertExists()
        onAllNodes(hasText("Approve")).assertCountEquals(0)
        onAllNodes(hasText("Allow")).assertCountEquals(0)
    }

    @Test
    fun `with nothing waiting the widget says so`() = runGlanceAppWidgetUnitTest {
        setAppWidgetSize(DpSize(250.dp, 140.dp))
        setContext(context)
        provideComposable { WaitingWidgetContent(waitingWidgetModel(emptyList()), Intent(context, MainActivity::class.java)) }
        onNode(hasText("Nothing waits")).assertExists()
    }

    @Test
    fun `latest keeps the asker's name apart from what it did, newest first`() {
        val long = "Totally Reins: approved by you " + "x".repeat(60)
        val entries = listOf(
            TestData.entry(1, "search", "released", at = 1_700_000_000),
            TestData.entry(2, "send", "denied", at = 1_700_000_100).copy(connectionLabel = long),
            TestData.entry(3, "search", "started", at = 1_699_999_000),
            TestData.entry(4, "search", "released", at = 1_600_000_000),
        )
        val model = latestWidgetModel(entries, 1_700_000_200)
        assertEquals(listOf("2", "1", "3"), model.rows.map { it.id })
        val denied = model.rows[0]
        assertEquals(false, denied.allowed)
        assertTrue(denied.who.length <= 33)
        assertFalse(denied.action.contains("Totally"))
        assertEquals(true, model.rows[1].allowed)
        assertNull(model.rows[2].allowed)
    }

    @Test
    fun `the latest widget shows its rows`() = runGlanceAppWidgetUnitTest {
        setAppWidgetSize(DpSize(300.dp, 160.dp))
        setContext(context)
        val model = latestWidgetModel(listOf(TestData.entry(1, "search", "released", at = 1_700_000_000)), 1_700_000_200)
        provideComposable { LatestWidgetContent(model, Intent(context, MainActivity::class.java)) }
        onNode(hasText("Latest")).assertExists()
        onNode(hasText("3m")).assertExists()
    }

    @Test
    fun `the autopilot widget shows the mode and offers Lock down until it is`() = runGlanceAppWidgetUnitTest {
        setAppWidgetSize(DpSize(140.dp, 140.dp))
        setContext(context)
        provideComposable { AutopilotWidgetContent(AutopilotWidgetModel(AutopilotMode.AUTO, 9), Intent(context, MainActivity::class.java)) }
        onNode(hasText("Auto")).assertExists()
        onNode(hasText("Mode")).assertExists()
        onNode(hasText("9 decided today")).assertExists()
        onNode(hasText("Lock down")).assertExists()
        assertTrue(AutopilotWidgetModel(AutopilotMode.AUTO, 0).offersLockDown)
        assertFalse(AutopilotWidgetModel(AutopilotMode.LOCKDOWN, 0).offersLockDown)
    }
}
