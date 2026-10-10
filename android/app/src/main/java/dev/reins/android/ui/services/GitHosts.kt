package dev.reins.android.ui.services

import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.InlineHelp
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType

/**
 * A git host besides GitHub that is connected with a pasted token: where its token page is (filled in where the host
 * allows it), what to do there, and what a token looks like so that one copied there can be picked up on return.
 */
class GitHost(
    val service: String,
    val name: String,
    private val page: (suffix: Int) -> String,
    /** A copied token is recognised by its shape, so nothing else on the clipboard is ever used; null: never picked up. */
    private val shape: Regex?,
    /** What to do on the token page. */
    val steps: String,
    /** The paste field's placeholder and the line under it. */
    val placeholder: String,
    val hint: String,
    /** The paste field is there from the start (the token alone is not enough to connect). */
    val pasteFirst: Boolean = false,
) {
    fun tokenUrl(suffix: Int = (100_000..999_999).random()): String = page(suffix)

    fun looksLikeToken(text: String?): Boolean {
        val t = text?.trim() ?: return false
        return shape?.matches(t) == true
    }
}

object GitHosts {
    private val all = listOf(
        GitHost(
            service = "gitlab",
            name = "GitLab",
            // GitLab refuses nothing by name, but a fresh number tells the tokens apart in its list.
            page = { "https://gitlab.com/-/user_settings/personal_access_tokens?name=Reins-$it&scopes=read_api,read_repository,write_repository" },
            shape = Regex("^glpat-[A-Za-z0-9_.-]{20,250}$"),
            steps = "GitLab opens with a new token already set up: read_api, read_repository and write_repository. Pick an expiry date, tap Create token, then copy it. Come back here and it connects by itself.",
            placeholder = "Personal access token (glpat-…)",
            hint = "A GitLab personal access token with read_api, read_repository and write_repository. It is kept encrypted on this phone.",
        ),
        GitHost(
            service = "codeberg",
            name = "Codeberg",
            page = { "https://codeberg.org/user/settings/applications" },
            shape = Regex("^[0-9a-f]{40}$"),
            steps = "Under Generate new token, name it Reins, choose Select permissions, and set repository to Read and write and user to Read (read:user, read:repository, write:repository). Tap Generate token and copy it. Come back here and it connects by itself.",
            placeholder = "Access token",
            hint = "A Codeberg access token with read:user and read and write access to repositories. It is kept encrypted on this phone.",
        ),
        GitHost(
            service = "bitbucket",
            name = "Bitbucket",
            page = { "https://id.atlassian.com/manage-profile/security/api-tokens" },
            shape = null,
            steps = "Tap Create API token with scopes, name it Reins, choose Bitbucket, and tick read:user:bitbucket, read:repository:bitbucket and write:repository:bitbucket. Copy the token. Bitbucket checks it together with your Atlassian account email, so paste both below as email:token, for example you@example.com:ATATT3xF… (app passwords no longer work).",
            placeholder = "email:token",
            hint = "Your Atlassian account email, a colon, then the API token. It is kept encrypted on this phone.",
            pasteFirst = true,
        ),
    ).associateBy { it.service }

    /** The host behind [service]; null for GitHub (which has its own page) and everything else. */
    fun of(service: String): GitHost? = all[service]
}

/**
 * A git host in a few taps: open its token page, create and copy the token there, come back. A copied token is picked
 * up from the clipboard and connected; pasting by hand is always possible.
 */
@Composable
internal fun GitHostConnect(host: GitHost, busy: Boolean, onToken: (String) -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    var waiting by remember { mutableStateOf(false) }
    var found by remember { mutableStateOf<String?>(null) }
    var manual by remember { mutableStateOf(host.pasteFirst) }
    fun clearClipboard() {
        runCatching { context.getSystemService(ClipboardManager::class.java)?.clearPrimaryClip() }
    }
    androidx.lifecycle.compose.LifecycleEventEffect(androidx.lifecycle.Lifecycle.Event.ON_RESUME) {
        val copied = clipboardText(context)?.trim()
        if (host.looksLikeToken(copied)) {
            if (waiting && !busy) {
                waiting = false
                // Do not leave the token lying on the clipboard.
                clearClipboard()
                onToken(copied!!)
            } else {
                found = copied
            }
        }
    }
    CapsuleButton("Create a token on ${host.name}", Modifier.fillMaxWidth().testTag("openTokenPage"), enabled = !busy, glyph = Glyph.Key) {
        waiting = true
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(host.tokenUrl())).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
    InlineHelp("${host.name} tokens", host.steps + "\n\n" + host.hint, Modifier.testTag("tokenSteps"), label = "Opens ${host.name}")
    found?.let { token ->
        CapsuleButton("Use the token I copied", Modifier.fillMaxWidth().testTag("useCopied"), style = ButtonStyle.Accent, enabled = !busy) {
            found = null
            clearClipboard()
            onToken(token)
        }
    }
    if (!manual) {
        CapsuleButton("I already have a token", Modifier.fillMaxWidth().testTag("pasteManually"), style = ButtonStyle.Ghost, enabled = !busy) { manual = true }
    } else {
        SecretForm(placeholder = host.placeholder, button = "Connect", busy = busy, onSubmit = onToken)
    }
}
