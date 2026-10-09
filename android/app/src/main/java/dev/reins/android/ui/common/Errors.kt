package dev.reins.android.ui.common

import dev.reins.android.platform.PasskeyException
import dev.reins.core.CoreException
import dev.reins.core.ForeignException

/** Why this phone may not become the approval device yet, and the two ways on (the Unlock screen offers both). */
const val OTHER_APPROVAL_DEVICE =
    "This account already has a phone for approvals. Approve this phone from it, or enter your recovery code."

/** The passkey provider has no PRF: its passkeys cannot open the vault, so none is added. */
const val PASSKEY_UNSUPPORTED = "This password manager can't unlock Reins. Choose another one, or keep the recovery code."

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
    is CoreException.OtherApprovalDevice -> OTHER_APPROVAL_DEVICE
    is PasskeyException -> message
    is ForeignException.Failed -> untrusted(reason)
    is ForeignException.NeedsUserInteraction -> "Gmail needs your permission. Open Settings and connect Gmail."
    else -> "Something went wrong (${javaClass.simpleName})."
}
