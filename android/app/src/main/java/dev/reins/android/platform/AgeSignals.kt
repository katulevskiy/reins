package dev.reins.android.platform

import android.content.Context
import com.google.android.play.agesignals.AgeSignalsManagerFactory
import com.google.android.play.agesignals.AgeSignalsRequest
import com.google.android.play.agesignals.AgeSignalsResult
import dev.reins.core.AgeSignalInput
import dev.reins.core.AgeSignalPlatform
import kotlin.coroutines.cancellation.CancellationException
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull

/**
 * What the phone's family features say about who uses it (Decisions v2 P7, 04 §4: a minor's mark is confirmed "through
 * the platform's family features where available"). Only ever a hint the ward's own acceptance goes with; Unavailable
 * never blocks anything.
 */
sealed class AgeSignal {
    /** 18 or over, as the platform says. The core refuses a minor's mark with it. */
    data object Adult : AgeSignal()

    /** Under 18; [upper] is the top of the range the platform gave (e.g. 15 for 13–15), when it gave one. */
    data class Minor(val upper: Int?) : AgeSignal()

    /** No answer: declined, not shared, an unsupported region, no Play Store, an app Play didn't install, an error. */
    data object Unavailable : AgeSignal()
}

/** Asks the platform for an age signal. Never throws: anything that goes wrong is [AgeSignal.Unavailable]. */
fun interface AgeSignals {
    suspend fun check(): AgeSignal
}

/** What the core is handed for the ward's acceptance. */
fun AgeSignal.toInput(): AgeSignalInput = when (this) {
    AgeSignal.Adult -> AgeSignalInput.Adult
    is AgeSignal.Minor -> AgeSignalInput.Minor(upper?.coerceIn(0, 255)?.toUByte(), AgeSignalPlatform.GOOGLE_PLAY)
    AgeSignal.Unavailable -> AgeSignalInput.Unavailable
}

/**
 * A Play Age Signals answer as a signal: an upper bound under 18 is a minor, a lower bound of 18 or more an adult;
 * no range (not shared, not verified) is no signal.
 */
fun ageSignalOf(lower: Int?, upper: Int?): AgeSignal = when {
    upper != null && upper < ADULT -> AgeSignal.Minor(upper)
    lower != null && lower >= ADULT -> AgeSignal.Adult
    else -> AgeSignal.Unavailable
}

private const val ADULT = 18

/**
 * Google Play Age Signals. It answers only where Play offers it and for an app Play installed (the `full` APK gets
 * "app not owned"), so most phones read as [AgeSignal.Unavailable]. It never asks for the user's consent here: what
 * was not shared is no signal.
 */
class PlayAgeSignals(private val context: Context, private val timeoutMs: Long = TIMEOUT_MS) : AgeSignals {
    override suspend fun check(): AgeSignal = try {
        withTimeoutOrNull(timeoutMs) { result()?.let { ageSignalOf(it.ageLower(), it.ageUpper()) } } ?: AgeSignal.Unavailable
    } catch (e: CancellationException) {
        throw e
    } catch (_: Throwable) {
        // A missing class (a stripped library), an AgeSignalsException, anything: no signal.
        AgeSignal.Unavailable
    }

    private suspend fun result(): AgeSignalsResult? = suspendCancellableCoroutine { continuation ->
        val task = AgeSignalsManagerFactory.create(context.applicationContext).checkAgeSignals(AgeSignalsRequest.builder().build())
        task.addOnCompleteListener { done ->
            if (continuation.isActive) continuation.resume(if (done.isSuccessful && !done.isCanceled) done.result else null)
        }
    }

    private companion object {
        /** Play binds to its service; a phone that never answers gives no signal, not a wait. */
        const val TIMEOUT_MS = 10_000L
    }
}

/** Lets tests answer without Play. */
object AgeSignalsProvider {
    @Volatile
    var factory: (Context) -> AgeSignals = { PlayAgeSignals(it) }
}
