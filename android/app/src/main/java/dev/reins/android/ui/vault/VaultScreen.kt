package dev.reins.android.ui.vault

import androidx.compose.ui.semantics.semantics
import dev.reins.android.feedback.play
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.height
import androidx.compose.ui.draw.clip
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import dev.reins.android.design.RowTile
import dev.reins.android.design.PaneRow
import dev.reins.android.design.PaneGroup
import dev.reins.android.design.SectionHeader
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.CircleIconButton
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.ListRow
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.pressable
import dev.reins.android.ui.common.untrusted
import dev.reins.core.VaultItemKind
import dev.reins.core.VaultItemSummary

/** What the user picks under "Add": the kinds Reins uses, each with the item kind it makes. */
enum class NewItem(val title: String, val detail: String, val kind: VaultItemKind, val glyph: Glyph) {
    ApiKey("API key", "For reins run and the API proxy: vault:NAME/password", VaultItemKind.LOGIN, Glyph.Key),
    SshKey("SSH key", "Made on this phone; the private key never leaves the vault", VaultItemKind.SSH_KEY, Glyph.Terminal),
    Login("Login", "A username and password for a website", VaultItemKind.LOGIN, Glyph.Person),
    Note("Secure note", "Any text: vault:NAME/notes", VaultItemKind.NOTE, Glyph.FileText),
    Card("Card", "A payment card", VaultItemKind.CARD, Glyph.Card),
    Identity("Identity", "Your name, address and documents", VaultItemKind.IDENTITY, Glyph.IdCard),
}

fun kindLabel(kind: VaultItemKind): String = when (kind) {
    VaultItemKind.LOGIN -> "Login"
    VaultItemKind.NOTE -> "Secure note"
    VaultItemKind.CARD -> "Card"
    VaultItemKind.IDENTITY -> "Identity"
    VaultItemKind.SSH_KEY -> "SSH key"
}

fun kindGlyph(kind: VaultItemKind): Glyph = when (kind) {
    VaultItemKind.LOGIN -> Glyph.Person
    VaultItemKind.NOTE -> Glyph.FileText
    VaultItemKind.CARD -> Glyph.Card
    VaultItemKind.IDENTITY -> Glyph.IdCard
    VaultItemKind.SSH_KEY -> Glyph.Terminal
}

/** Integrations > Password vault > Open the vault: every item, a search, and "Add". */
@Composable
fun VaultScreen(viewModel: VaultViewModel, onBack: () -> Unit, onOpen: (String) -> Unit, onAdd: () -> Unit) {
    val c = LocalColors.current
    val items by viewModel.items.collectAsStateWithLifecycle()
    val query by viewModel.query.collectAsStateWithLifecycle()
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val phoneKey by viewModel.phoneKey.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.load() }
    // Names, usernames, card digits and this phone's key: not for screenshots or the recent apps.
    dev.reins.android.ui.common.SecureWindow()

    val context = androidx.compose.ui.platform.LocalContext.current
    val feedback = dev.reins.android.feedback.LocalFeedback.current
    var help by remember { mutableStateOf(false) }
    // Next (Vault 01): a search well, the items in a raised group under a header with its +, and how a computer asks.
    Screen(title = "Vault", onBack = onBack) {
        SearchWell(query, viewModel::search, Modifier.padding(horizontal = 16.dp).padding(top = 8.dp))
        ui.error?.let { Banner(untrusted(it), Modifier.padding(horizontal = 16.dp, vertical = 8.dp), BannerKind.Error, tag = "vaultError") }
        val current = items
        when {
            current == null -> RText("Loading…", RType.sans(15f), c.ink2, Modifier.padding(32.dp))
            current.isEmpty() && query.isBlank() -> {
                SectionHeader("Items", glyph = Glyph.Vault, count = "0", onAdd = onAdd, addLabel = "Add", addTag = "vaultAdd")
                EmptyState(Glyph.Key, "Nothing in the vault yet", "A computer finds an item by name, like vault:OpenAI/password.", tag = "vaultEmpty")
            }
            current.isEmpty() -> EmptyState(Glyph.Search, "Nothing matches", null, tag = "vaultNoMatch")
            else -> {
                SectionHeader("Items", glyph = Glyph.Vault, count = current.size.toString(), onAdd = onAdd, addLabel = "Add", addTag = "vaultAdd")
                PaneGroup {
                    current.forEachIndexed { i, item -> ItemRow(item, first = i == 0) { onOpen(item.id) } }
                }
            }
        }
        SectionHeader("From a computer", glyph = Glyph.Laptop, trailing = { dev.reins.android.ui.settings.HelpIcon(help, "the vault from a computer") { help = it } })
        dev.reins.android.design.HelpText(
            "reins run and the API proxy find an item by its exact name: vault:NAME/password, /username, /notes or a " +
                "custom field's name. The SSH agent offers every SSH key. From a computer, reins vault add NAME saves " +
                "a secret here without showing it to an AI.",
            help,
            Modifier.padding(horizontal = 24.dp).padding(bottom = 8.dp),
        )
        PaneGroup {
            PaneRow(
                "vault:NAME/password",
                leading = { RowTile(Glyph.Terminal) },
                trailing = { RoundCopy("Copy", "copyVaultRef") { copyPlain(context, "vault:NAME/password"); feedback.play(dev.reins.android.feedback.Event.Copied) } },
                first = true,
                titleMono = true,
            )
            phoneKey?.let { key ->
                PaneRow(
                    "Phone key",
                    leading = { RowTile(Glyph.Phone) },
                    value = key,
                    valueMono = true,
                    trailing = { RoundCopy("Copy phone key", "copyPhoneKey") { copyPlain(context, key); feedback.play(dev.reins.android.feedback.Event.Copied) } },
                    tag = "phoneKey",
                )
            }
        }
        Spacer(Modifier.height(32.dp))
    }
}

/** The search well (`.srch`): sunken, 44 tall, radius 14, the glyph and the words. */
@Composable
internal fun SearchWell(query: String, onChange: (String) -> Unit, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().height(44.dp).clip(RoundedCornerShape(14.dp)).background(c.paperSunken).padding(horizontal = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(Glyph.Search, c.inkMeta, size = 18.dp)
        Box(Modifier.weight(1f)) {
            if (query.isEmpty()) RText("Search", RType.sans(16f), c.inkMeta, maxLines = 1)
            androidx.compose.foundation.text.BasicTextField(
                query,
                onChange,
                Modifier.fillMaxWidth().testTag("vaultSearch"),
                textStyle = RType.sans(16f).copy(color = c.ink),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(c.violet),
                singleLine = true,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
            )
        }
    }
}

/** A round copy button (40 dp) in ink-2. */
@Composable
internal fun RoundCopy(label: String, tag: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier.size(40.dp).clip(androidx.compose.foundation.shape.CircleShape).pressable(shape = androidx.compose.foundation.shape.CircleShape, label = label, onClick = onClick)
            .semantics { contentDescription = label }.testTag(tag),
        contentAlignment = Alignment.Center,
    ) { GlyphIcon(Glyph.Copy, c.ink2, size = 18.dp) }
}

internal fun copyPlain(context: android.content.Context, text: String) {
    context.getSystemService(android.content.ClipboardManager::class.java)?.setPrimaryClip(android.content.ClipData.newPlainText("Reins", text))
}

@Composable
private fun ItemRow(item: VaultItemSummary, first: Boolean, onClick: () -> Unit) {
    val c = LocalColors.current
    PaneRow(
        untrusted(item.name),
        leading = { RowTile(kindGlyph(item.kind)) },
        value = untrusted(item.subtitle).takeIf { it.isNotBlank() && !item.favorite }?.let { if (it.length > 18) it.take(17) + "…" else it },
        valueMono = item.kind == VaultItemKind.SSH_KEY,
        trailing = if (item.favorite) ({ GlyphIcon(Glyph.StarFilled, c.warning, size = 18.dp) }) else null,
        chevron = true,
        first = first,
        tag = "vaultItem:${item.id}",
        onClick = onClick,
    )
}

/** Vault > Add: what kind of item. */
@Composable
fun VaultAddScreen(onBack: () -> Unit, onPick: (NewItem) -> Unit) {
    val c = LocalColors.current
    // Next (Vault 02): the kinds as flat rows, each with its tile; a kind opens its form.
    Screen(title = "Add", onBack = onBack) {
        androidx.compose.runtime.CompositionLocalProvider(dev.reins.android.ui.settings.LocalRaisedGroups provides false) {
            dev.reins.android.ui.settings.SettingsGroup(header = "Kind", gap = 10.dp) {
                NewItem.entries.forEach { kind ->
                    dev.reins.android.ui.settings.SetRow(kind.title, kind.glyph, Modifier.testTag("newItem:${kind.name}")) { onPick(kind) }
                }
            }
        }
        Spacer(Modifier.height(32.dp))
    }
}
