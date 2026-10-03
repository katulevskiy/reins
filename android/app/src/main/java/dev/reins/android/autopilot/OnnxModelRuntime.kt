package dev.rewarden.android.autopilot

import dev.rewarden.core.ForeignException
import dev.rewarden.core.ModelInput
import dev.rewarden.core.ModelOutput
import dev.rewarden.core.ModelRuntime
import java.util.concurrent.locks.ReentrantReadWriteLock
import kotlin.concurrent.read
import kotlin.concurrent.write

/** Opens a model file. The seam between [OnnxModelRuntime] and ONNX Runtime, so tests never load native code. */
fun interface InferenceEngine {
    fun open(path: String): InferenceSession
}

/** A loaded model. [run] may be called from several threads at once. */
interface InferenceSession : AutoCloseable {
    fun run(input: PackedInput): RawOutput
}

/**
 * Autopilot's forward pass (spec §6.3): the core tokenizes, calibrates and decides; this only runs the network. The
 * session is opened once and reused; runs share it, loading or unloading waits for them to finish.
 */
class OnnxModelRuntime(private val engine: InferenceEngine = OrtEngine) : ModelRuntime {
    private val lock = ReentrantReadWriteLock()
    private var session: InferenceSession? = null
    private var loadedPath: String? = null

    /** The file currently loaded, if any. */
    val loaded: String? get() = lock.read { loadedPath }

    override fun load(path: String) {
        lock.write {
            if (path == loadedPath && session != null) return
            closeSession()
            session = failing("could not load the model") { engine.open(path) }
            loadedPath = path
        }
    }

    override fun run(input: ModelInput): ModelOutput = lock.read {
        val current = session ?: throw ForeignException.Failed("no model is loaded")
        failing("the model could not run") {
            val packed = ModelTensors.pack(input)
            ModelTensors.unpack(packed, current.run(packed))
        }
    }

    override fun unload() {
        lock.write { closeSession() }
    }

    private fun closeSession() {
        try {
            session?.close()
        } catch (_: Exception) {
            // Freed either way.
        }
        session = null
        loadedPath = null
    }

    /** Every failure crosses to the core as a [ForeignException], which leaves the request to the user. */
    private inline fun <T> failing(what: String, block: () -> T): T = try {
        block()
    } catch (e: ForeignException) {
        throw e
    } catch (e: Throwable) {
        throw ForeignException.Failed("$what: ${e.message ?: e.javaClass.simpleName}")
    }
}
