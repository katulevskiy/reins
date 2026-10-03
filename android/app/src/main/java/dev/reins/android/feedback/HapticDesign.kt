package dev.reins.android.feedback

import android.os.VibrationEffect
import android.os.VibrationEffect.Composition as C
import android.view.HapticFeedbackConstants as H

/** One primitive in a composition: [scale] 0..1 is the Standard strength, [delayMs] precedes it. */
data class Step(val primitive: Int, val scale: Float, val delayMs: Int = 0)

/** One segment of the waveform fallback: vibrate at [amplitude] (0 = pause) for [ms]. */
data class Segment(val ms: Long, val amplitude: Int)

/**
 * How a [Haptic] is felt. The engine walks a ladder from the richest effect the device can render down to a plain
 * pulse:
 *
 * 1. The platform's own `performHapticFeedback` constant (OEM tuned, so it feels native) when [preferView] and the
 *    strength is Standard.
 * 2. A `VibrationEffect.Composition` of primitives, scaled by strength.
 * 3. A view constant for patterns that are not [preferView] (Standard only).
 * 4. The designed [waveform] (amplitude control), then the predefined effect, then the waveform without amplitudes.
 */
data class HapticSpec(
    val haptic: Haptic,
    val preferView: Boolean,
    /** SDK level -> `HapticFeedbackConstants` value, or null when this device has none for the moment. */
    val view: (Int) -> Int?,
    /** Alternatives tried in order; the first whose primitives are all supported wins. */
    val compositions: List<List<Step>>,
    val predefined: Int,
    val waveform: List<Segment>,
    /** Prefer [waveform] over [predefined] when amplitude control exists (the compound patterns). */
    val designedWaveform: Boolean = false,
    /** 0 light (detent-class), 1 action, 2 event, 3 alert. */
    val priority: Int,
    /** The same haptic never repeats inside this many ms. */
    val minGapMs: Long,
)

object HapticTable {
    private fun always(constant: Int): (Int) -> Int? = { constant }
    private fun api(level: Int, constant: Int, below: Int? = null): (Int) -> Int? = { sdk -> if (sdk >= level) constant else below }

    fun spec(haptic: Haptic): HapticSpec = specs[haptic.ordinal]

    private val specs: List<HapticSpec> by lazy { Haptic.entries.map(::build) }

    /** The rising ticks of [Haptic.Surge]: scale and gap per step, ending in the crack. */
    internal val surgeRamp: List<Pair<Float, Int>> = listOf(
        0.20f to 0, 0.30f to 60, 0.42f to 50, 0.56f to 40, 0.70f to 32,
    )

    private fun surge(finish: Step, closer: Step?): List<Step> = buildList {
        for ((scale, gap) in surgeRamp) add(Step(C.PRIMITIVE_TICK, scale, gap))
        add(finish)
        if (closer != null) add(closer)
    }

    private fun build(haptic: Haptic): HapticSpec = when (haptic) {
        // The faintest detent: slider steps. API 34 has a dedicated frequent tick.
        Haptic.Tick -> HapticSpec(
            haptic, true, api(34, H.SEGMENT_FREQUENT_TICK, H.CLOCK_TICK),
            listOf(listOf(Step(C.PRIMITIVE_LOW_TICK, 0.5f)), listOf(Step(C.PRIMITIVE_TICK, 0.3f))),
            VibrationEffect.EFFECT_TICK, listOf(Segment(10, 40)), priority = 0, minGapMs = 70,
        )
        // A choice was made: crisper than Tick.
        Haptic.Select -> HapticSpec(
            haptic, true, api(34, H.SEGMENT_TICK, H.CONTEXT_CLICK),
            listOf(listOf(Step(C.PRIMITIVE_TICK, 0.7f)), listOf(Step(C.PRIMITIVE_CLICK, 0.35f))),
            VibrationEffect.EFFECT_TICK, listOf(Segment(12, 90)), priority = 0, minGapMs = 50,
        )
        // Switches: a rising pair for on, a falling pair for off (the system toggle constants from API 34).
        Haptic.ToggleOn -> HapticSpec(
            haptic, true, api(34, H.TOGGLE_ON),
            listOf(
                listOf(Step(C.PRIMITIVE_LOW_TICK, 0.5f), Step(C.PRIMITIVE_TICK, 0.8f, 20)),
                listOf(Step(C.PRIMITIVE_TICK, 0.5f), Step(C.PRIMITIVE_CLICK, 0.6f, 25)),
            ),
            VibrationEffect.EFFECT_CLICK, listOf(Segment(8, 60), Segment(20, 0), Segment(12, 130)), priority = 1, minGapMs = 80,
        )
        Haptic.ToggleOff -> HapticSpec(
            haptic, true, api(34, H.TOGGLE_OFF),
            listOf(
                listOf(Step(C.PRIMITIVE_TICK, 0.7f), Step(C.PRIMITIVE_LOW_TICK, 0.4f, 20)),
                listOf(Step(C.PRIMITIVE_CLICK, 0.5f), Step(C.PRIMITIVE_TICK, 0.4f, 25)),
            ),
            VibrationEffect.EFFECT_TICK, listOf(Segment(12, 110), Segment(20, 0), Segment(8, 50)), priority = 1, minGapMs = 80,
        )
        // A committed action: approve, save, refresh.
        Haptic.Confirm -> HapticSpec(
            haptic, true, always(H.CONFIRM),
            listOf(listOf(Step(C.PRIMITIVE_CLICK, 0.65f)), listOf(Step(C.PRIMITIVE_TICK, 0.9f))),
            VibrationEffect.EFFECT_CLICK, listOf(Segment(20, 170)), priority = 1, minGapMs = 120,
        )
        // Soft rise, then a small settle: a permission given, an AI connected.
        Haptic.Success -> HapticSpec(
            haptic, false, { null },
            listOf(
                listOf(Step(C.PRIMITIVE_QUICK_RISE, 0.45f), Step(C.PRIMITIVE_LOW_TICK, 0.55f, 40)),
                listOf(Step(C.PRIMITIVE_CLICK, 0.5f), Step(C.PRIMITIVE_TICK, 0.5f, 60)),
            ),
            VibrationEffect.EFFECT_CLICK, listOf(Segment(24, 70), Segment(24, 130), Segment(30, 0), Segment(18, 110)),
            designedWaveform = true, priority = 2, minGapMs = 400,
        )
        // Two crisp taps: something needs you.
        Haptic.Attention -> HapticSpec(
            haptic, false, { null },
            listOf(
                listOf(Step(C.PRIMITIVE_TICK, 0.85f), Step(C.PRIMITIVE_TICK, 0.85f, 90)),
                listOf(Step(C.PRIMITIVE_CLICK, 0.6f), Step(C.PRIMITIVE_CLICK, 0.6f, 90)),
            ),
            VibrationEffect.EFFECT_DOUBLE_CLICK, listOf(Segment(18, 200), Segment(70, 0), Segment(18, 200)),
            designedWaveform = true, priority = 2, minGapMs = 400,
        )
        // A short, heavy double: failed.
        Haptic.Error -> HapticSpec(
            haptic, false, always(H.REJECT),
            listOf(
                listOf(Step(C.PRIMITIVE_CLICK, 1f), Step(C.PRIMITIVE_THUD, 0.8f, 60)),
                listOf(Step(C.PRIMITIVE_CLICK, 1f), Step(C.PRIMITIVE_CLICK, 0.9f, 70)),
            ),
            VibrationEffect.EFFECT_HEAVY_CLICK, listOf(Segment(35, 255), Segment(55, 0), Segment(45, 230)),
            designedWaveform = true, priority = 3, minGapMs = 400,
        )
        // Revoke, delete, disconnect: a low thud with a click on top.
        Haptic.Heavy -> HapticSpec(
            haptic, false, { null },
            listOf(
                listOf(Step(C.PRIMITIVE_THUD, 0.85f), Step(C.PRIMITIVE_CLICK, 0.5f, 30)),
                listOf(Step(C.PRIMITIVE_CLICK, 1f)),
            ),
            VibrationEffect.EFFECT_HEAVY_CLICK, listOf(Segment(40, 255)), priority = 2, minGapMs = 300,
        )
        // A small pop that decays: Autopilot answered for you.
        Haptic.Pop -> HapticSpec(
            haptic, false, { null },
            listOf(
                listOf(Step(C.PRIMITIVE_TICK, 0.8f), Step(C.PRIMITIVE_LOW_TICK, 0.35f, 25)),
                listOf(Step(C.PRIMITIVE_TICK, 0.8f)),
            ),
            VibrationEffect.EFFECT_TICK, listOf(Segment(12, 120), Segment(14, 0), Segment(8, 50)), priority = 1, minGapMs = 120,
        )
        // A refusal: firmer than Confirm, and falling where Confirm is one click. Not an error (REJECT is only the
        // fallback): saying no is a normal answer.
        Haptic.Deny -> HapticSpec(
            haptic, false, always(H.REJECT),
            listOf(
                listOf(Step(C.PRIMITIVE_CLICK, 0.85f), Step(C.PRIMITIVE_LOW_TICK, 0.55f, 40)),
                listOf(Step(C.PRIMITIVE_CLICK, 0.85f), Step(C.PRIMITIVE_TICK, 0.4f, 40)),
            ),
            VibrationEffect.EFFECT_CLICK, listOf(Segment(18, 210), Segment(30, 0), Segment(12, 80)),
            designedWaveform = true, priority = 1, minGapMs = 200,
        )
        // Bypass on: five ticks that firm up and bunch together, then a hard crack with a thud under it, about
        // 250 ms. Without THUD the crack is a double click; without TICK it is clicks only.
        Haptic.Surge -> HapticSpec(
            haptic, false, { null },
            listOf(
                surge(Step(C.PRIMITIVE_CLICK, 0.9f, 24), Step(C.PRIMITIVE_THUD, 1f, 8)),
                surge(Step(C.PRIMITIVE_CLICK, 0.85f, 24), Step(C.PRIMITIVE_CLICK, 1f, 14)),
                listOf(
                    Step(C.PRIMITIVE_CLICK, 0.25f), Step(C.PRIMITIVE_CLICK, 0.35f, 60), Step(C.PRIMITIVE_CLICK, 0.5f, 50),
                    Step(C.PRIMITIVE_CLICK, 0.7f, 40), Step(C.PRIMITIVE_CLICK, 1f, 30),
                ),
            ),
            VibrationEffect.EFFECT_HEAVY_CLICK,
            listOf(
                Segment(28, 40), Segment(18, 0), Segment(28, 70), Segment(14, 0), Segment(28, 110), Segment(12, 0),
                Segment(28, 160), Segment(10, 0), Segment(28, 210), Segment(8, 0), Segment(40, 255),
            ),
            designedWaveform = true, priority = 2, minGapMs = 500,
        )
        // Bypass off: a very short, sharp double tick.
        Haptic.Zip -> HapticSpec(
            haptic, false, { null },
            listOf(
                listOf(Step(C.PRIMITIVE_TICK, 0.9f), Step(C.PRIMITIVE_TICK, 0.9f, 28)),
                listOf(Step(C.PRIMITIVE_CLICK, 0.5f), Step(C.PRIMITIVE_CLICK, 0.5f, 28)),
            ),
            VibrationEffect.EFFECT_TICK,
            listOf(Segment(6, 200), Segment(18, 0), Segment(6, 200)),
            designedWaveform = true, priority = 1, minGapMs = 100,
        )
        // Autopilot up: an irregular crackle of micro-pulses (uneven strength and gaps) ending in a firm crack, about
        // 180 ms.
        Haptic.Lightning -> HapticSpec(
            haptic, false, { null },
            listOf(
                listOf(
                    Step(C.PRIMITIVE_TICK, 0.45f), Step(C.PRIMITIVE_TICK, 0.80f, 31), Step(C.PRIMITIVE_LOW_TICK, 0.35f, 17),
                    Step(C.PRIMITIVE_TICK, 0.90f, 44), Step(C.PRIMITIVE_CLICK, 1f, 52),
                ),
                listOf(
                    Step(C.PRIMITIVE_CLICK, 0.40f), Step(C.PRIMITIVE_CLICK, 0.70f, 36), Step(C.PRIMITIVE_CLICK, 0.35f, 20),
                    Step(C.PRIMITIVE_CLICK, 0.85f, 44), Step(C.PRIMITIVE_CLICK, 1f, 52),
                ),
            ),
            VibrationEffect.EFFECT_DOUBLE_CLICK,
            listOf(
                Segment(7, 110), Segment(22, 0), Segment(5, 220), Segment(13, 0), Segment(6, 70), Segment(34, 0),
                Segment(8, 255), Segment(40, 0), Segment(24, 255),
            ),
            designedWaveform = true, priority = 2, minGapMs = 400,
        )
    }

    val all: List<HapticSpec> get() = specs
}

/** What the device can do, probed once at start. */
data class HapticCapabilities(
    val sdk: Int,
    /** Primitive ids this vibrator renders (`areAllPrimitivesSupported`). */
    val primitives: Set<Int>,
    val amplitudeControl: Boolean,
    /** A window view is attached for `performHapticFeedback`. */
    val hasView: Boolean,
)

/** The effect chosen for one haptic on one device. */
sealed interface HapticPlan {
    data class ViewConstant(val constant: Int) : HapticPlan
    data class Composition(val steps: List<Step>) : HapticPlan
    data class Predefined(val effect: Int) : HapticPlan
    data class Waveform(val timings: LongArray, val amplitudes: IntArray?) : HapticPlan {
        override fun equals(other: Any?) = other is Waveform && timings.contentEquals(other.timings) && amplitudes.contentEquals(other.amplitudes)
        override fun hashCode() = timings.contentHashCode() * 31 + amplitudes.contentHashCode()
    }

    /** Nothing to play, with why (kept for the debug log). */
    data class Skip(val reason: String) : HapticPlan
}

object HapticPlanner {
    /** Primitive scale at [strength], never imperceptible and never above full. */
    fun scaled(scale: Float, strength: HapticStrength): Float = (scale * strength.scale).coerceIn(0.05f, 1f)

    fun plan(haptic: Haptic, strength: HapticStrength, caps: HapticCapabilities): HapticPlan {
        val spec = HapticTable.spec(haptic)
        val standard = strength == HapticStrength.Standard
        val view = if (caps.hasView && standard) spec.view(caps.sdk) else null

        if (spec.preferView && view != null) return HapticPlan.ViewConstant(view)

        spec.compositions.firstOrNull { steps -> steps.all { it.primitive in caps.primitives } }?.let { steps ->
            return HapticPlan.Composition(steps.map { it.copy(scale = scaled(it.scale, strength)) })
        }
        if (view != null) return HapticPlan.ViewConstant(view)

        // Hardware without primitives. Without amplitude control a Subtle request can only be honoured by dropping the
        // lightest haptics: playing them at full strength would be the opposite.
        if (!caps.amplitudeControl && strength == HapticStrength.Subtle && spec.priority == 0) {
            return HapticPlan.Skip("subtle: no amplitude control")
        }
        if (caps.amplitudeControl && (spec.designedWaveform || !standard)) return waveform(spec, strength, true)
        if (standard || !caps.amplitudeControl) return HapticPlan.Predefined(spec.predefined)
        return waveform(spec, strength, caps.amplitudeControl)
    }

    fun waveform(spec: HapticSpec, strength: HapticStrength, amplitudes: Boolean): HapticPlan.Waveform {
        if (!amplitudes) return HapticPlan.Waveform(onOffTimings(spec.waveform), null)
        val timings = LongArray(spec.waveform.size) { spec.waveform[it].ms }
        val amps = IntArray(spec.waveform.size) {
            val a = spec.waveform[it].amplitude
            if (a == 0) 0 else (a * strength.scale).toInt().coerceIn(1, 255)
        }
        return HapticPlan.Waveform(timings, amps)
    }

    /** `createWaveform(timings, repeat)` alternates off, on, off, ... starting with off. */
    fun onOffTimings(segments: List<Segment>): LongArray {
        val out = ArrayList<Long>()
        var lastOn = false // the implicit leading slot is "off"
        out.add(0L)
        for (segment in segments) {
            val isOn = segment.amplitude > 0
            if (isOn == lastOn) out[out.lastIndex] = out.last() + segment.ms else out.add(segment.ms)
            lastOn = isOn
        }
        return out.toLongArray()
    }
}
