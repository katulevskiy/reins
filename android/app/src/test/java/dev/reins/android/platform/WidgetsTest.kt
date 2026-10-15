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
import dev.reins.android.platform.widgets.waitingLabel
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
 * The home screen widgets (And-Outside 04): the Waiting widget carries the top card of the stack, or the pond; the
 * Autopilot widget the mode and Lock down. Nothing is ever approved from a widget: it opens the request.
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
        assertEquals("Waiting · 2", waitingLabel(model))
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
    fun `the waiting widget shows the ask and Review, never Approve`() = runGlanceAppWidgetUnitTest {
        setAppWidgetSize(DpSize(250.dp, 140.dp))
        setContext(context)
        val model = waitingWidgetModel(listOf(TestData.pending("req1", "read", 3u, quick = true), TestData.pending("req2", "send", 1u)))
        provideComposable { WaitingWidgetContent(model, Intent(context, MainActivity::class.java)) }
        onNode(hasText("WAITING · 2")).assertExists()
        onNode(hasText("Read 3 emails")).assertExists()
        onNode(hasText("Review")).assertExists()
        onAllNodes(hasText("Approve")).assertCountEquals(0)
        onAllNodes(hasText("Allow")).assertCountEquals(0)
    }

    @Test
    fun `with nothing waiting the widget is the pond`() = runGlanceAppWidgetUnitTest {
        setAppWidgetSize(DpSize(250.dp, 140.dp))
        setContext(context)
        provideComposable { WaitingWidgetContent(waitingWidgetModel(emptyList()), Intent(context, MainActivity::class.java)) }
        onNode(hasText("Nothing waits.")).assertExists()
        onAllNodes(hasText("Review")).assertCountEquals(0)
    }

    @Test
    fun `the autopilot widget shows the mode and offers Lock down until it is`() = runGlanceAppWidgetUnitTest {
        setAppWidgetSize(DpSize(140.dp, 140.dp))
        setContext(context)
        provideComposable { AutopilotWidgetContent(AutopilotWidgetModel(AutopilotMode.AUTO, 9), Intent(context, MainActivity::class.java)) }
        onNode(hasText("Auto")).assertExists()
        onNode(hasText("9 decided today")).assertExists()
        onNode(hasText("Lock down")).assertExists()
        assertTrue(AutopilotWidgetModel(AutopilotMode.AUTO, 0).offersLockDown)
        assertFalse(AutopilotWidgetModel(AutopilotMode.LOCKDOWN, 0).offersLockDown)
    }
}
