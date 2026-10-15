package dev.reins.android.platform

import dev.reins.core.OwnerCheck
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The core's owner check (starting Overnight, a new length, what it does with what is too risky), answered with the
 * phone's own prompt: biometrics or the device credential, as for a bypass. [title] says what; the core's reason is
 * the subtitle. Only a pass now confirms: a cancel, no screen lock, or an error refuses, and the core changes nothing.
 * [last] keeps how it went, so the screen can tell a cancel (nothing to say) from a phone without a screen lock.
 */
class OwnerPrompt(
    private val authenticator: Authenticator,
    private val title: String,
    /** The prompt needs the main thread; the core asks from its own. */
    private val on: CoroutineContext = Dispatchers.Main.immediate,
) : OwnerCheck {
    @Volatile
    var last: AuthResult? = null
        private set

    override suspend fun confirm(reason: String): Boolean {
        val result = try {
            withContext(on) { authenticator.authenticate(title, reason) }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            AuthResult.Cancelled
        }
        last = result
        return result == AuthResult.Success
    }
}
