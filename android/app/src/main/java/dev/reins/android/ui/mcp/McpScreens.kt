package dev.reins.android.ui.mcp

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.Glyph
import dev.reins.android.design.InlineHelp
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.ServiceAvatar
import dev.reins.android.design.Tag
import dev.reins.android.design.Toggle
import dev.reins.android.design.pressable
import dev.reins.android.platform.Browser
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.untrusted
import dev.reins.core.McpServerView
import dev.reins.core.McpToolView

private fun statusTint(status: String, c: RColors): Color = when (status) {
    "ok" -> c.success
    "needs_sign_in" -> c.warning
    else -> c.danger
}

@Composable
private fun StatusChip(status: String, tag: String) {
    Tag(mcpStatusLabel(status), Modifier.testTag(tag).semantics(mergeDescendants = true) {}, tint = statusTint(status, LocalColors.current))
}

@Composable
private fun rememberPageOpener(): PageOpener {
    val context = LocalContext.current
    return remember(context) { { url: String -> Browser.open(context, url) } }
}

// ---- the list (part of the Integrations screen) -----------------------------------------------------------------

/** The MCP servers the user added, under the integrations, with the way to add another. */
@Composable
fun McpServersSection(servers: List<McpServerView>, onOpen: (String) -> Unit, onAdd: () -> Unit) {
    val c = LocalColors.current
    Group(
        header = "MCP servers",
        footer = "Tools of the MCP servers you add here are offered to your AIs through Reins. You approve each call, the way you approve everything else.",
    ) {
        if (servers.isEmpty()) {
            Column(Modifier.padding(16.dp).testTag("noMcp")) {
                RText("No MCP servers yet", RType.sans(16f, FontWeight.Medium), c.text)
                RText(
                    "Add one by its address, like https://mcp.linear.app/mcp.",
                    RType.sans(13f, lineHeight = 18f),
                    c.secondary,
                    Modifier.padding(top = 2.dp),
                )
            }
        }
        servers.forEachIndexed { i, server ->
            if (i > 0) Hairline(inset = 74.dp)
            McpServerRow(server) { onOpen(server.id) }
        }
    }
    CapsuleButton(
        "Add MCP server",
        Modifier.padding(horizontal = 16.dp, vertical = 12.dp).fillMaxWidth().testTag("addMcp"),
        style = ButtonStyle.Secondary,
        glyph = Glyph.Plus,
        onClick = onAdd,
    )
}

@Composable
private fun McpServerRow(server: McpServerView, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .testTag("mcp:${server.id}")
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ServiceAvatar("mcp", size = 44.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            RText(untrusted(server.name), RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1)
            RText(
                mcpHost(server.url) + " · " + toolCount(server.tools.size),
                RType.sans(13f),
                c.secondary,
                maxLines = 1,
                ltr = true,
                overflow = TextOverflow.MiddleEllipsis,
            )
        }
        Spacer(Modifier.width(10.dp))
        StatusChip(server.status, "mcpStatus:${server.id}")
        Spacer(Modifier.width(6.dp))
        GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp)
    }
}

// ---- adding ---------------------------------------------------------------------------------------------------------

/** A server by its address; a name and an access token are optional. */
@Composable
fun McpAddScreen(viewModel: McpViewModel, onBack: () -> Unit, onAdded: (String) -> Unit) {
    val c = LocalColors.current
    val busy by viewModel.busy.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()
    val signingIn by viewModel.signingIn.collectAsStateWithLifecycle()
    val open = rememberPageOpener()
    var url by remember { mutableStateOf("") }
    var name by remember { mutableStateOf("") }
    var token by remember { mutableStateOf("") }
    LaunchedEffect(Unit) { viewModel.clearError() }

    Screen(title = "Add MCP server", onBack = onBack) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            RTextField(
                url, { url = it }, "https://mcp.example.com/mcp", tag = "mcpUrl", enabled = !busy, mono = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
            )
            RTextField(name, { name = it }, "Name (optional)", tag = "mcpName", enabled = !busy)
            RTextField(
                token, { token = it }, "Access token (optional)", tag = "mcpToken", enabled = !busy, password = true, mono = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
            )
            InlineHelp(
                "MCP servers",
                "Paste the address the service gives for its MCP server. Reins connects from this phone; if the server wants you to sign in, its page opens here.\n\n" +
                    "The token is only for servers that give you one instead of a sign-in. It is kept encrypted on this phone.",
            )
            CapsuleButton(
                "Add",
                Modifier.fillMaxWidth().padding(top = 6.dp).testTag("mcpAddSubmit"),
                enabled = !busy && url.isNotBlank(),
                busy = busy,
            ) {
                val sent = token
                token = ""
                viewModel.add(url, name, sent, open, onAdded)
            }
            if (signingIn != null) {
                Banner(
                    "Finish signing in on the page that opened.",
                    Modifier.padding(top = 6.dp),
                    tag = "mcpSigningIn",
                )
            }
            error?.let { Banner(untrusted(it), Modifier.padding(top = 6.dp), BannerKind.Error, tag = "mcpError") }
        }
    }
}

// ---- one server ---------------------------------------------------------------------------------------------------

/** One server: whether it works, its tools and what each may do, and the ways to sign in again or remove it. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun McpServerScreen(viewModel: McpViewModel, state: AppState, id: String, onBack: () -> Unit, onRemoved: () -> Unit) {
    val c = LocalColors.current
    val servers by state.mcpServers.collectAsStateWithLifecycle()
    val notice by state.mcpNotice.collectAsStateWithLifecycle()
    val busy by viewModel.busy.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()
    val signingIn by viewModel.signingIn.collectAsStateWithLifecycle()
    val open = rememberPageOpener()
    var removing by remember { mutableStateOf(false) }
    LaunchedEffect(id) { viewModel.clearError() }
    // How a sign-in ended is shown while this page is open, and only once.
    DisposableEffect(id) { onDispose { if (state.mcpNotice.value?.serverId == id) state.setMcpNotice(null) } }

    val server = servers.firstOrNull { it.id == id }
    if (server == null) {
        Screen(title = "MCP server", onBack = onBack) {
            Banner("This server is no longer added.", Modifier.padding(16.dp), BannerKind.Warning, tag = "mcpGone")
        }
        return
    }
    Screen(title = untrusted(server.name), subtitle = mcpHost(server.url), modifier = Modifier.testTag("mcpDetail"), onBack = onBack) {
        notice?.takeIf { it.serverId == id }?.let {
            Banner(untrusted(it.text), Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), if (it.failed) BannerKind.Error else BannerKind.Notice, tag = "mcpNotice")
        }
        Group(header = "Server") {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ServiceAvatar("mcp", size = 40.dp)
                    Spacer(Modifier.width(12.dp))
                    RText(untrusted(server.url), RType.mono(13.5f), c.secondary, Modifier.weight(1f), maxLines = 2, ltr = true)
                    Spacer(Modifier.width(8.dp))
                    StatusChip(server.status, "mcpStatus")
                }
                server.error?.takeIf { it.isNotBlank() }?.let {
                    RText(untrusted(it), RType.sans(14f, lineHeight = 20f), c.danger, Modifier.testTag("mcpServerError"))
                }
                if (server.status == "needs_sign_in") {
                    RText(
                        "Sign in to use its tools.",
                        RType.sans(14f, lineHeight = 20f),
                        c.secondary,
                    )
                    CapsuleButton("Sign in", Modifier.fillMaxWidth().padding(top = 6.dp).testTag("mcpSignIn"), enabled = !busy, busy = busy) {
                        viewModel.refresh(id, open)
                    }
                }
            }
        }
        if (signingIn == id) {
            Banner(
                "Finish signing in on the page that opened.",
                Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp),
                tag = "mcpSigningIn",
            )
        }
        error?.let { Banner(untrusted(it), Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), BannerKind.Error, tag = "mcpError") }
        Group(
            header = "Tools (${server.tools.size})",
            footer = "Your AIs can ask to use these tools. Tools that only read need a read permission; anything else is a change you approve. Large results go through your Reins server instead of this phone.",
        ) {
            if (server.tools.isEmpty()) {
                RText(
                    if (server.status == "ok") "This server has no tools." else "No tools yet.",
                    RType.sans(14.5f),
                    c.secondary,
                    Modifier.padding(16.dp).testTag("noTools"),
                )
            }
            server.tools.forEachIndexed { i, tool ->
                if (i > 0) Hairline()
                ToolRow(tool, enabled = !busy) { heavy -> viewModel.setHeavy(id, tool.name, heavy) }
            }
        }
        Row(
            Modifier.padding(horizontal = 16.dp, vertical = 18.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            CapsuleButton("Refresh", Modifier.weight(1f).testTag("mcpRefresh"), style = ButtonStyle.Secondary, enabled = !busy, glyph = Glyph.Refresh) {
                viewModel.refresh(id, open)
            }
            CapsuleButton("Remove server", Modifier.weight(1f).testTag("mcpRemove"), style = ButtonStyle.Destructive, enabled = !busy, glyph = Glyph.Trash) {
                removing = true
            }
        }
    }
    if (removing) {
        ConfirmDialog(
            title = "Remove ${untrusted(server.name)}?",
            text = "Its tools, sign-in and permissions go.",
            confirmLabel = "Remove",
            onConfirm = {
                removing = false
                viewModel.remove(id, onRemoved)
            },
            onDismiss = { removing = false },
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ToolRow(tool: McpToolView, enabled: Boolean, onHeavy: (Boolean) -> Unit) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().testTag("tool:${tool.name}").padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        RText(untrusted(tool.title).ifEmpty { tool.name }, RType.sans(16f, FontWeight.SemiBold), c.text, maxLines = 2)
        if (tool.title != tool.name) RText(untrusted(tool.name), RType.mono(12.5f), c.tertiary, maxLines = 1, ltr = true)
        if (tool.description.isNotBlank()) {
            RText(untrusted(tool.description), RType.sans(13.5f, lineHeight = 19f), c.secondary, maxLines = 4)
        }
        FlowRow(Modifier.padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            toolBadges(tool).forEach { badge ->
                Tag(
                    badge.label,
                    Modifier.testTag("badge:${tool.name}:${badge.key}"),
                    tint = when (badge) {
                        ToolBadge.ReadOnly -> c.success
                        ToolBadge.Changes -> c.send
                        ToolBadge.AsksEveryTime -> c.danger
                        ToolBadge.Heavy -> c.accent
                    },
                )
            }
        }
        Row(Modifier.padding(top = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            RText("Large results via server", RType.sans(13.5f), c.secondary, Modifier.weight(1f))
            Spacer(Modifier.width(10.dp))
            Toggle(tool.heavy, Modifier.testTag("heavy:${tool.name}"), enabled = enabled, onChange = onHeavy)
        }
    }
}
