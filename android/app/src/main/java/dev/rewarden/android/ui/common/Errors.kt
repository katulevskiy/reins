package dev.rewarden.android.ui.common

import dev.rewarden.core.CoreException
import dev.rewarden.core.ForeignException

/** A message the user can act on. Core error text never contains secrets or URLs. */
fun Throwable.userMessage(): String = when (this) {
    is CoreException.NotLoggedIn -> "You're signed out. Sign in again."
    is CoreException.TwoFactorRequired -> "Enter your two-factor code."
    is CoreException.UnsupportedTwoFactor ->
        "This account's two-factor method isn't supported. Use an authenticator app (TOTP)."
    is CoreException.InvalidCredentials -> "Wrong email, password or two-factor code."
    is CoreException.Network -> "Can't reach the server. Check your connection and try again."
    is CoreException.Server -> if (status.toInt() == 403) {
        "Another phone is your approval device now."
    } else {
        "The server had a problem (${status}). ${untrusted(reason)}".trim()
    }
    is CoreException.GmailNeedsConsent -> "Gmail needs your permission. Open Settings and connect Gmail."
    is CoreException.Gmail -> "Gmail: ${untrusted(reason)}"
    is CoreException.Service -> untrusted(reason)
    is CoreException.ServiceNeedsAttention -> untrusted(reason)
    is CoreException.NotFound -> "That request is no longer waiting."
    is CoreException.Invalid -> untrusted(reason)
    is CoreException.Storage -> "Storage problem: ${untrusted(reason)}"
    is ForeignException.Failed -> untrusted(reason)
    is ForeignException.NeedsUserInteraction -> "Gmail needs your permission. Open Settings and connect Gmail."
    else -> "Something went wrong (${javaClass.simpleName})."
}
