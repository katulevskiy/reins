package dev.reins.android.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import dev.reins.android.design.pressable

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Slider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.ListRow
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SelectChip
import dev.reins.android.design.SwitchRow
import dev.reins.android.feedback.AndroidFeedback
import dev.reins.android.feedback.CueCategory
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.FeedbackSettings
import dev.reins.android.feedback.HapticStrength
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Settings > Sounds & haptics: one master above everything (notifications included), the sounds by kind with a
 * volume, haptics with a strength, and a list that plays the real thing. The switches here are the only ones Reins
 * listens to: the phone's touch-sound and touch-vibration settings are not consulted.
 */
@Composable
fun SoundsScreen(engine: AndroidFeedback, onBack: () -> Unit) {
    val s by engine.store.settings.collectAsStateWithLifecycle()
    val update = engine.store::update

    Screen(title = "Sounds and haptics", onBack = onBack) {
        Group(
            Modifier.padding(top = 10.dp),
            footer = "Off silences every sound and vibration in Reins, its notifications included.",
        ) {
            SwitchRow("Sounds & haptics", s.master, { on -> update { copy(master = on) } }, glyph = Glyph.Speaker, tag = "soundsMaster")
        }

        Group(
            header = "Sounds",
            footer = "While Reins is open. In the background the same chimes come with the notification, at your phone's " +
                "notification volume. Sound when open: a new request chimes at that volume while Reins is on screen too; " +
                "silent, vibrate and Do Not Disturb hold it back.",
        ) {
            SwitchRow("Sounds", s.sounds, { on -> update { copy(sounds = on) } }, glyph = Glyph.Speaker, enabled = s.master, tag = "sounds")
            Hairline(inset = 51.dp)
            SwitchRow(
                "Interface",
                s.interfaceSounds,
                { on -> update { copy(interfaceSounds = on) } },
                glyph = Glyph.Apps,
                enabled = s.soundsOn,
                tag = "interfaceSounds",
            )
            Hairline(inset = 51.dp)
            SwitchRow(
                "Requests",
                s.requestSounds,
                { on -> update { copy(requestSounds = on) } },
                glyph = Glyph.Bell,
                enabled = s.soundsOn,
                tag = "requestSounds",
            )
            Hairline(inset = 51.dp)
            SwitchRow(
                "Sound when open",
                s.soundWhenOpen,
                { on -> update { copy(soundWhenOpen = on) } },
                glyph = Glyph.Phone,
                enabled = s.allows(CueCategory.Requests),
                tag = "soundWhenOpen",
            )
            Hairline(inset = 51.dp)
            SwitchRow(
                "Alerts",
                s.alertSounds,
                { on -> update { copy(alertSounds = on) } },
                glyph = Glyph.Warning,
                enabled = s.soundsOn,
                tag = "alertSounds",
            )
            Hairline(inset = 51.dp)
            SwitchRow(
                "Autopilot",
                s.autopilotSounds,
                { on -> update { copy(autopilotSounds = on) } },
                glyph = Glyph.Sparkle,
                enabled = s.soundsOn,
                tag = "autopilotSounds",
            )
            Hairline(inset = 51.dp)
            VolumeRow(engine, s)
        }

        Group(header = "Haptics") {
            SwitchRow("Haptics", s.haptics, { on -> update { copy(haptics = on) } }, glyph = Glyph.Vibrate, enabled = s.master, tag = "haptics")
            Hairline(inset = 51.dp)
            StrengthRow(s) { level -> update { copy(strength = level) } }
        }

        Group(header = "Feel them", footer = "Each plays its sound and haptic, even when that kind of sound is off.") {
            PreviewChips(engine, s)
        }
        Spacer(Modifier.height(32.dp))
    }
}

private const val VOLUME_STEPS = 10

/** The level: a ten-step slider whose detents climb the scale as you drag, heard at the level they set. */
@Composable
private fun VolumeRow(engine: AndroidFeedback, s: FeedbackSettings) {
    val c = LocalColors.current
    var last by remember { mutableIntStateOf(Math.round(s.volume * VOLUME_STEPS)) }
    val enabled = s.soundsOn
    Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            GlyphIcon(Glyph.Speaker, c.secondary, size = 21.dp)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                RText("Volume", RType.sans(16f, FontWeight.Medium), if (enabled) c.text else c.tertiary)
                RText("Media volume", RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp))
            }
            Spacer(Modifier.width(10.dp))
            RText("${Math.round(s.volume * 100)}%", RType.mono(14f, FontWeight.Medium), c.secondary, Modifier.testTag("volumeValue"))
        }
        Slider(
            value = s.volume,
            onValueChange = { v ->
                engine.store.update { copy(volume = v) }
                val step = Math.round(v * VOLUME_STEPS)
                if (step != last) {
                    last = step
                    engine.preview(Event.Detent.haptic, Event.Detent.cue, step)
                }
            },
            onValueChangeFinished = { engine.preview(null, Event.Selection.cue) },
            steps = VOLUME_STEPS - 1,
            enabled = enabled,
            modifier = Modifier.testTag("volume"),
        )
    }
}

@Composable
private fun StrengthRow(s: FeedbackSettings, onChange: (HapticStrength) -> Unit) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
        // Subtle · Standard · Strong in the chooser's well; the choice is felt at the strength it picks.
        dev.reins.android.design.Chooser(
            HapticStrength.entries.map { dev.reins.android.design.Choice(it, it.label, "strength:${it.name}") },
            s.strength,
            onChange,
        )
    }
}

/** A moment to audition: its title, when it plays, and the event (or a little sequence of them). */
private class Sample(val title: String, val supporting: String, val events: List<Event>, val gapMs: Long = 0, val steps: List<Int>? = null)

private val samples = listOf(
    Sample("Request", "Something waits for your approval", listOf(Event.RequestArrived)),
    Sample("Approve", "Approved once", listOf(Event.Approved)),
    Sample("Deny", "Denied", listOf(Event.Denied)),
    Sample("Allow for a while", "A permission that stays", listOf(Event.GrantCreated)),
    Sample("Connected", "An AI, a server or an account", listOf(Event.Connected)),
    Sample("File approved", "An upload can be downloaded", listOf(Event.UploadApproved)),
    Sample("Revoke", "A grant or connection removed", listOf(Event.Revoked)),
    Sample("Switch", "On, then off", listOf(Event.ToggleOn, Event.ToggleOff), gapMs = 450),
    Sample("Slider", "Detents climbing the scale", listOf(Event.Detent), gapMs = 110, steps = listOf(0, 1, 2, 3, 4)),
    Sample("Error", "Something went wrong", listOf(Event.Error)),
    Sample("Alert", "A grant ends soon", listOf(Event.Alert)),
    Sample("Autopilot approved", "Approved for you, quietly", listOf(Event.AutoApproved)),
    Sample("Autopilot denied", "Denied for you", listOf(Event.AutoDenied)),
    Sample("More autonomy", "From Manual to Auto", listOf(Event.AutopilotOn)),
    Sample("Less autonomy", "Back to Manual", listOf(Event.AutopilotOff)),
    Sample("Bypass", "On, then off", listOf(Event.BypassOn, Event.BypassOff), gapMs = 900),
    Sample("Lockdown", "Everything denied", listOf(Event.LockdownOn)),
)

/**
 * The moments to play with, as chips two to a row: each shows its haptic as a little score (a bar per beat, taller
 * for a stronger one) and plays it with its sound when tapped.
 */
@Composable
private fun PreviewChips(engine: AndroidFeedback, s: FeedbackSettings) {
    val c = LocalColors.current
    val scope = rememberCoroutineScope()
    var running by remember { mutableStateOf(false) }
    val enabled = s.soundsOn || s.hapticsOn
    Column(Modifier.padding(horizontal = 10.dp, vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        samples.chunked(2).forEach { pair ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                pair.forEach { sample ->
                    val shape = androidx.compose.foundation.shape.RoundedCornerShape(14.dp)
                    Row(
                        Modifier
                            .weight(1f)
                            .height(44.dp)
                            .clip(shape)
                            .background(c.paperRaised)
                            .border(1.dp, c.rule, shape)
                            .pressable(enabled = enabled, shape = shape, label = sample.title) {
                                if (!running) {
                                    running = true
                                    scope.launch {
                                        for (event in sample.events) for (step in sample.steps ?: listOf(0)) {
                                            engine.preview(event.haptic, event.cue, step)
                                            delay(sample.gapMs)
                                        }
                                        running = false
                                    }
                                }
                            }
                            .semantics { contentDescription = "${sample.title}: ${sample.supporting}" }
                            .testTag("try:${sample.title}")
                            .padding(horizontal = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Score(sample, if (enabled) c.ink else c.inkFaint)
                        RText(sample.title, RType.sans(14f, FontWeight.Medium), if (enabled) c.ink else c.inkMeta, Modifier.weight(1f), maxLines = 1)
                    }
                }
                if (pair.size == 1) Spacer(Modifier.weight(1f))
            }
        }
    }
}

/** A haptic as a little score: a bar per beat, the later ones of a climb taller. */
@Composable
private fun Score(sample: Sample, color: androidx.compose.ui.graphics.Color) {
    val beats = (sample.events.size * (sample.steps?.size ?: 1)).coerceIn(1, 6)
    Row(Modifier.height(16.dp).width(28.dp), horizontalArrangement = Arrangement.spacedBy(2.dp), verticalAlignment = Alignment.Bottom) {
        repeat(beats) { i ->
            val h = if (sample.steps != null) 5f + 11f * (i + 1) / beats else if (i == 0) 16f else 10f
            androidx.compose.foundation.layout.Box(Modifier.width(3.dp).height(h.dp).clip(androidx.compose.foundation.shape.RoundedCornerShape(1.dp)).background(color))
        }
    }
}
