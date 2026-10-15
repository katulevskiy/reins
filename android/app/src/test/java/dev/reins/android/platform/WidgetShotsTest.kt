package dev.reins.android.platform

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import androidx.compose.ui.graphics.toArgb
import android.view.View
import android.widget.FrameLayout
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.glance.appwidget.ExperimentalGlanceRemoteViewsApi
import androidx.glance.appwidget.GlanceRemoteViews
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.reins.android.MainActivity
import dev.reins.android.TestData
import dev.reins.android.platform.widgets.AutopilotWidgetContent
import dev.reins.android.platform.widgets.AutopilotWidgetModel
import dev.reins.android.platform.widgets.WaitingWidgetContent
import dev.reins.android.platform.widgets.waitingWidgetModel
import dev.reins.core.AutopilotMode
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The widgets' content as the launcher would draw it: composed to RemoteViews by Glance, inflated, and drawn to PNG
 * (`-Dreins.screenshots=/dir`, skipped otherwise). The launcher's own frame and corner clipping are not part of it.
 */
@OptIn(ExperimentalGlanceRemoteViewsApi::class)
abstract class WidgetShotsBase(private val theme: String) {
    private val context: Context = ApplicationProvider.getApplicationContext()

    private fun shoot(name: String, size: DpSize, content: @Composable () -> Unit) {
        val dir = System.getProperty("reins.screenshots")
        assumeTrue(dir != null)
        val result = runBlocking { GlanceRemoteViews().compose(context, size) { content() } }
        val parent = FrameLayout(context)
        val view = result.remoteViews.apply(context, parent)
        val density = context.resources.displayMetrics.density
        val w = (size.width.value * density).toInt()
        val h = (size.height.value * density).toInt()
        // A little of the wallpaper's dark around the widget, as on a home screen.
        val pad = (12 * density).toInt()
        parent.addView(view, FrameLayout.LayoutParams(w, h))
        parent.measure(View.MeasureSpec.makeMeasureSpec(w, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(h, View.MeasureSpec.EXACTLY))
        parent.layout(0, 0, w, h)
        val bitmap = Bitmap.createBitmap(w + 2 * pad, h + 2 * pad, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawColor(0xFF2A2650.toInt())
        canvas.translate(pad.toFloat(), pad.toFloat())
        parent.draw(canvas)
        File(dir!!).mkdirs()
        File(dir, "$name-$theme.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    private val open get() = Intent(context, MainActivity::class.java)

    @Test
    fun widgetWaiting() {
        val model = waitingWidgetModel(listOf(TestData.pending("req1", "read", 3u, quick = true), TestData.pending("req2", "send", 1u)))
        shoot("sg-60-widget-waiting", DpSize(300.dp, 160.dp)) { WaitingWidgetContent(model, open) }
    }

    /** A command, written the one way (ASK-TEXT.md): the folder in the serif, the matched part in mono, the icon. */
    @Test
    fun widgetWaitingCommand() {
        val run = TestData.pending("req1", "write", 1u, waitUntil = 1_700_000_160, label = "Claude Code", service = "desktop", op = "ask")
            .copy(label = TestData.label("terminal", "delete", "Run", "rm -rf jarx && npm run build && npm test", code = true))
        shoot("sg-63-widget-command", DpSize(300.dp, 180.dp)) { WaitingWidgetContent(waitingWidgetModel(listOf(run)), open) }
    }

    /** The notifications' large icons (drawn without Compose), each kind ringed in its field, as on a heads-up. */
    @Test
    fun notificationIcons() {
        val dir = System.getProperty("reins.screenshots")
        assumeTrue(dir != null)
        val kinds = listOf("github" to "push", "terminal" to "delete", "terminal" to "run", "gmail" to "read", "file" to "read", "secrets" to "secrets", "ssh" to "secrets", "mcp:linear" to "write", "payments" to "pay", "desktop" to "ask")
        val px = (64 * context.resources.displayMetrics.density).toInt()
        val gap = px / 4
        val bitmap = Bitmap.createBitmap(kinds.size * (px + gap) + gap, px + 2 * gap, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        canvas.drawColor(if (theme == "dark") 0xFF121215.toInt() else 0xFFF6F6F8.toInt())
        kinds.forEachIndexed { i, (provider, action) ->
            val ring = dev.reins.android.ui.common.actionField(action).fill().toArgb()
            val icon = dev.reins.android.ui.common.IntegrationBitmap.draw(context, provider, action, px, ring)
            val x = (gap + i * (px + gap)).toFloat()
            canvas.drawRoundRect(x - gap / 3f, gap * 2f / 3f, x + px + gap / 3f, gap + px + gap / 3f, gap / 2f, gap / 2f, android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply { color = ring })
            canvas.drawBitmap(icon, x, gap.toFloat(), null)
        }
        File(dir!!).mkdirs()
        File(dir, "sg-64-notification-icons-$theme.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    @Test
    fun widgetPond() {
        shoot("sg-61-widget-pond", DpSize(300.dp, 160.dp)) { WaitingWidgetContent(waitingWidgetModel(emptyList()), open) }
    }

    @Test
    fun widgetAutopilot() {
        shoot("sg-62-widget-autopilot", DpSize(150.dp, 150.dp)) { AutopilotWidgetContent(AutopilotWidgetModel(AutopilotMode.AUTO, 9), open) }
    }
}

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-notnight-xxhdpi")
class WidgetShotsLight : WidgetShotsBase("light")

@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-normal-night-xxhdpi")
class WidgetShotsDark : WidgetShotsBase("dark")
