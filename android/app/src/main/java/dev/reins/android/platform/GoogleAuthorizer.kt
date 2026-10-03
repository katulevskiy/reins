package dev.reins.android.platform

import android.accounts.Account
import android.accounts.AccountManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.auth.api.identity.RevokeAccessRequest
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.CommonStatusCodes
import com.google.android.gms.common.api.Scope
import dev.reins.core.ForeignException
import dev.reins.core.GoogleTokenProvider

/**
 * Google access tokens (Gmail, Calendar, Contacts) come straight from Google Play Services on this phone: no refresh
 * token, no client secret, nothing stored. The core asks for a token for every call, naming the account and the
 * service; consent is a one-time user interaction per account and service. An empty account name means the phone's
 * default Google account (used once, by phones that connected Gmail before accounts were tracked).
 */
class GoogleAuthorizer(context: Context) : GoogleTokenProvider {
    private val client = Identity.getAuthorizationClient(context.applicationContext)

    private fun request(account: String, service: String): AuthorizationRequest = AuthorizationRequest.builder()
        .setRequestedScopes(scopesOf(service).map(::Scope))
        .apply { if (account.isNotEmpty()) setAccount(Account(account, GOOGLE_ACCOUNT_TYPE)) }
        .build()

    override suspend fun accessToken(account: String, service: String): String {
        val result = authorize(account, service)
        if (result.hasResolution()) throw ForeignException.NeedsUserInteraction()
        return result.accessToken ?: throw ForeignException.Failed("Google returned no access token")
    }

    /** The consent screen to show for [account] and [service], or null when consent was already given. */
    suspend fun consentIntent(account: String, service: String): PendingIntent? {
        val result = authorize(account, service)
        return if (result.hasResolution()) result.pendingIntent else null
    }

    /** Revokes this app's access to [service] for [account]. */
    suspend fun revoke(account: String, service: String) {
        val google = Account(account, GOOGLE_ACCOUNT_TYPE)
        client.revokeAccess(RevokeAccessRequest.builder().setAccount(google).setScopes(scopesOf(service).map(::Scope)).build()).await()
    }

    private suspend fun authorize(account: String, service: String): AuthorizationResult = try {
        client.authorize(request(account, service)).await()
    } catch (e: ApiException) {
        throw if (e.statusCode == CommonStatusCodes.DEVELOPER_ERROR) {
            ForeignException.Failed(SETUP_NEEDED)
        } else {
            ForeignException.Failed("Google could not authorize ${nameOf(service)} (code ${e.statusCode})")
        }
    } catch (e: Exception) {
        if (e is kotlin.coroutines.cancellation.CancellationException) throw e
        throw ForeignException.Failed("Google Play services is not available: ${e.javaClass.simpleName}")
    }

    companion object {
        const val GMAIL_READONLY = "https://www.googleapis.com/auth/gmail.readonly"
        const val GMAIL_SEND = "https://www.googleapis.com/auth/gmail.send"
        const val CALENDAR_EVENTS = "https://www.googleapis.com/auth/calendar.events"
        const val CALENDAR_READONLY = "https://www.googleapis.com/auth/calendar.readonly"
        const val CONTACTS_READONLY = "https://www.googleapis.com/auth/contacts.readonly"
        const val GOOGLE_ACCOUNT_TYPE = "com.google"
        const val SETUP_NEEDED =
            "Google access needs setup: register this app's package and signing SHA-1 as an Android OAuth client in Google Cloud, and enable the Gmail, Calendar and People APIs."

        /** The OAuth scopes a Google service needs. */
        fun scopesOf(service: String): List<String> = when (service) {
            "gcalendar" -> listOf(CALENDAR_EVENTS, CALENDAR_READONLY)
            "gcontacts" -> listOf(CONTACTS_READONLY)
            else -> listOf(GMAIL_READONLY, GMAIL_SEND)
        }

        private fun nameOf(service: String) = when (service) {
            "gcalendar" -> "Google Calendar"
            "gcontacts" -> "Google Contacts"
            else -> "Gmail"
        }

        /** Android's own Google account picker, which also offers "Add account". */
        fun chooseAccountIntent(): Intent = AccountManager.newChooseAccountIntent(
            null, null, arrayOf(GOOGLE_ACCOUNT_TYPE), null, null, null, null,
        )

        /** The address the account picker returned, if the user picked one. */
        fun chosenAccount(data: Intent?): String? =
            data?.getStringExtra(AccountManager.KEY_ACCOUNT_NAME)?.trim()?.lowercase()?.takeIf { '@' in it }
    }
}
