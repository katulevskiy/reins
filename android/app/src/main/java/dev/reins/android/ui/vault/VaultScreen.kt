package dev.reins.android.ui.vault

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
    SshKey("SSH key", "Made on this phone; the private key never leaves the vault", VaultItemKind.SSH_KEY, Glyph.Lock),
    Login("Login", "A username and password for a website", VaultItemKind.LOGIN, Glyph.Link),
    Note("Secure note", "Any text: vault:NAME/notes", VaultItemKind.NOTE, Glyph.List),
    Card("Card", "A payment card", VaultItemKind.CARD, Glyph.Tray),
    Identity("Identity", "Your name, address and documents", VaultItemKind.IDENTITY, Glyph.People),
}

fun kindLabel(kind: VaultItemKind): String = when (kind) {
    VaultItemKind.LOGIN -> "Login"
    VaultItemKind.NOTE -> "Secure note"
    VaultItemKind.CARD -> "Card"
    VaultItemKind.IDENTITY -> "Identity"
    VaultItemKind.SSH_KEY -> "SSH key"
}

fun kindGlyph(kind: VaultItemKind): Glyph = when (kind) {
    VaultItemKind.LOGIN -> Glyph.Key
    VaultItemKind.NOTE -> Glyph.List
    VaultItemKind.CARD -> Glyph.Tray
    VaultItemKind.IDENTITY -> Glyph.People
    VaultItemKind.SSH_KEY -> Glyph.Lock
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

    Screen(
        title = "Vault",
        subtitle = "Password vault",
        onBack = onBack,
        actions = { CircleIconButton(Glyph.Plus, Modifier.testTag("vaultAdd"), label = "Add", onClick = onAdd) },
    ) {
        Box(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
            RTextField(
                query,
                viewModel::search,
                "Search names, usernames, websites",
                tag = "vaultSearch",
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
            )
        }
        ui.error?.let { Banner(untrusted(it), Modifier.padding(horizontal = 16.dp, vertical = 8.dp), BannerKind.Error, tag = "vaultError") }
        val current = items
        when {
            current == null -> RText("Loading…", RType.sans(15f), c.secondary, Modifier.padding(32.dp))
            current.isEmpty() && query.isBlank() -> EmptyState(
                Glyph.Key,
                "Nothing in the vault yet",
                "Add an API key, a login or an SSH key. The desktop app finds them by name, like vault:OpenAI/password.",
                tag = "vaultEmpty",
            )
            current.isEmpty() -> EmptyState(Glyph.Search, "Nothing matches", null, tag = "vaultNoMatch")
            else -> Group(header = "${current.size} ${if (current.size == 1) "item" else "items"}") {
                current.forEachIndexed { i, item ->
                    if (i > 0) Hairline(inset = 68.dp)
                    ItemRow(item) { onOpen(item.id) }
                }
            }
        }
        RText(
            "reins run and the API proxy find an item by its exact name: vault:NAME/password, /username, /notes or a " +
                "custom field's name. The SSH agent offers every SSH key. From a computer, reins vault add NAME saves " +
                "a secret here without showing it to an AI.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 32.dp, end = 32.dp, top = 16.dp, bottom = if (phoneKey == null) 32.dp else 8.dp),
        )
        phoneKey?.let {
            RText(
                "This phone's key for reins vault add: $it",
                RType.sans(13.5f, lineHeight = 19f),
                c.tertiary,
                Modifier.padding(start = 32.dp, end = 32.dp, bottom = 32.dp).testTag("phoneKey"),
            )
        }
    }
}

@Composable
private fun ItemRow(item: VaultItemSummary, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .testTag("vaultItem:${item.id}")
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).background(c.accentSoft, RoundedCornerShape(12.dp)), contentAlignment = Alignment.Center) {
            GlyphIcon(kindGlyph(item.kind), c.accent, size = 20.dp)
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(untrusted(item.name), RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1)
            RText(
                untrusted(item.subtitle).ifBlank { kindLabel(item.kind) },
                RType.sans(13f),
                c.secondary,
                Modifier.padding(top = 2.dp),
                maxLines = 1,
            )
        }
        if (item.favorite) {
            GlyphIcon(Glyph.Star, c.warning, size = 16.dp)
            Spacer(Modifier.width(6.dp))
        }
        GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp)
    }
}

/** Vault > Add: what kind of item. */
@Composable
fun VaultAddScreen(onBack: () -> Unit, onPick: (NewItem) -> Unit) {
    val c = LocalColors.current
    Screen(title = "Add to the vault", onBack = onBack) {
        Group(header = "What to add") {
            NewItem.entries.forEachIndexed { i, kind ->
                if (i > 0) Hairline(inset = 52.dp)
                ListRow(
                    kind.title,
                    Modifier.testTag("newItem:${kind.name}"),
                    subtitle = kind.detail,
                    glyph = kind.glyph,
                    tint = c.accent,
                    chevron = true,
                    onClick = { onPick(kind) },
                )
            }
        }
        RText(
            "Everything is encrypted on this phone with your vault's key before it reaches the server.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 32.dp, end = 32.dp, top = 16.dp, bottom = 32.dp),
        )
    }
}
