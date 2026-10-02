package dev.rewarden.android.ui.activity

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.ConnectorTags
import dev.rewarden.android.design.EmptyState
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen
import dev.rewarden.android.design.Spinner
import dev.rewarden.android.state.AppState
import dev.rewarden.android.ui.common.formatFull
import dev.rewarden.android.ui.common.untrusted
import dev.rewarden.android.ui.common.userMessage
import dev.rewarden.core.EmailContent
import kotlin.coroutines.cancellation.CancellationException

/** What opening an email came to. */
sealed interface EmailLoad {
    data object Loading : EmailLoad

    data class Loaded(val email: EmailContent) : EmailLoad

    data class Failed(val message: String) : EmailLoad
}

/**
 * One email that an AI was given, opened in full. The phone keeps only which emails were shared, so the text is fetched
 * from Gmail now (and an email deleted since says so). The header from the log shows while it loads.
 */
@Composable
fun EmailScreen(
    entryId: Long,
    index: Int,
    state: AppState,
    load: suspend (account: String?, messageId: String) -> EmailContent,
    onBack: () -> Unit,
) {
    val c = LocalColors.current
    val entries by state.activity.collectAsStateWithLifecycle()
    val entry = entries.firstOrNull { it.id == entryId }
    val logged = entry?.info?.messages?.getOrNull(index)
    var result by remember(entryId, index) { mutableStateOf<EmailLoad>(EmailLoad.Loading) }
    var attempt by remember { mutableIntStateOf(0) }

    LaunchedEffect(entryId, index, attempt) {
        val id = logged?.id?.takeIf { it.isNotEmpty() } ?: return@LaunchedEffect
        result = EmailLoad.Loading
        result = try {
            EmailLoad.Loaded(load(entry.account, id))
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            EmailLoad.Failed(e.userMessage())
        }
    }

    Screen(title = "Email", onBack = onBack) {
        if (entry == null || logged == null) {
            EmptyState(Glyph.Mail, "Not found", "That email is no longer in the history.", tag = "emailGone")
            return@Screen
        }
        val email = (result as? EmailLoad.Loaded)?.email
        Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp)) {
            RText(
                untrusted(email?.subject ?: logged.subject).ifBlank { "(no subject)" },
                RType.sans(22f, FontWeight.SemiBold, lineHeight = 28f),
                c.text,
                Modifier.testTag("emailSubject"),
            )
            ConnectorTags(entry.service, entry.account, Modifier.padding(top = 10.dp))
            Header("From", untrusted(email?.from ?: logged.from), Modifier.padding(top = 16.dp).testTag("emailFrom"))
            email?.to?.takeIf { it.isNotEmpty() }?.let { Header("To", it.joinToString(", ") { a -> untrusted(a) }, Modifier.testTag("emailTo")) }
            email?.cc?.takeIf { it.isNotEmpty() }?.let { Header("Cc", it.joinToString(", ") { a -> untrusted(a) }) }
            Header("Date", formatFull(email?.date ?: logged.date))
        }
        when (val r = result) {
            EmailLoad.Loading -> Row(Modifier.fillMaxWidth().padding(24.dp).testTag("emailLoading"), verticalAlignment = Alignment.CenterVertically) {
                Spinner(c.accent, size = 22.dp)
                Spacer(Modifier.width(12.dp))
                RText("Fetching it from Gmail…", RType.sans(15f), c.secondary)
            }
            is EmailLoad.Failed -> Column(Modifier.padding(16.dp)) {
                Banner(untrusted(r.message), kind = BannerKind.Warning, tag = "emailError")
                CapsuleButton("Try again", Modifier.padding(top = 12.dp).testTag("emailRetry"), style = ButtonStyle.Secondary, compact = true) { attempt++ }
            }
            is EmailLoad.Loaded -> SelectionContainer {
                RText(
                    untrusted(r.email.body).ifBlank { "(This email has no text.)" },
                    RType.sans(16f, lineHeight = 23f),
                    c.text,
                    Modifier.padding(horizontal = 20.dp, vertical = 12.dp).testTag("emailBody"),
                )
            }
        }
        Spacer(Modifier.padding(bottom = 32.dp))
    }
}

@Composable
private fun Header(label: String, value: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().padding(top = 6.dp)) {
        RText(label, RType.sans(13f, FontWeight.Medium), c.tertiary, Modifier.width(52.dp))
        RText(value, RType.sans(14.5f), c.text, Modifier.weight(1f), ltr = true)
    }
}
