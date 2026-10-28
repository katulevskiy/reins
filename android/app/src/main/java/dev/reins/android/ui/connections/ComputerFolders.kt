package dev.reins.android.ui.connections

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalButton
import dev.reins.android.design.hairlineBelow
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.activity.FOLDERS_HELP
import dev.reins.android.ui.activity.folderKey
import dev.reins.android.ui.activity.folderKindName
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.android.ui.settings.SettingsViewModel
import dev.reins.core.FolderView

/**
 * A computer's Folders: the ones trusted (signed into its policy), each with a remove (it only tightens: at once, no
 * owner check), then the ones it suggests and this phone has not trusted (dismissed ones too), each with Trust (the
 * owner check first). Paths are shown whole, wrapped, as the computer gave them. Nothing to show: no section at all.
 */
@Composable
fun ComputerFoldersSection(connectionId: String, viewModel: SettingsViewModel, authenticator: Authenticator?) {
    LaunchedEffect(connectionId) { viewModel.loadFolders(connectionId) }
    val read by viewModel.folders.collectAsStateWithLifecycle()
    val folders = read?.takeIf { it.connectionId == connectionId } ?: return
    if (folders.confirmed.isEmpty() && folders.suggested.isEmpty()) return
    var help by rememberSaveable { mutableStateOf(false) }
    SettingsGroup(
        Modifier.testTag("computerFolders"),
        header = "Folders",
        help = FOLDERS_HELP,
        helpOpen = help,
        onHelp = { help = it },
        glyph = Glyph.FolderSearch,
    ) {
        folders.confirmed.forEach { folder ->
            FolderRow(folder, trusted = true, tag = "trustedFolder:${folderKey(connectionId, folder.path, folder.kind)}") {
                RoundIconButton(
                    Glyph.Trash,
                    "Remove",
                    Modifier.testTag("removeFolder:${folderKey(connectionId, folder.path, folder.kind)}"),
                    size = 36.dp,
                ) { viewModel.removeFolder(connectionId, folder) }
            }
        }
        folders.suggested.forEach { folder ->
            FolderRow(folder, trusted = false, tag = "suggestedFolder:${folderKey(connectionId, folder.path, folder.kind)}") {
                // Without the owner's check nothing is trusted here.
                if (authenticator != null) {
                    SignalButton(
                        "Trust",
                        Modifier.testTag("confirmFolder:${folderKey(connectionId, folder.path, folder.kind)}"),
                        style = ButtonStyle.Soft,
                        size = ButtonSize.Small,
                        glyph = Glyph.Check,
                    ) { viewModel.trustFolder(connectionId, folder, authenticator) }
                }
            }
        }
    }
}

/** One folder: its tile (a shield once trusted), the path whole in mono, its kind under it, then [action]. */
@Composable
private fun FolderRow(folder: FolderView, trusted: Boolean, tag: String, action: @Composable () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().defaultMinSize(minHeight = 56.dp).hairlineBelow(c.rule).testTag(tag).padding(vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(32.dp).clip(RoundedCornerShape(9.dp)).background(c.paperSunken), contentAlignment = Alignment.Center) {
            GlyphIcon(if (trusted) Glyph.ShieldCheck else Glyph.FolderSearch, c.ink2, size = 18.dp)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            RText(untrusted(folder.path), RType.mono(13.5f), c.ink, Modifier.testTag("$tag:path"), ltr = true)
            RText(
                if (trusted) folderKindName(folder.kind) else "${folderKindName(folder.kind)} · Suggested",
                RType.meta,
                c.inkMeta,
                Modifier.padding(top = 2.dp).testTag("$tag:kind"),
                maxLines = 1,
            )
        }
        Spacer(Modifier.width(10.dp))
        action()
    }
}
