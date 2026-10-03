package dev.reins.android.feedback

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import android.os.VibrationAttributes
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log
import android.view.View
import androidx.core.content.ContextCompat
import java.lang.ref.WeakReference

/** Lets tests watch what the app asks for, before any switch or rate limit. Production leaves it null. */
object FeedbackProvider {
    @Volatile
    var observer: Feedback? = null
}

/**
 * The real [FeedbackEnvironment]: whether the activity is started, plus whether the screen is interactive. That one is
 * a binder call, so it is cached for [TTL_MS] and dropped at once by the screen on / off broadcasts (registered only
 * while in front).
 */
class SystemEnvironment(private val context: Context, vibrator: Vibrator?, clock: () -> Long = SystemClock::uptimeMillis) : FeedbackEnvironment {
    private val power = context.getSystemService(PowerManager::class.java)

    @Volatile
    var foreground = false

    private val interactive = TtlValue(TTL_MS, clock) { power?.isInteractive != false }
    private val motor: Boolean by lazy { vibrator?.hasVibrator() == true }

    override val active: Boolean get() = foreground && interactive.get()
    override val hasVibrator: Boolean get() = motor

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) = interactive.invalidate()
    }
    private var watching = false

    /** Keeps the screen state exact while the app is in front; costs nothing in the background. */
    fun watch(on: Boolean) {
        if (on == watching) return
        watching = on
        if (on) {
            val filter = IntentFilter().apply {
                addAction(Intent.ACTION_SCREEN_ON)
                addAction(Intent.ACTION_SCREEN_OFF)
            }
            ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
            interactive.invalidate()
        } else {
            runCatching { context.unregisterReceiver(receiver) }
        }
    }

    companion object {
        /** Safety net behind the broadcasts: nothing stays stale longer than this. */
        const val TTL_MS = 2_000L
    }
}

/**
 * Haptics and sound for the whole app. [FeedbackGate] decides when something plays, [HapticPlanner] how a haptic is
 * felt on this hardware, [CueTable] which sound and how loud, and [Event] what each of Reins's moments is made of.
 * Nothing plays while the app is in the background: the events that matter there arrive as notifications, whose
 * channels sound the same chimes (`AppNotifier`).
 *
 * Debug builds log one line per request to the `ReinsFeedback` tag: what played (and as what) or why it did not.
 */
class AndroidFeedback(
    context: Context,
    val store: FeedbackStore,
    private val clock: () -> Long = SystemClock::uptimeMillis,
) : Feedback, TapFeedback {
    private val context = context.applicationContext
    private val debug = (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
    private val vibrator: Vibrator? = context.getSystemService(VibratorManager::class.java)?.defaultVibrator
    private val env = SystemEnvironment(this.context, vibrator)
    private val gate = FeedbackGate({ store.current }, env, clock)
    private val claims = ClaimTracker(clock)
    private val bank = SoundBank(this.context)
    private val main = Handler(Looper.getMainLooper())
    private val warm = WarmPolicy(clock)
    private var warming = false
    private val warmCheck = object : Runnable {
        override fun run() {
            if (warm.wanted()) {
                main.postDelayed(this, warm.remainingMs() + 50)
            } else {
                bank.keepWarm(false)
                warming = false
            }
        }
    }
    private var viewRef = WeakReference<View?>(null)

    private val primitives: Set<Int> by lazy {
        val v = vibrator ?: return@lazy emptySet()
        ALL_PRIMITIVES.filter { v.areAllPrimitivesSupported(it) }.toSet()
    }
    private val amplitudeControl: Boolean by lazy { vibrator?.hasAmplitudeControl() == true }

    init {
        // Resource lookups and decoding stay off the main thread; SoundPool.load itself returns at once.
        Thread({ bank.load() }, "feedback-load").start()
        log("ready: vibrator=${vibrator?.hasVibrator()} outputRate=${bank.outputRate} resampled=${OutputPath.needsResampling(bank.outputRate)}")
    }

    /** The window's view, for `performHapticFeedback` (OEM-tuned effects). */
    fun attachView(view: View?) {
        viewRef = WeakReference(view)
    }

    /** The app is in front (its activity is started). Nothing plays otherwise. */
    fun setForeground(foreground: Boolean) {
        env.foreground = foreground
        env.watch(foreground)
        if (!foreground) {
            warm.stop()
            main.removeCallbacks(warmCheck)
            bank.keepWarm(false)
            warming = false
        }
        log("foreground=$foreground")
    }

    /**
     * A finger went down somewhere in the app (UI thread, `ACTION_DOWN`). Wakes the sound output now so it is running
     * by the time the click that sounds arrives; see [WarmPolicy].
     */
    fun onTouchDown() {
        if (!env.foreground || !store.current.soundsOn) return
        warm.touch()
        if (!warming) {
            warming = bank.keepWarm(true)
            if (warming) {
                main.removeCallbacks(warmCheck)
                main.postDelayed(warmCheck, WarmPolicy.IDLE_MS + 50)
            }
        }
    }

    // ── Feedback ───────────────────────────────────────────────────────────

    override fun haptic(haptic: Haptic) {
        FeedbackProvider.observer?.haptic(haptic)
        claims.claimHaptic()
        play(haptic)
    }

    override fun cue(cue: Cue, step: Int) {
        FeedbackProvider.observer?.cue(cue, step)
        claims.claimCue()
        play(cue, step)
    }

    override fun defaultTap() {
        val released = clock()
        main.postDelayed({
            claims.quiet() // a press just answered: a dialog closing around it stays silent
            // The sound first: the vibrator service is a binder call, and the sound is the part perceived late.
            if (claims.cueClaimed(released)) log("cue Tap skip default tap: claimed") else play(Cue.Tap, 0)
            if (claims.hapticClaimed(released)) log("haptic Select skip default tap: claimed") else play(Haptic.Select)
        }, ClaimTracker.DEFER_MS)
    }

    override fun quietClose() = claims.quiet()

    override fun cueUnlessRecent(cue: Cue, windowMs: Long) {
        if (claims.cueWithin(windowMs)) log("cue $cue skip: another cue just played") else cue(cue)
    }

    /** The Settings page auditioning a moment: ignores rate limits and category switches, nothing else. */
    fun preview(haptic: Haptic?, cue: Cue?, step: Int = 0) {
        cue?.let { claims.claimCue(); play(it, step, preview = true) }
        haptic?.let { claims.claimHaptic(); play(it, preview = true) }
    }

    /** How richly this vibration motor renders haptics, for the Settings page. */
    fun hapticTier(): String = when {
        vibrator?.hasVibrator() != true -> "This phone has no vibration motor"
        VibrationEffect.Composition.PRIMITIVE_CLICK in primitives && VibrationEffect.Composition.PRIMITIVE_TICK in primitives -> "Rich haptics: precise clicks and ticks"
        amplitudeControl -> "Standard haptics: adjustable vibration"
        else -> "Basic haptics: a simple on/off motor"
    }

    // ── haptics ────────────────────────────────────────────────────────────

    private fun play(haptic: Haptic, preview: Boolean = false) {
        when (val d = gate.haptic(haptic, preview)) {
            is Decision.Skip -> log("haptic $haptic skip: ${d.why.label}")
            Decision.Play -> {
                val view = viewRef.get()?.takeIf { it.isAttachedToWindow }
                val strength = store.current.strength
                var plan = HapticPlanner.plan(haptic, strength, caps(view != null))
                var ok = perform(plan, view)
                if (!ok && plan is HapticPlan.ViewConstant) {
                    plan = HapticPlanner.plan(haptic, strength, caps(false))
                    ok = perform(plan, null)
                }
                log("haptic $haptic ${if (ok) "play" else "skip"} $plan")
            }
        }
    }

    private fun caps(hasView: Boolean) = HapticCapabilities(android.os.Build.VERSION.SDK_INT, primitives, amplitudeControl, hasView)

    private fun perform(plan: HapticPlan, view: View?): Boolean = try {
        when (plan) {
            is HapticPlan.Skip -> false
            is HapticPlan.ViewConstant -> view?.performHapticFeedback(plan.constant) == true
            is HapticPlan.Composition -> vibrate(
                VibrationEffect.startComposition().also { c ->
                    plan.steps.forEach { c.addPrimitive(it.primitive, it.scale, it.delayMs) }
                }.compose(),
            )
            is HapticPlan.Predefined -> vibrate(VibrationEffect.createPredefined(plan.effect))
            is HapticPlan.Waveform -> vibrate(
                if (plan.amplitudes != null) VibrationEffect.createWaveform(plan.timings, plan.amplitudes, -1)
                else VibrationEffect.createWaveform(plan.timings, -1),
            )
        }
    } catch (e: RuntimeException) {
        Log.w(TAG, "haptic failed", e)
        false
    }

    private fun vibrate(effect: VibrationEffect): Boolean {
        val v = vibrator ?: return false
        if (android.os.Build.VERSION.SDK_INT >= 33) {
            v.vibrate(effect, VibrationAttributes.createForUsage(VibrationAttributes.USAGE_TOUCH))
        } else {
            @Suppress("DEPRECATION")
            v.vibrate(effect, android.media.AudioAttributes.Builder().setUsage(android.media.AudioAttributes.USAGE_ASSISTANCE_SONIFICATION).build())
        }
        return true
    }

    // ── sound ──────────────────────────────────────────────────────────────

    private fun play(cue: Cue, step: Int, preview: Boolean = false) {
        when (val d = gate.cue(cue, preview)) {
            is Decision.Skip -> log("cue $cue skip: ${d.why.label}")
            Decision.Play -> {
                val spec = CueTable.spec(cue)
                val volume = CueTable.volume(spec, store.current.gain)
                val rate = if (cue == Cue.Detent) DetentLadder.rate(step) else 1f
                val started = bank.play(cue, volume, rate, spec.priority)
                log(
                    if (started) "cue $cue play vol=${"%.2f".format(volume)} rate=${"%.2f".format(rate)}${if (cue == Cue.Detent) " step=$step" else ""}"
                    else "cue $cue skip: ${Skipped.NotLoaded.label}",
                )
            }
        }
    }

    private fun log(message: String) {
        if (debug) Log.d(TAG, message)
    }

    companion object {
        const val TAG = "ReinsFeedback"
        private val ALL_PRIMITIVES = intArrayOf(
            VibrationEffect.Composition.PRIMITIVE_CLICK,
            VibrationEffect.Composition.PRIMITIVE_TICK,
            VibrationEffect.Composition.PRIMITIVE_LOW_TICK,
            VibrationEffect.Composition.PRIMITIVE_THUD,
            VibrationEffect.Composition.PRIMITIVE_SPIN,
            VibrationEffect.Composition.PRIMITIVE_QUICK_RISE,
            VibrationEffect.Composition.PRIMITIVE_SLOW_RISE,
            VibrationEffect.Composition.PRIMITIVE_QUICK_FALL,
        )
    }
}
