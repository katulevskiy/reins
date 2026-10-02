package dev.rewarden.android.autopilot

import ai.onnxruntime.OnnxJavaType
import ai.onnxruntime.OnnxTensor
import ai.onnxruntime.OrtEnvironment
import ai.onnxruntime.OrtSession
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.LongBuffer

/**
 * ONNX Runtime for Android. Everything runs on this phone's CPU: all graph optimisations, up to four threads, one
 * session per model file. ONNX Runtime's own telemetry is switched off here and its start-up provider is removed in
 * the manifest, so nothing about the model or its use leaves the phone.
 */
object OrtEngine : InferenceEngine {
    private const val MAX_THREADS = 4

    private val env: OrtEnvironment by lazy {
        OrtEnvironment.getEnvironment().also { env ->
            try {
                env.setTelemetry(false)
            } catch (_: Exception) {
                // Not every platform has the switch; Android sends nothing once the provider is gone.
            }
        }
    }

    override fun open(path: String): InferenceSession {
        val options = OrtSession.SessionOptions().apply {
            setOptimizationLevel(OrtSession.SessionOptions.OptLevel.ALL_OPT)
            setIntraOpNumThreads(Runtime.getRuntime().availableProcessors().coerceIn(1, MAX_THREADS))
            setExecutionMode(OrtSession.SessionOptions.ExecutionMode.SEQUENTIAL)
        }
        return options.use { OrtInference(env, env.createSession(path, it)) }
    }
}

private class OrtInference(private val env: OrtEnvironment, private val session: OrtSession) : InferenceSession {
    /** A model exported without an input (an older export without `qtype`, say) simply does not get it. */
    private val inputs = session.inputNames

    override fun run(input: PackedInput): RawOutput {
        val tensors = LinkedHashMap<String, OnnxTensor>()
        try {
            fun add(name: String, make: () -> OnnxTensor) {
                if (name in inputs) tensors[name] = make()
            }
            add(ModelTensors.INPUT_IDS) { OnnxTensor.createTensor(env, LongBuffer.wrap(input.inputIds), input.tokenShape) }
            add(ModelTensors.ATTENTION_MASK) { OnnxTensor.createTensor(env, LongBuffer.wrap(input.attentionMask), input.tokenShape) }
            add(ModelTensors.MARKER_POS) { OnnxTensor.createTensor(env, LongBuffer.wrap(input.markerPos), input.markerShape) }
            add(ModelTensors.MARKER_MASK) {
                val bytes = ByteBuffer.allocateDirect(input.markerMask.size).order(ByteOrder.nativeOrder())
                bytes.put(input.markerMask).rewind()
                OnnxTensor.createTensor(env, bytes, input.markerShape, OnnxJavaType.BOOL)
            }
            add(ModelTensors.QTYPE) { OnnxTensor.createTensor(env, LongBuffer.wrap(input.qtype), input.batchShape) }
            return session.run(tensors, setOf(ModelTensors.LOGITS, ModelTensors.ACT, ModelTensors.POOLED)).use { result ->
                fun read(name: String): RawTensor {
                    val tensor = result.get(name).orElseThrow { IllegalStateException("the model has no output $name") } as OnnxTensor
                    val buffer = tensor.floatBuffer
                    val values = FloatArray(buffer.remaining()).also { buffer.get(it) }
                    return RawTensor(values, tensor.info.shape)
                }
                RawOutput(read(ModelTensors.LOGITS), read(ModelTensors.ACT), read(ModelTensors.POOLED))
            }
        } finally {
            tensors.values.forEach { it.close() }
        }
    }

    override fun close() = session.close()
}
