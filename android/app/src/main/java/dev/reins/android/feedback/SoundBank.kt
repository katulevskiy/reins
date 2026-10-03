package dev.reins.android.feedback

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.SoundPool
import android.util.Log
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger

/**
 * Every cue preloaded into one [SoundPool]: no decoding on press, no audio focus (the user's music keeps playing).
 *
 * Built for the shortest tap-to-audible path:
 *  - the attributes ask for the low-latency output, which needs the asset rate to equal the device's output rate; when
 *    it does not, the assets are resampled once into the cache ([OutputPath], [Resampler]) so the mixer never has to;
 *  - every stream's track is created ahead of the first cue ([prime]), and a silent looped stream keeps the output out
 *    of standby while the user is touching the app ([keepWarm]);
 *  - [play] is one synchronous call, with no thread hop. Callers issue the sound before the haptic: the vibrator is a
 *    binder call, and the sound is the channel a person hears as late.
 */
class SoundBank(private val context: Context) {
    private val pool: SoundPool = SoundPool.Builder()
        .setMaxStreams(FeedbackGate.MAX_STREAMS + 1) // one more than the gate allows: the keep-alive loop owns it
        .setAudioAttributes(attributes())
        .build()

    /** Sample id per resource name: cues that share a file share the sample. */
    private val ids = ConcurrentHashMap<String, Int>()
    private val ready = ConcurrentHashMap.newKeySet<Int>()
    private val pending = AtomicInteger()

    @Volatile private var keepAliveStream = 0

    /** The device mixer's rate (`PROPERTY_OUTPUT_SAMPLE_RATE`); the assets are 48 kHz. */
    val outputRate: Int = OutputPath.parseRate(
        context.getSystemService(AudioManager::class.java)?.getProperty(AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE),
    )

    init {
        pool.setOnLoadCompleteListener { _, sampleId, status ->
            if (status == 0) ready.add(sampleId) else Log.w(AndroidFeedback.TAG, "sound $sampleId failed to load: $status")
            if (pending.decrementAndGet() == 0) prime()
        }
    }

    /** Starts decoding every cue; returns at once (SoundPool loads asynchronously). Call off the main thread. */
    fun load() {
        val dir = if (OutputPath.needsResampling(outputRate)) resampledDir() else null
        val names = CueTable.resources + KEEP_ALIVE
        pending.set(names.size)
        for (name in names) {
            val id = loadOne(name, dir)
            if (id == 0) {
                Log.w(AndroidFeedback.TAG, "no resource $name")
                if (pending.decrementAndGet() == 0) prime()
            } else {
                ids[name] = id
            }
        }
    }

    private fun loadOne(name: String, dir: File?): Int {
        val res = context.resources.getIdentifier(name, "raw", context.packageName)
        if (res == 0) return 0
        if (dir != null) {
            val file = File(dir, "$name.wav")
            if (!file.isFile || file.length() == 0L) {
                val resampled = runCatching {
                    context.resources.openRawResource(res).use { Resampler.resampleBytes(it.readBytes(), outputRate) }
                }.getOrNull()
                if (resampled != null) file.writeBytes(resampled)
            }
            if (file.isFile && file.length() > 0L) return pool.load(file.path, 1)
        }
        return pool.load(context, res, 1)
    }

    /** The cache of assets at [outputRate]; older rates and app versions are dropped. */
    private fun resampledDir(): File {
        val updated = runCatching { context.packageManager.getPackageInfo(context.packageName, 0).lastUpdateTime }.getOrDefault(0L)
        val name = "sounds-$outputRate-$updated"
        context.cacheDir.listFiles { f -> f.isDirectory && f.name.startsWith("sounds-") && f.name != name }?.forEach { it.deleteRecursively() }
        return File(context.cacheDir, name).also { it.mkdirs() }
    }

    /**
     * Once everything is decoded: create each stream's AudioTrack now, silently (a track is created at a stream's first
     * play, which costs a few ms the first cue would otherwise pay). Overlapping plays of the shortest cue reach
     * different streams.
     */
    private fun prime() {
        val id = ids[CueTable.spec(Cue.Tap).resource]?.takeIf { it in ready } ?: return
        repeat(FeedbackGate.MAX_STREAMS) { pool.play(id, PRIME_VOLUME, PRIME_VOLUME, 0, 0, 1f) }
    }

    /** [rate] resamples (pitch); [volume] is 0..1. Returns whether a stream started. */
    fun play(cue: Cue, volume: Float, rate: Float, priority: Int): Boolean {
        val id = ids[CueTable.spec(cue).resource]?.takeIf { it in ready } ?: return false
        return pool.play(id, volume, volume, priority, 0, rate) != 0
    }

    /**
     * Starts or stops the silent looped stream that keeps the output awake. Returns whether it is running after the
     * call (false before the silence has loaded: the caller tries again on the next touch).
     */
    @Synchronized
    fun keepWarm(on: Boolean): Boolean {
        if (!on) {
            if (keepAliveStream != 0) pool.stop(keepAliveStream)
            keepAliveStream = 0
            return false
        }
        if (keepAliveStream != 0) return true
        val id = ids[KEEP_ALIVE]?.takeIf { it in ready } ?: return false
        keepAliveStream = pool.play(id, 1f, 1f, 0, -1, 1f)
        return keepAliveStream != 0
    }

    companion object {
        /** `res/raw/silence_keepalive.wav`: 100 ms of digital silence, looped (not an `fx_` file: it is not a cue). */
        const val KEEP_ALIVE = "silence_keepalive"

        /** Inaudible (-60 dB) but not zero, so no layer treats the priming plays as muted and skips the track. */
        const val PRIME_VOLUME = 0.001f

        // USAGE_GAME, not USAGE_ASSISTANCE_SONIFICATION: the sonification usage is the system-sound path, which some
        // screen recorders (Samsung's) silence while they record. Game sound effects follow the media volume, are not
        // muted by a capture, and are part of a recording's media sounds. No audio focus is requested.
        // FLAG_LOW_LATENCY is deprecated in favour of AudioTrack performance modes, which SoundPool cannot take; the
        // audio policy still reads it and routes the stream to the fast output.
        @Suppress("DEPRECATION")
        private fun attributes(): AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_GAME)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .setFlags(AudioAttributes.FLAG_LOW_LATENCY)
            .setAllowedCapturePolicy(AudioAttributes.ALLOW_CAPTURE_BY_ALL)
            .build()
    }
}
