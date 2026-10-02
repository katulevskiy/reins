package dev.rewarden.android.platform

import com.google.android.gms.tasks.CancellationTokenSource
import com.google.android.gms.tasks.Task
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine

/** Suspends until the Play services [Task] completes (the library `kotlinx-coroutines-play-services` is not a dependency). */
suspend fun <T> Task<T>.await(): T = suspendCancellableCoroutine { continuation ->
    addOnCompleteListener { task ->
        val error = task.exception
        when {
            error != null -> continuation.resumeWithException(error)
            task.isCanceled -> continuation.cancel()
            else -> continuation.resume(task.result)
        }
    }
}
