package dev.reins.android.ui.activity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.AppContainer
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalButton
import dev.reins.android.design.fieldSurface
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.OwnerPrompt
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.untrustedName
import dev.reins.android.ui.common.userMessage
import dev.reins.core.FolderKindChoice
import dev.reins.core.FolderQuestion
import kotlin.coroutines.cancellation.CancellationException

// A computer's folders: the Reins desktop app proposes its workspace folders (project roots, scratch folders) and this
// phone signs the ones the owner trusts into that computer's policy. Each proposal is a card in the question cards'
// shell, after them. Never a notification: the cards are only listed here.

/** "Projects" for a root (its folders are projects), "Scratch" for a scratch folder (deleting in it loses nothing). */
fun folderKindName(kind: FolderKindChoice): String = when (kind) {
    FolderKindChoice.ROOT -> "Projects"
    FolderKindChoice.SCRATCH -> "Scratch"
}

/** What the (?) beside a computer's folders says. */
const val FOLDERS_HELP = "Trusted folders go into this computer's signed policy. Projects: each folder in it is a project. " +
    "Scratch: deleting in it loses nothing. * stands for any one folder."

/** One folder of one computer, for test tags and keys: "d1:ROOT:/home/me/code". */
fun folderKey(connectionId: String, path: String, kind: FolderKindChoice): String = "$connectionId:${kind.name}:$path"

fun folderKey(q: FolderQuestion): String = folderKey(q.connectionId, q.path, q.kind)

/** The open folder proposals, one card each, newest first. */
@Composable
fun FolderCards(
    folders: List<FolderQuestion>,
    onTrust: (FolderQuestion) -> Unit,
    onNotNow: (FolderQuestion) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (folders.isEmpty()) return
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("folderCards"), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        folders.forEach { q -> FolderCard(q, { onTrust(q) }, { onNotNow(q) }) }
    }
}

/**
 * One proposal in the question cards' shell: the label line (the computer, its folder kind, a (?)), the question in the
 * serif, the path exactly as the computer gave it (wrapped, never cut), then Trust and Not now.
 */
@Composable
private fun FolderCard(q: FolderQuestion, onTrust: () -> Unit, onNotNow: () -> Unit) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(26.dp)
    val key = folderKey(q)
    var help by rememberSaveable(key) { mutableStateOf(false) }
    Column(
        Modifier
            .fillMaxWidth()
            .fieldSurface(Field.Grant, shape)
            .clip(shape)
            .testTag("folder:$key")
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            GlyphIcon(Glyph.Laptop, c.onField, size = 18.dp)
            RText(
                "${untrustedName(q.computer, 32)} · ${folderKindName(q.kind)}",
                RType.sans(14f, FontWeight.SemiBold),
                c.onField,
                Modifier.weight(1f).testTag("folderWho:$key"),
                maxLines = 1,
            )
            RoundIconButton(Glyph.Help, if (help) "Hide help" else "About folders", Modifier.testTag("folderHelp:$key"), onField = true, size = 32.dp) {
                help = !help
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            RText("Trust this folder?", RType.askSmall, c.onField, Modifier.semantics { heading() }.testTag("folderTitle:$key"), maxLines = 2)
            // The path whole: what is signed is exactly what is shown.
            RText(untrusted(q.path), RType.mono(14f), c.onField, Modifier.testTag("folderPath:$key"), ltr = true)
            if (help) RText(FOLDERS_HELP, RType.sans(13f, lineHeight = 18f), c.onField2, Modifier.testTag("folderHelpText:$key"))
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SignalButton(
                "Trust",
                Modifier.weight(1f).testTag("trustFolder:$key"),
                style = ButtonStyle.OnFieldWhite,
                size = ButtonSize.Small,
                glyph = Glyph.Check,
                onClick = onTrust,
            )
            SignalButton(
                "Not now",
                Modifier.weight(1f).testTag("notNowFolder:$key"),
                style = ButtonStyle.OnFieldWash,
                size = ButtonSize.Small,
                onClick = onNotNow,
            )
        }
    }
}

/**
 * Trusts the folder [path] of computer [connectionId]: it loosens, so the core asks the owner first ([OwnerPrompt]).
 * Its card leaves at once and comes back when the trust did not land. Returns what to say, or null.
 */
suspend fun trustFolder(container: AppContainer, authenticator: Authenticator, connectionId: String, path: String, kind: FolderKindChoice): String? {
    val owner = OwnerPrompt(authenticator, "Trust a folder")
    container.state.removeFolderQuestion(connectionId, path, kind)
    return try {
        container.core.confirmComputerFolder(connectionId, path, kind, owner)
        container.feedback.play(Event.Selection)
        null
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        when (owner.last) {
            AuthResult.Cancelled -> null
            AuthResult.Unavailable -> "Set a screen lock or fingerprint on this phone first."
            else -> {
                container.feedback.play(Event.Error)
                e.userMessage()
            }
        }
    } finally {
        rereadFolderQuestions(container)
    }
}

/** "Not now": the card leaves; the folder stays suggested on the computer's page. Changes nothing signed. */
suspend fun dismissFolder(container: AppContainer, q: FolderQuestion): String? {
    container.state.removeFolderQuestion(q.connectionId, q.path, q.kind)
    return try {
        container.core.dismissComputerFolder(q.connectionId, q.path, q.kind)
        null
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        container.feedback.play(Event.Error)
        e.userMessage()
    } finally {
        rereadFolderQuestions(container)
    }
}

/** Removes a trusted folder: it only tightens, so it applies at once (no owner check). Returns what to say, or null. */
suspend fun removeFolder(container: AppContainer, connectionId: String, path: String, kind: FolderKindChoice): String? = try {
    container.core.removeComputerFolder(connectionId, path, kind)
    container.feedback.play(Event.Revoked)
    null
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    container.feedback.play(Event.Error)
    e.userMessage()
} finally {
    rereadFolderQuestions(container)
}

private suspend fun rereadFolderQuestions(container: AppContainer) {
    try {
        container.state.setFolderQuestions(container.core.folderQuestions())
    } catch (e: CancellationException) {
        throw e
    } catch (_: Exception) {
    }
}
