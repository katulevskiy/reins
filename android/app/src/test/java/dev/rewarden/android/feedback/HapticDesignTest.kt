package dev.rewarden.android.feedback

import android.os.VibrationEffect.Composition as C
import android.view.HapticFeedbackConstants as H
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HapticDesignTest {
    private val allPrimitives = setOf(C.PRIMITIVE_CLICK, C.PRIMITIVE_TICK, C.PRIMITIVE_LOW_TICK, C.PRIMITIVE_THUD, C.PRIMITIVE_SPIN, C.PRIMITIVE_QUICK_RISE, C.PRIMITIVE_SLOW_RISE, C.PRIMITIVE_QUICK_FALL)

    private fun caps(sdk: Int = 36, primitives: Set<Int> = allPrimitives, amplitude: Boolean = true, view: Boolean = true) =
        HapticCapabilities(sdk, primitives, amplitude, view)

    private val devices = listOf(
        caps(), caps(sdk = 31, primitives = emptySet()), caps(sdk = 31, primitives = emptySet(), amplitude = false, view = false),
        caps(sdk = 31, primitives = setOf(C.PRIMITIVE_CLICK, C.PRIMITIVE_TICK)), caps(sdk = 34, view = false),
        caps(sdk = 33, primitives = setOf(C.PRIMITIVE_CLICK)),
    )

    private fun steps(h: Haptic, c: HapticCapabilities = caps(view = false), strength: HapticStrength = HapticStrength.Standard) =
        (HapticPlanner.plan(h, strength, c) as HapticPlan.Composition).steps

    private fun wave(h: Haptic, strength: HapticStrength = HapticStrength.Standard) = HapticPlanner.waveform(HapticTable.spec(h), strength, true)

    @Test
    fun `every haptic has a spec`() {
        assertEquals(Haptic.entries.size, HapticTable.all.size)
        for (h in Haptic.entries) {
            val s = HapticTable.spec(h)
            assertEquals(h, s.haptic)
            assertTrue("$h has compositions", s.compositions.isNotEmpty())
            assertTrue("$h has a waveform", s.waveform.isNotEmpty() && s.waveform.any { it.amplitude > 0 })
            assertTrue("$h gap", s.minGapMs > 0)
        }
    }

    @Test
    fun `every haptic plans on every device at every strength`() {
        for (h in Haptic.entries) for (strength in HapticStrength.entries) for (d in devices) {
            val plan = HapticPlanner.plan(h, strength, d)
            if (plan is HapticPlan.Skip) {
                assertTrue("only the lightest haptics may be dropped: $h $strength $d", strength == HapticStrength.Subtle && !d.amplitudeControl && HapticTable.spec(h).priority == 0)
            }
            if (plan is HapticPlan.Composition) assertTrue(plan.steps.all { it.primitive in d.primitives && it.scale in 0.05f..1f })
            if (plan is HapticPlan.Waveform) {
                assertEquals(plan.timings.size, plan.amplitudes?.size ?: plan.timings.size)
                assertTrue(plan.amplitudes?.all { it in 0..255 } ?: true)
            }
        }
    }

    @Test
    fun `Standard prefers the platform constant where it fits`() {
        assertEquals(HapticPlan.ViewConstant(H.SEGMENT_FREQUENT_TICK), HapticPlanner.plan(Haptic.Tick, HapticStrength.Standard, caps(sdk = 34)))
        assertEquals(HapticPlan.ViewConstant(H.CLOCK_TICK), HapticPlanner.plan(Haptic.Tick, HapticStrength.Standard, caps(sdk = 31)))
        assertEquals(HapticPlan.ViewConstant(H.TOGGLE_ON), HapticPlanner.plan(Haptic.ToggleOn, HapticStrength.Standard, caps(sdk = 34)))
        assertEquals(HapticPlan.ViewConstant(H.TOGGLE_OFF), HapticPlanner.plan(Haptic.ToggleOff, HapticStrength.Standard, caps(sdk = 34)))
        assertEquals(HapticPlan.ViewConstant(H.CONFIRM), HapticPlanner.plan(Haptic.Confirm, HapticStrength.Standard, caps(sdk = 31)))
    }

    @Test
    fun `toggle on rises and toggle off falls`() {
        for (d in listOf(caps(sdk = 31), caps(sdk = 31, primitives = emptySet()))) {
            assertFalse(HapticPlanner.plan(Haptic.ToggleOn, HapticStrength.Standard, d) == HapticPlanner.plan(Haptic.ToggleOff, HapticStrength.Standard, d))
        }
        val on = HapticTable.spec(Haptic.ToggleOn).compositions.first()
        val off = HapticTable.spec(Haptic.ToggleOff).compositions.first()
        assertTrue(on.last().scale > on.first().scale)
        assertTrue(off.last().scale < off.first().scale)
    }

    @Test
    fun `compound patterns are designed, not platform constants`() {
        for (h in listOf(Haptic.Success, Haptic.Attention, Haptic.Heavy, Haptic.Deny)) {
            val plan = HapticPlanner.plan(h, HapticStrength.Standard, caps())
            assertTrue("$h $plan", plan is HapticPlan.Composition && plan.steps.size >= 2)
        }
        // Error and Deny go through the composition too; REJECT is only their fallback.
        assertTrue(HapticPlanner.plan(Haptic.Error, HapticStrength.Standard, caps()) is HapticPlan.Composition)
        assertEquals(HapticPlan.ViewConstant(H.REJECT), HapticPlanner.plan(Haptic.Error, HapticStrength.Standard, caps(primitives = emptySet())))
        assertEquals(HapticPlan.ViewConstant(H.REJECT), HapticPlanner.plan(Haptic.Deny, HapticStrength.Standard, caps(primitives = emptySet())))
    }

    @Test
    fun `deny is a firm click that falls away, firmer than approve and lighter than an error`() {
        val deny = steps(Haptic.Deny)
        assertEquals(C.PRIMITIVE_CLICK, deny.first().primitive)
        assertTrue(deny.last().scale < deny.first().scale) // falls
        assertTrue(deny.first().scale > steps(Haptic.Confirm).first().scale)
        assertTrue(deny.sumOf { it.scale.toDouble() } < steps(Haptic.Error).sumOf { it.scale.toDouble() })
        assertTrue(HapticTable.spec(Haptic.Deny).priority < HapticTable.spec(Haptic.Error).priority)
        val amps = wave(Haptic.Deny).amplitudes!!.filter { it > 0 }
        assertEquals(amps.sortedDescending(), amps)
    }

    @Test
    fun `strength scales primitives`() {
        fun first(s: HapticStrength) = steps(Haptic.Select, caps(sdk = 34, view = false), s).first().scale
        val subtle = first(HapticStrength.Subtle)
        val standard = first(HapticStrength.Standard)
        val strong = first(HapticStrength.Strong)
        assertTrue("$subtle < $standard < $strong", subtle < standard && standard < strong)
        assertEquals(0.7f * 0.5f, subtle, 1e-6f)
        assertEquals(1f, HapticPlanner.scaled(0.9f, HapticStrength.Strong), 0f)
        assertEquals(0.05f, HapticPlanner.scaled(0.01f, HapticStrength.Subtle), 0f)
    }

    @Test
    fun `Subtle and Strong never use the fixed-strength platform constant`() {
        assertTrue(HapticPlanner.plan(Haptic.Tick, HapticStrength.Subtle, caps(sdk = 34)) is HapticPlan.Composition)
        assertTrue(HapticPlanner.plan(Haptic.Tick, HapticStrength.Strong, caps(sdk = 34)) is HapticPlan.Composition)
    }

    @Test
    fun `the waveform fallback scales its amplitude`() {
        val none = caps(sdk = 31, primitives = emptySet(), view = false)
        val strong = HapticPlanner.plan(Haptic.Confirm, HapticStrength.Strong, none) as HapticPlan.Waveform
        val spec = HapticTable.spec(Haptic.Confirm)
        assertEquals((spec.waveform.first().amplitude * 1.5f).toInt(), strong.amplitudes!!.first())
        val subtle = HapticPlanner.plan(Haptic.Confirm, HapticStrength.Subtle, none) as HapticPlan.Waveform
        assertTrue(subtle.amplitudes!!.first() < spec.waveform.first().amplitude)
    }

    @Test
    fun `without amplitude control it plays predefined effects and drops light ones when Subtle`() {
        val basic = caps(sdk = 31, primitives = emptySet(), amplitude = false, view = false)
        assertTrue(HapticPlanner.plan(Haptic.Confirm, HapticStrength.Standard, basic) is HapticPlan.Predefined)
        assertTrue(HapticPlanner.plan(Haptic.Tick, HapticStrength.Subtle, basic) is HapticPlan.Skip)
        assertNotNull(HapticPlanner.plan(Haptic.Error, HapticStrength.Subtle, basic))
    }

    @Test
    fun `designed haptics play as waveforms without primitives`() {
        val bare = caps(sdk = 31, primitives = emptySet(), view = false)
        for (h in listOf(Haptic.Success, Haptic.Deny, Haptic.Surge, Haptic.Zip, Haptic.Lightning)) {
            assertEquals("$h", wave(h), HapticPlanner.plan(h, HapticStrength.Standard, bare))
        }
    }

    @Test
    fun `on-off timings alternate starting off`() {
        val t = HapticPlanner.onOffTimings(listOf(Segment(10, 100), Segment(20, 0), Segment(5, 50)))
        assertEquals(listOf(0L, 10L, 20L, 5L), t.toList())
        val merged = HapticPlanner.onOffTimings(listOf(Segment(10, 100), Segment(5, 200)))
        assertEquals(listOf(0L, 15L), merged.toList())
    }

    @Test
    fun `priorities order light, action and alert`() {
        assertTrue(HapticTable.spec(Haptic.Tick).priority < HapticTable.spec(Haptic.Confirm).priority)
        assertTrue(HapticTable.spec(Haptic.Confirm).priority < HapticTable.spec(Haptic.Error).priority)
    }

    // --- the Autopilot haptics

    private fun duration(steps: List<Step>) = steps.sumOf { it.delayMs }

    @Test
    fun `surge is a quarter-second crescendo ending in a hard crack`() {
        val s = steps(Haptic.Surge)
        val ramp = s.dropLast(2)
        assertTrue(ramp.size >= 5 && ramp.all { it.primitive == C.PRIMITIVE_TICK })
        assertEquals(ramp.map { it.scale }.sorted(), ramp.map { it.scale }) // firms up
        val gaps = ramp.drop(1).map { it.delayMs }
        assertEquals(gaps.sortedDescending(), gaps) // and bunches together
        assertEquals(C.PRIMITIVE_THUD, s.last().primitive)
        assertEquals(1f, s.last().scale, 0f)
        val total = duration(s) + s.size * 10 // each primitive plays about ten milliseconds
        assertTrue("about 250 ms, was $total", total in 200..320)
        val clicksOnly = steps(Haptic.Surge, caps(sdk = 31, primitives = setOf(C.PRIMITIVE_CLICK), view = false))
        assertTrue(clicksOnly.all { it.primitive == C.PRIMITIVE_CLICK })
        assertEquals(clicksOnly.map { it.scale }.sorted(), clicksOnly.map { it.scale })
        val w = wave(Haptic.Surge)
        val on = w.timings.indices.filter { w.amplitudes!![it] > 0 }.map { w.amplitudes!![it] }
        assertEquals(on.sorted(), on)
    }

    @Test
    fun `zip is a very short sharp double tick`() {
        val s = steps(Haptic.Zip)
        assertEquals(2, s.size)
        assertTrue(s.all { it.primitive == C.PRIMITIVE_TICK && it.scale >= 0.85f })
        assertTrue(wave(Haptic.Zip).timings.sum() <= 40)
    }

    @Test
    fun `lightning is an irregular crackle with a final crack`() {
        val s = steps(Haptic.Lightning)
        val pulses = s.dropLast(1)
        assertTrue(pulses.size in 3..5)
        assertEquals(C.PRIMITIVE_CLICK, s.last().primitive)
        assertEquals(1f, s.last().scale, 0f)
        assertTrue(pulses.map { it.scale }.toSet().size == pulses.size) // uneven strength
        assertTrue(s.drop(1).map { it.delayMs }.toSet().size == s.size - 1) // and spacing
        assertTrue((duration(s) + s.size * 10) in 150..230)
    }

    @Test
    fun `an automatic answer is felt more lightly than the user's own`() {
        assertTrue(steps(Haptic.Pop).first().scale <= 0.8f)
        assertTrue(steps(Haptic.Pop).sumOf { it.scale.toDouble() } < steps(Haptic.Deny).sumOf { it.scale.toDouble() })
        assertTrue(HapticTable.spec(Event.AutoDenied.haptic!!).priority == 0)
    }
}
