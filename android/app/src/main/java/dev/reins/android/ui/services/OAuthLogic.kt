package dev.reins.android.ui.services

import dev.reins.android.ui.mcp.isAppRedirect
import java.net.URI

/** Where X's sign-in page sends the browser back to (the core registers it with X). */
const val X_REDIRECT_HOST = "x-oauth"

/** X's sign-in page sent the browser to the app's own X redirect address (anything else is never handed to the core). */
fun isXRedirect(uri: String?): Boolean = isAppRedirect(uri, X_REDIRECT_HOST)

/** A sign-in page the core gives for an "oauth" service may only be an https page. */
fun isSignInPage(url: String): Boolean = try {
    val uri = URI(url)
    uri.scheme == "https" && !uri.host.isNullOrEmpty()
} catch (e: java.net.URISyntaxException) {
    false
}
