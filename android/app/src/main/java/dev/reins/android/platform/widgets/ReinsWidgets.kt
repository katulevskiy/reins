package dev.reins.android.platform.widgets

import android.content.Context
import android.content.Intent
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.Image
import androidx.glance.ImageProvider
import androidx.glance.action.ActionParameters
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.LocalSize
import androidx.glance.appwidget.action.ActionCallback
import androidx.glance.appwidget.action.actionRunCallback
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.appwidget.updateAll
import androidx.glance.background
import androidx.glance.color.ColorProvider
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.ContentScale
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.size
import androidx.glance.layout.width
import androidx.glance.semantics.contentDescription
import androidx.glance.semantics.semantics
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import dev.reins.android.MainActivity
import dev.reins.android.ReinsApp
import dev.reins.android.design.Glyph
import dev.reins.android.design.RColors
import dev.reins.android.ui.common.IntegrationBitmap
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AppNotifier
import dev.reins.core.AutopilotMode
import dev.reins.core.PendingKind
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

// The home screen widgets (And-Outside 05), with Glance: Waiting (the count on the top card's field, or the waiting list
// when wide), Mode (and Lock down), and Latest. Their content is these composables over plain models, so a test can
// render it; the widgets only read the app's state and open Reins. Nothing is ever approved from a widget.

private val Paper = ColorProvider(RColors.Light.paperRaised, RColors.Dark.paperRaised)
private val Ink = ColorProvider(RColors.Light.ink, RColors.Dark.ink)
private val InkMeta = ColorProvider(RColors.Light.inkMeta, RColors.Dark.inkMeta)
private val Sunken = ColorProvider(RColors.Light.paperSunken, RColors.Dark.paperSunken)
private val OnField = ColorProvider(Color.White, Color.White)
private val OnField2 = ColorProvider(Color.White.copy(alpha = 0.82f), Color.White.copy(alpha = 0.82f))

/** The widgets' sizes: the small square, and the wide one that lists. */
internal val SMALL = DpSize(150.dp, 150.dp)
internal val WIDE = DpSize(300.dp, 160.dp)

/** Wide enough for a list. */
private fun isWide(size: DpSize) = size.width >= 220.dp

@Composable
private fun GlyphImage(glyph: Glyph, light: Color, dark: Color, size: Dp) {
    val context = androidx.glance.LocalContext.current
    val px = (size.value * context.resources.displayMetrics.density).toInt().coerceAtLeast(1)
    val night = (context.resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) == android.content.res.Configuration.UI_MODE_NIGHT_YES
    Image(ImageProvider(IntegrationBitmap.glyph(glyph, px, (if (night) dark else light).toArgb())), contentDescription = null, modifier = GlanceModifier.size(size))
}

@Composable
private fun RequestIcon(provider: String, action: String, ring: Int, size: Dp) {
    val context = androidx.glance.LocalContext.current
    val px = (size.value * context.resources.displayMetrics.density).toInt().coerceAtLeast(1)
    val icon = runCatching { IntegrationBitmap.draw(context, provider, action, px, ring) }.getOrNull() ?: return
    Image(ImageProvider(icon), contentDescription = null, modifier = GlanceModifier.size(size))
}

private fun ringColor(context: Context): Int {
    val night = (context.resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) == android.content.res.Configuration.UI_MODE_NIGHT_YES
    return (if (night) RColors.Dark.paperRaised else RColors.Light.paperRaised).toArgb()
}

/** Opens Reins on [model]'s top card (its request, pairing, file or phone), or on the stack. */
fun openIntent(context: Context, top: WaitingWidgetModel.TopCard?): Intent = openItem(context, top?.id, top?.kind)

private fun openItem(context: Context, id: String?, kind: PendingKind?): Intent {
    val intent = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    if (id == null || kind == null) return intent
    return intent.setAction(AppNotifier.ACTION_OPEN_ITEM)
        .putExtra(
            AppNotifier.EXTRA_KIND,
            when (kind) {
                PendingKind.REQUEST -> AppNotifier.KIND_REQUEST
                PendingKind.PAIRING -> AppNotifier.KIND_PAIRING
                PendingKind.BLOB -> AppNotifier.KIND_BLOB
                PendingKind.JOIN -> AppNotifier.KIND_JOIN
            },
        )
        .putExtra(AppNotifier.EXTRA_ID, id)
}

/** A list widget's head: a small glyph and the words, in meta. */
@Composable
private fun ListHead(text: String, glyph: Glyph?) {
    Row(GlanceModifier.fillMaxWidth().padding(bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        glyph?.let {
            GlyphImage(it, RColors.Light.inkMeta, RColors.Dark.inkMeta, 14.dp)
            Spacer(GlanceModifier.width(6.dp))
        }
        Text(text, style = TextStyle(color = InkMeta, fontSize = 12.sp, fontWeight = FontWeight.Medium), maxLines = 1)
    }
}

/**
 * Waiting. Small: the top card's field, its icon, the count large and the ask under it. Wide: "3 waiting" and the
 * list, each row its icon, the ask, and who asks. With nothing waiting, paper and "Nothing waits". Every part only
 * opens Reins; approving happens there.
 */
@Composable
fun WaitingWidgetContent(model: WaitingWidgetModel, open: Intent, size: DpSize = LocalSize.current) {
    val context = androidx.glance.LocalContext.current
    val top = model.top
    if (top == null) {
        Column(
            GlanceModifier.fillMaxSize().cornerRadius(24.dp).background(Paper).padding(16.dp).clickable(actionStartActivity(open)),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GlyphImage(Glyph.Tray, RColors.Light.inkMeta, RColors.Dark.inkMeta, 26.dp)
            Spacer(GlanceModifier.height(8.dp))
            Text("Nothing waits", style = TextStyle(color = Ink, fontSize = 15.sp, fontWeight = FontWeight.Medium))
        }
        return
    }
    if (isWide(size)) {
        Column(GlanceModifier.fillMaxSize().cornerRadius(24.dp).background(Paper).padding(horizontal = 14.dp, vertical = 12.dp).clickable(actionStartActivity(open))) {
            ListHead(waitingCount(model), Glyph.Tray)
            model.rows.forEach { row ->
                Row(
                    GlanceModifier.fillMaxWidth().height(30.dp).clickable(actionStartActivity(openItem(context, row.id, row.kind)))
                        .semantics { contentDescription = "${row.ask}. ${row.who}. Opens the request." },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RequestIcon(row.provider, row.action, ringColor(context), 22.dp)
                    Spacer(GlanceModifier.width(10.dp))
                    Text(row.ask, style = TextStyle(color = Ink, fontSize = 13.sp, fontWeight = FontWeight.Medium), maxLines = 1, modifier = GlanceModifier.defaultWeight())
                    if (row.who.isNotEmpty()) {
                        Spacer(GlanceModifier.width(8.dp))
                        Text(row.who, style = TextStyle(color = InkMeta, fontSize = 12.sp), maxLines = 1)
                    }
                }
            }
            if (model.waiting > model.rows.size) {
                Text("+${model.waiting - model.rows.size} more", style = TextStyle(color = InkMeta, fontSize = 12.sp), maxLines = 1)
            }
        }
        return
    }
    Box(
        GlanceModifier.fillMaxSize().cornerRadius(24.dp).background(ImageProvider(GrainBitmap.of(top.field.fill().toArgb())), ContentScale.FillBounds)
            .clickable(actionStartActivity(open))
            .semantics { contentDescription = "${waitingCount(model)}. ${top.ask}. ${top.who}. Opens the request." },
    ) {
        Column(GlanceModifier.fillMaxSize().padding(14.dp)) {
            RequestIcon(top.provider, top.action, top.field.fill().toArgb(), 24.dp)
            Text("${model.waiting}", style = TextStyle(color = OnField, fontSize = 40.sp, fontWeight = FontWeight.Medium), maxLines = 1)
            Spacer(GlanceModifier.defaultWeight())
            Text(top.ask, style = TextStyle(color = OnField, fontSize = 14.sp, fontWeight = FontWeight.Medium), maxLines = 2)
            if (top.who.isNotEmpty()) Text(top.who, style = TextStyle(color = OnField2, fontSize = 11.sp), maxLines = 1)
        }
    }
}

/**
 * Mode: "Mode", the mode with its glyph, and Lock down, which only takes power away; the rest opens Trust. Locked down,
 * the button opens Trust, where ending it asks the owner.
 */
@Composable
fun AutopilotWidgetContent(model: AutopilotWidgetModel, open: Intent) {
    val glyph = when (model.mode) {
        AutopilotMode.AUTO -> Glyph.AutopilotFilled
        AutopilotMode.LOCKDOWN -> Glyph.Block
        else -> Glyph.Person
    }
    Column(GlanceModifier.fillMaxSize().cornerRadius(24.dp).background(Paper).padding(14.dp).clickable(actionStartActivity(open))) {
        Text("Mode", style = TextStyle(color = InkMeta, fontSize = 12.sp, fontWeight = FontWeight.Medium), maxLines = 1)
        Spacer(GlanceModifier.defaultWeight())
        Row(verticalAlignment = Alignment.CenterVertically) {
            when (model.mode) {
                AutopilotMode.AUTO -> GlyphImage(glyph, RColors.Light.violet, RColors.Dark.violet, 18.dp)
                AutopilotMode.LOCKDOWN -> GlyphImage(glyph, RColors.Light.kindWarn, RColors.Dark.kindWarn, 18.dp)
                else -> GlyphImage(glyph, RColors.Light.ink, RColors.Dark.ink, 18.dp)
            }
            Spacer(GlanceModifier.width(6.dp))
            Text(model.name, style = TextStyle(color = Ink, fontSize = if (model.name.length > 7) 18.sp else 22.sp, fontWeight = FontWeight.Medium), maxLines = 1)
        }
        Text(model.line, style = TextStyle(color = InkMeta, fontSize = 11.sp), maxLines = 1)
        Spacer(GlanceModifier.defaultWeight())
        Row(
            GlanceModifier.fillMaxWidth().height(36.dp).cornerRadius(18.dp).background(Sunken)
                .clickable(if (model.offersLockDown) actionRunCallback<LockDownAction>() else actionStartActivity(open)),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GlyphImage(Glyph.Block, RColors.Light.ink, RColors.Dark.ink, 14.dp)
            Spacer(GlanceModifier.width(5.dp))
            Text(if (model.offersLockDown) "Lock down" else "Locked down", style = TextStyle(color = Ink, fontSize = 13.sp, fontWeight = FontWeight.Medium), maxLines = 1)
        }
    }
}

/** Latest: the last few entries, who (apart) did what, when, and a tick or a cross; it opens Activity. */
@Composable
fun LatestWidgetContent(model: LatestWidgetModel, open: Intent) {
    val context = androidx.glance.LocalContext.current
    Column(GlanceModifier.fillMaxSize().cornerRadius(24.dp).background(Paper).padding(horizontal = 14.dp, vertical = 12.dp).clickable(actionStartActivity(open))) {
        ListHead("Latest", null)
        if (model.rows.isEmpty()) {
            Text("Nothing yet", style = TextStyle(color = InkMeta, fontSize = 13.sp), maxLines = 1)
        }
        model.rows.forEach { row ->
            Row(GlanceModifier.fillMaxWidth().height(30.dp), verticalAlignment = Alignment.CenterVertically) {
                RequestIcon(row.provider, row.actionKey, ringColor(context), 22.dp)
                Spacer(GlanceModifier.width(10.dp))
                Row(GlanceModifier.defaultWeight(), verticalAlignment = Alignment.CenterVertically) {
                    // The asker's name in its own style, never run into Reins' words after it.
                    if (row.who.isNotEmpty()) Text(row.who + " ", style = TextStyle(color = InkMeta, fontSize = 13.sp, fontWeight = FontWeight.Medium), maxLines = 1)
                    Text(row.action, style = TextStyle(color = Ink, fontSize = 13.sp, fontWeight = FontWeight.Medium), maxLines = 1)
                }
                Spacer(GlanceModifier.width(8.dp))
                Text(row.time, style = TextStyle(color = InkMeta, fontSize = 11.sp), maxLines = 1)
                row.allowed?.let { ok ->
                    Spacer(GlanceModifier.width(6.dp))
                    if (ok) GlyphImage(Glyph.Check, RColors.Light.success, RColors.Dark.success, 14.dp)
                    else GlyphImage(Glyph.Close, RColors.Light.danger, RColors.Dark.danger, 14.dp)
                }
            }
        }
    }
}

class WaitingWidget : GlanceAppWidget() {
    override val sizeMode = SizeMode.Responsive(setOf(SMALL, WIDE))

    override suspend fun provideGlance(context: Context, id: GlanceId) {
        val container = (context.applicationContext as ReinsApp).container
        withContext(Dispatchers.IO) { runCatching { container.refreshPending() } }
        val model = waitingWidgetModel(container.state.pending.value)
        val open = openIntent(context, model.top)
        provideContent { WaitingWidgetContent(model, open) }
    }
}

class AutopilotWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        val container = (context.applicationContext as ReinsApp).container
        withContext(Dispatchers.IO) { runCatching { container.refreshAutopilot() } }
        val model = autopilotWidgetModel(container.state.autopilot.value, container.state.activity.value, System.currentTimeMillis() / 1000)
        val open = Intent(context, MainActivity::class.java).setAction(AppNotifier.ACTION_OPEN_AUTOPILOT)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        provideContent { AutopilotWidgetContent(model, open) }
    }
}

/** Lock down from the widget: no check, it only takes power away (the heavy thud, as in the app). */
class LockDownAction : ActionCallback {
    override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
        val container = (context.applicationContext as ReinsApp).container
        container.feedback.play(Event.LockdownOn)
        runCatching {
            container.core.setAutopilotMode(null, AutopilotMode.LOCKDOWN, null)
            container.refreshAutopilot()
        }
        Widgets.refresh(context)
    }
}

class WaitingWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = WaitingWidget()
}

class LatestWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        val container = (context.applicationContext as ReinsApp).container
        val model = latestWidgetModel(container.state.activity.value, System.currentTimeMillis() / 1000)
        val open = Intent(context, MainActivity::class.java).setAction(AppNotifier.ACTION_OPEN_ACTIVITY)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        provideContent { LatestWidgetContent(model, open) }
    }
}

class LatestWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = LatestWidget()
}

class AutopilotWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = AutopilotWidget()
}

object Widgets {
    /** Redraws the widgets after what waits or the mode changed. */
    suspend fun refresh(context: Context) {
        runCatching { WaitingWidget().updateAll(context) }
        runCatching { AutopilotWidget().updateAll(context) }
        runCatching { LatestWidget().updateAll(context) }
    }
}
