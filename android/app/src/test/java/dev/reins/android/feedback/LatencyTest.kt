package dev.reins.android.feedback

import java.io.File
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.log10
import kotlin.math.sin
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/** The tap-to-audible path: cached system reads, the output rate, the assets' own leading silence, the keep-warm policy. */
class LatencyTest {
    private val raw = File("src/main/res/raw")

    // --- the gate must not query the system per event

    @Test
    fun `a TTL value reads once per window`() {
        var now = 0L
        var reads = 0
        val v = TtlValue(2_000, { now }) { ++reads }
        repeat(100) { assertEquals(1, v.get()) }
        assertEquals(1, reads)
        now = 1_999
        assertEquals(1, v.get())
        now = 2_000
        assertEquals(2, v.get())
        assertEquals(2, reads)
    }

    @Test
    fun `invalidating a TTL value forces a fresh read`() {
        var reads = 0
        val v = TtlValue(60_000, { 5L }) { ++reads }
        v.get()
        v.invalidate()
        assertEquals(2, v.get())
        assertEquals(2, v.get())
    }

    // --- output rate

    @Test
    fun `the output rate is parsed with a safe default`() {
        assertEquals(48_000, OutputPath.parseRate("48000"))
        assertEquals(44_100, OutputPath.parseRate(" 44100 "))
        assertEquals(OutputPath.DEFAULT_RATE, OutputPath.parseRate(null))
        assertEquals(OutputPath.DEFAULT_RATE, OutputPath.parseRate("fast"))
        assertEquals(OutputPath.DEFAULT_RATE, OutputPath.parseRate("0"))
        assertFalse(OutputPath.needsResampling(48_000))
        assertTrue(OutputPath.needsResampling(44_100))
        assertTrue(OutputPath.needsResampling(96_000))
    }

    // --- resampling for the rare non-48k mixer

    private fun sine(rate: Int, hz: Double, ms: Int): Wav {
        val n = rate * ms / 1000
        val fade = rate / 200
        return Wav(rate, 1, ShortArray(n) { i ->
            val edge = minOf(i, n - 1 - i).coerceAtMost(fade) / fade.toDouble()
            (12_000 * edge * sin(2 * PI * hz * i / rate)).toInt().toShort()
        })
    }

    private fun zeroCrossings(s: ShortArray) = (1 until s.size).count { s[it - 1] < 0 && s[it] >= 0 }

    @Test
    fun `a WAV round-trips`() {
        val w = sine(48_000, 880.0, 30)
        val parsed = Wav.parse(w.toBytes())
        assertNotNull(parsed)
        assertEquals(48_000, parsed!!.rate)
        assertEquals(1, parsed.channels)
        assertArrayEquals(w.samples, parsed.samples)
        assertNull(Wav.parse(ByteArray(10)))
        assertNull(Wav.parse("RIFF....WAVEjunk".toByteArray()))
    }

    @Test
    fun `resampling keeps pitch, length and clean ends`() {
        for (target in listOf(44_100, 96_000, 16_000)) {
            val src = sine(48_000, 1_000.0, 100)
            val out = Resampler.resample(src, target)
            assertEquals(target, out.rate)
            assertEquals(target / 10.0, out.samples.size.toDouble(), 2.0) // 100 ms
            assertEquals("rate $target", 100.0, zeroCrossings(out.samples).toDouble(), 2.0) // 1 kHz for 100 ms
            assertEquals(0, out.samples.first().toInt())
            assertEquals(0, out.samples.last().toInt())
            val peak = out.samples.maxOf { abs(it.toInt()) }
            assertTrue("peak $peak", peak in 11_000..12_600) // no resampling gain or clipping
        }
    }

    @Test
    fun `resampling to the same rate is the identity`() {
        val w = sine(48_000, 500.0, 20)
        assertSame(w, Resampler.resample(w, 48_000))
    }

    @Test
    fun `a shipped asset resamples end to end`() {
        val out = Resampler.resampleBytes(File(raw, "fx_tap.wav").readBytes(), 44_100)
        assertNotNull(out)
        val w = Wav.parse(out!!)!!
        assertEquals(44_100, w.rate)
        assertTrue(w.frames in 1_400..1_520) // 32 ms
    }

    // --- keep warm

    @Test
    fun `the output is kept warm while touched, then lapses`() {
        var now = 1_000L
        val warm = WarmPolicy({ now })
        assertFalse(warm.wanted())
        warm.touch()
        assertTrue(warm.wanted())
        now += WarmPolicy.IDLE_MS - 1
        assertTrue(warm.wanted())
        assertEquals(1, warm.remainingMs())
        warm.touch() // a new touch pushes the deadline out
        now += WarmPolicy.IDLE_MS - 1
        assertTrue(warm.wanted())
        now += 2
        assertFalse(warm.wanted())
        assertEquals(0, warm.remainingMs())
        warm.touch()
        warm.stop()
        assertFalse(warm.wanted())
    }

    // --- the assets themselves: leading silence is latency

    private fun shipped(): List<File> = raw.listFiles { f -> f.name.startsWith("fx_") && f.name.endsWith(".wav") }!!.sortedBy { it.name }

    @Test
    fun `every cue starts sounding within one millisecond`() {
        val files = shipped()
        assertTrue(files.size >= CueTable.resources.size)
        for (f in files) {
            val w = Wav.parse(f.readBytes())!!
            val peak = w.samples.maxOf { abs(it.toInt()) }
            val onset = w.samples.indexOfFirst { abs(it.toInt()) >= peak * 0.01 } // -40 dB re peak
            val ms = onset * 1000.0 / w.rate
            assertTrue("${f.name}: onset at ${"%.2f".format(ms)} ms", ms <= 1.0)
            assertEquals(f.name, 0, w.samples.first().toInt()) // and from exactly zero (no click)
        }
    }

    @Test
    fun `every cue is mono 48 kHz and never clips`() {
        for (f in shipped()) {
            val w = Wav.parse(f.readBytes())!!
            assertEquals(f.name, 48_000, w.rate)
            assertEquals(f.name, 1, w.channels)
            val peak = w.samples.maxOf { abs(it.toInt()) }
            val peakDb = 20 * log10(peak / 32768.0)
            assertTrue("${f.name} peak $peakDb dBFS", peakDb <= -0.4) // the mastering ceiling is -0.5 dBFS
        }
    }

    @Test
    fun `the keep-alive loop is silent and not a cue`() {
        val f = File(raw, "${SoundBank.KEEP_ALIVE}.wav")
        assertTrue(f.isFile)
        val w = Wav.parse(f.readBytes())!!
        assertEquals(48_000, w.rate)
        assertTrue(w.samples.all { it.toInt() == 0 })
        assertTrue(shipped().none { it.name.startsWith(SoundBank.KEEP_ALIVE) })
    }
}
