package dev.reins.android.platform.widgets

import android.content.Context
import android.content.Intent
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
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
import androidx.glance.text.FontFamily
import androidx.glance.text.FontStyle
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import dev.reins.android.MainActivity
import dev.reins.android.R
import dev.reins.android.ReinsApp
import dev.reins.android.design.RColors
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AppNotifier
import dev.reins.core.AutopilotMode
import dev.reins.core.PendingKind
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

// The home screen widgets (And-Outside 04), with Glance. Their content is these composables over plain models, so a
// test can render it; the widgets only read the app's state and open Reins. Nothing is ever approved from a widget.

private val Paper = ColorProvider(RColors.Light.paperRaised, RColors.Dark.paperRaised)
private val Ink = ColorProvider(RColors.Light.ink, RColors.Dark.ink)
private val InkMeta = ColorProvider(RColors.Light.inkMeta, RColors.Dark.inkMeta)
private val Violet = ColorProvider(RColors.Light.violet, RColors.Dark.violet)
private val Sunken = ColorProvider(RColors.Light.paperSunken, RColors.Dark.paperSunken)
private val OnField = ColorProvider(Color.White, Color.White)
private val OnField2 = ColorProvider(Color.White.copy(alpha = 0.82f), Color.White.copy(alpha = 0.82f))
private val OnFieldWash = ColorProvider(Color.White.copy(alpha = 0.2f), Color.White.copy(alpha = 0.2f))

private fun label(color: androidx.glance.unit.ColorProvider) = TextStyle(color = color, fontSize = 11.sp, fontWeight = FontWeight.Medium, fontFamily = FontFamily.Monospace)

/** Opens Reins on [model]'s top card (its request, pairing, file or phone), or on the stack. */
fun openIntent(context: Context, top: WaitingWidgetModel.TopCard?): Intent {
    val intent = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    if (top == null) return intent
    return intent.setAction(AppNotifier.ACTION_OPEN_ITEM)
        .putExtra(
            AppNotifier.EXTRA_KIND,
            when (top.kind) {
                PendingKind.REQUEST -> AppNotifier.KIND_REQUEST
                PendingKind.PAIRING -> AppNotifier.KIND_PAIRING
                PendingKind.BLOB -> AppNotifier.KIND_BLOB
                PendingKind.JOIN -> AppNotifier.KIND_JOIN
            },
        )
        .putExtra(AppNotifier.EXTRA_ID, top.id)
}

/**
 * Waiting: the top card of the stack on its field (the grain pre-baked), "Waiting · 2", the ask in the serif, who asks,
 * and Review; the whole widget opens it. With nothing waiting, paper and the pond: "Nothing waits."
 */
@Composable
fun WaitingWidgetContent(model: WaitingWidgetModel, open: Intent) {
    val top = model.top
    if (top == null) {
        Column(
            GlanceModifier.fillMaxSize().cornerRadius(24.dp).background(Paper).padding(16.dp).clickable(actionStartActivity(open)),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Image(ImageProvider(R.drawable.signal_pond), contentDescription = null, modifier = GlanceModifier.fillMaxWidth().height(70.dp), contentScale = ContentScale.Fit)
            Spacer(GlanceModifier.height(8.dp))
            Text("Nothing waits.", style = TextStyle(color = Ink, fontSize = 17.sp, fontFamily = FontFamily.Serif, fontStyle = FontStyle.Italic))
        }
        return
    }
    Box(
        GlanceModifier.fillMaxSize().cornerRadius(24.dp).background(ImageProvider(GrainBitmap.of(top.field.fill().toArgb())), ContentScale.FillBounds)
            .clickable(actionStartActivity(open))
            .semantics { contentDescription = "${top.ask}. Opens the request." },
    ) {
        Column(GlanceModifier.fillMaxSize().padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                // The request's icon (the integration with the action on its corner), ringed in the field.
                val context = androidx.glance.LocalContext.current
                val px = (26 * context.resources.displayMetrics.density).toInt().coerceAtLeast(1)
                val icon = runCatching { dev.reins.android.ui.common.IntegrationBitmap.draw(context, top.provider, top.action, px, top.field.fill().toArgb()) }.getOrNull()
                if (icon != null) {
                    Image(ImageProvider(icon), contentDescription = null, modifier = GlanceModifier.size(26.dp))
                    Spacer(GlanceModifier.width(8.dp))
                }
                Text(waitingLabel(model).uppercase(), style = label(OnField2), maxLines = 1)
            }
            Spacer(GlanceModifier.height(8.dp))
            Text(top.ask, style = TextStyle(color = OnField, fontSize = 21.sp, fontFamily = FontFamily.Serif), maxLines = 3)
            top.details?.let {
                Spacer(GlanceModifier.height(3.dp))
                Text(it, style = TextStyle(color = OnField, fontSize = 12.sp, fontFamily = FontFamily.Monospace), maxLines = 2)
            }
            Spacer(GlanceModifier.height(4.dp))
            Text(top.who, style = TextStyle(color = OnField2, fontSize = 12.sp), maxLines = 1)
            Spacer(GlanceModifier.defaultWeight())
            // Review, never Approve: the request opens, and the answer is given there.
            Box(
                GlanceModifier.fillMaxWidth().height(36.dp).cornerRadius(18.dp).background(OnField).clickable(actionStartActivity(open)),
                contentAlignment = Alignment.Center,
            ) { Text("Review", style = TextStyle(color = ColorProvider(Color(0xFF16151B), Color(0xFF16151B)), fontSize = 14.sp, fontWeight = FontWeight.Medium)) }
        }
    }
}

/**
 * Autopilot: the mode in the serif (violet for Auto), how much it decided today, and Lock down, which only takes power
 * away; the rest opens Trust.
 */
@Composable
fun AutopilotWidgetContent(model: AutopilotWidgetModel, open: Intent) {
    Column(GlanceModifier.fillMaxSize().cornerRadius(24.dp).background(Paper).padding(16.dp).clickable(actionStartActivity(open))) {
        Text("AUTOPILOT", style = label(InkMeta), maxLines = 1)
        Spacer(GlanceModifier.height(6.dp))
        Text(
            model.name,
            style = TextStyle(color = if (model.mode == AutopilotMode.AUTO) Violet else Ink, fontSize = 30.sp, fontFamily = FontFamily.Serif),
            maxLines = 1,
        )
        Text(model.line, style = TextStyle(color = InkMeta, fontSize = 12.sp), maxLines = 1)
        Spacer(GlanceModifier.defaultWeight())
        Box(
            GlanceModifier.fillMaxWidth().height(34.dp).cornerRadius(17.dp).background(Sunken)
                .clickable(if (model.offersLockDown) actionRunCallback<LockDownAction>() else actionStartActivity(open)),
            contentAlignment = Alignment.Center,
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(if (model.offersLockDown) "Lock down" else "Locked down", style = TextStyle(color = Ink, fontSize = 13.sp, fontWeight = FontWeight.Medium))
            }
        }
    }
}

class WaitingWidget : GlanceAppWidget() {
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

class AutopilotWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = AutopilotWidget()
}

object Widgets {
    /** Redraws the widgets after what waits or the mode changed. */
    suspend fun refresh(context: Context) {
        runCatching { WaitingWidget().updateAll(context) }
        runCatching { AutopilotWidget().updateAll(context) }
    }
}
