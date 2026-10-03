package dev.reins.android.autopilot

import dev.reins.core.ForeignException
import dev.reins.core.ModelInput
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** The model runtime without ONNX Runtime: the tensors it builds and reads, and how it holds its session. */
class ModelRuntimeTest {
    private fun input(batch: Int = 2, seqLen: Int = 4, k: Int = 3) = ModelInput(
        batch.toUInt(), seqLen.toUInt(), k.toUInt(),
        List(batch * seqLen) { it.toLong() },
        List(batch * seqLen) { 1L },
        List(batch * k) { (it % seqLen).toLong() },
        ByteArray(batch * k) { 1 },
        List(batch) { 0L },
    )

    /** Answers with tensors of the right shapes for the batch it gets, numbered so their order can be checked. */
    private class FakeSession(val hidden: Int = 5) : InferenceSession {
        val runs = CopyOnWriteArrayList<PackedInput>()
        var closed = false

        override fun run(input: PackedInput): RawOutput {
            runs += input
            val b = input.batch.toLong()
            return RawOutput(
                RawTensor(FloatArray(input.batch * input.k) { it.toFloat() }, longArrayOf(b, input.k.toLong())),
                RawTensor(FloatArray(input.batch * 2) { (-it).toFloat() }, longArrayOf(b, 2)),
                RawTensor(FloatArray(input.batch * hidden) { it / 10f }, longArrayOf(b, hidden.toLong())),
            )
        }

        override fun close() {
            closed = true
        }
    }

    private class FakeEngine : InferenceEngine {
        val opened = CopyOnWriteArrayList<String>()
        val sessions = CopyOnWriteArrayList<FakeSession>()
        var failure: Exception? = null

        override fun open(path: String): InferenceSession {
            failure?.let { throw it }
            opened += path
            return FakeSession().also { sessions += it }
        }
    }

    @Test
    fun `packing keeps the row-major layout and the shapes`() {
        val packed = ModelTensors.pack(input())
        assertArrayEquals(longArrayOf(2, 4), packed.tokenShape)
        assertArrayEquals(longArrayOf(2, 3), packed.markerShape)
        assertArrayEquals(longArrayOf(2), packed.batchShape)
        assertArrayEquals(LongArray(8) { it.toLong() }, packed.inputIds)
        assertEquals(6, packed.markerMask.size)
    }

    @Test
    fun `an array of the wrong size is refused, naming it`() {
        val bad = input().copy(markerPos = listOf(1L, 2L))
        try {
            ModelTensors.pack(bad)
            fail("packed a short marker_pos")
        } catch (e: IllegalArgumentException) {
            assertTrue(e.message!!.contains("marker_pos"))
        }
    }

    @Test
    fun `unpacking reads the hidden size from the pooled tensor`() {
        val packed = ModelTensors.pack(input())
        val out = ModelTensors.unpack(packed, FakeSession(hidden = 7).run(packed))
        assertEquals(7u, out.hidden)
        assertEquals(6, out.logits.size)
        assertEquals(4, out.act.size)
        assertEquals(14, out.pooled.size)
        assertEquals(listOf(0f, -1f, -2f, -3f), out.act)
    }

    @Test
    fun `outputs that do not fit the batch are refused`() {
        val packed = ModelTensors.pack(input())
        val raw = RawOutput(RawTensor(FloatArray(5), longArrayOf(1, 5)), RawTensor(FloatArray(4), longArrayOf(2, 2)), RawTensor(FloatArray(10), longArrayOf(2, 5)))
        try {
            ModelTensors.unpack(packed, raw)
            fail("unpacked short logits")
        } catch (e: IllegalArgumentException) {
            assertTrue(e.message!!.contains("logits"))
        }
    }

    @Test
    fun `the session is opened once and reused, and a new file replaces it`() {
        val engine = FakeEngine()
        val runtime = OnnxModelRuntime(engine)
        runtime.load("/models/a/model.onnx")
        runtime.load("/models/a/model.onnx")
        assertEquals(1, engine.opened.size)
        runtime.run(input())
        runtime.run(input())
        assertEquals(2, engine.sessions.single().runs.size)
        runtime.load("/models/b/model.onnx")
        assertTrue("the old session is freed", engine.sessions[0].closed)
        assertEquals("/models/b/model.onnx", runtime.loaded)
    }

    @Test
    fun `running without a model, or a model that fails to load, is a foreign error`() {
        val engine = FakeEngine()
        val runtime = OnnxModelRuntime(engine)
        try {
            runtime.run(input())
            fail("ran without a model")
        } catch (e: ForeignException.Failed) {
            assertTrue(e.reason.contains("no model"))
        }
        engine.failure = IllegalStateException("bad file")
        try {
            runtime.load("/x.onnx")
            fail("loaded a broken file")
        } catch (e: ForeignException.Failed) {
            assertTrue(e.reason.contains("bad file"))
        }
        assertNull(runtime.loaded)
    }

    @Test
    fun `a malformed batch fails as a foreign error, not a crash`() {
        val runtime = OnnxModelRuntime(FakeEngine())
        runtime.load("/a.onnx")
        try {
            runtime.run(input().copy(qtype = emptyList()))
            fail("ran a batch without qtype")
        } catch (e: ForeignException.Failed) {
            assertTrue(e.reason.contains("qtype"))
        }
    }

    @Test
    fun `unloading frees the session`() {
        val engine = FakeEngine()
        val runtime = OnnxModelRuntime(engine)
        runtime.load("/a.onnx")
        runtime.unload()
        assertTrue(engine.sessions.single().closed)
        assertNull(runtime.loaded)
    }

    @Test
    fun `runs from several threads share the session`() {
        val engine = FakeEngine()
        val runtime = OnnxModelRuntime(engine)
        runtime.load("/a.onnx")
        val pool = Executors.newFixedThreadPool(4)
        repeat(40) { pool.submit { runtime.run(input()) } }
        pool.shutdown()
        assertTrue(pool.awaitTermination(10, TimeUnit.SECONDS))
        assertEquals(40, engine.sessions.single().runs.size)
    }
}
