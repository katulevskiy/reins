package dev.reins.android.platform

import android.content.Context
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.services.isXRedirect
import dev.reins.core.AccountView
import dev.reins.core.ReinsCoreInterface
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** How a sign-in in the browser (X) ended. */
sealed interface OAuthSignInResult {
    data class Done(val account: AccountView) : OAuthSignInResult

    data class Failed(val service: String, val message: String) : OAuthSignInResult

    /** Not the redirect, or no sign-in was waiting for one: nothing was sent to the core. */
    data object Ignored : OAuthSignInResult
}

/**
 * The sign-in to a service whose sign-in the core runs ("oauth", X), waiting for its browser page to come back. Like
 * [McpSignIn], the service is kept on disk (it is not secret) so a sign-in still finishes when Android stopped the app
 * while the page was open. Each redirect is used once; the core checks that it belongs to the sign-in it started.
 */
class OAuthSignIn(
    context: Context,
    private val core: () -> ReinsCoreInterface,
    private val state: AppState,
    private val now: () -> Long = System::currentTimeMillis,
) {
    private val prefs = SealedPreferences(context, "oauth_sign_in")

    private val _finishing = MutableStateFlow<String?>(null)

    /** The service whose redirect is being handed to the core right now. */
    val finishing: StateFlow<String?> = _finishing.asStateFlow()

    /** The sign-in page of [service] is about to open. */
    fun begin(service: String) {
        prefs.edit().putString(KEY_SERVICE, service).putLong(KEY_STARTED, now()).commit()
    }

    /** The service whose sign-in waits for its redirect, unless it was started too long ago. */
    fun pending(): String? {
        val service = prefs.getString(KEY_SERVICE, null) ?: return null
        return service.takeIf { now() - prefs.getLong(KEY_STARTED, 0) in 0..MAX_AGE_MILLIS }
    }

    fun clear() {
        prefs.edit().clear().commit()
    }

    /** Hands [redirect] to the core for the waiting sign-in. */
    suspend fun finish(redirect: String): OAuthSignInResult {
        if (!isXRedirect(redirect)) return OAuthSignInResult.Ignored
        val epoch = state.accountEpoch.value
        val service = pending()?.takeIf { it == X } ?: return OAuthSignInResult.Ignored
        clear()
        _finishing.value = service
        val result = try {
            OAuthSignInResult.Done(core().oauthFinish(service, redirect))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            OAuthSignInResult.Failed(service, e.userMessage())
        } finally {
            _finishing.value = null
        }
        return if (state.isCurrent(epoch)) result else OAuthSignInResult.Ignored
    }

    private companion object {
        const val X = "x"
        const val KEY_SERVICE = "service"
        const val KEY_STARTED = "started_at"

        /** A sign-in left open longer than this is not finished by a late redirect. */
        const val MAX_AGE_MILLIS = 30L * 60 * 1000
    }
}
