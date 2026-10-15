package dev.reins.android.ui.connections

import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.BrandTile
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.ConnectionAvatar
import dev.reins.android.design.shownIcon
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.KindTile
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.Pill
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalButton
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.main.RootHeader
import dev.reins.android.ui.main.mood
import dev.reins.android.ui.main.numberWord
import dev.reins.android.ui.mcp.mcpStatusLabel
import dev.reins.android.ui.mcp.toolCount
import dev.reins.android.ui.settings.BottomSheet
import dev.reins.android.ui.settings.connectionLine
import dev.reins.android.ui.settings.copyText
import dev.reins.android.ui.signin.AccountRules
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ConnectionView
import dev.reins.core.McpServerView
import dev.reins.core.ServiceView

/**
 * The Connections tab (Reins Signal, iOS-Connections 01 / And-Core 04): both ends of the wire. Who can ask (the AI apps
 * and computers that were in Settings) and what they can reach (the integrations and MCP servers), one list; services
 * not yet connected sit at the bottom as quiet rows. The + opens "Connect something".
 */
@Composable
fun ConnectionsScreen(
    state: AppState,
    serverUrl: String?,
    onConnection: (String) -> Unit,
    /** A service's page ("gmail", "payments", any other id). */
    onService: (String) -> Unit,
    onMcp: (String) -> Unit,
    onAddMcp: () -> Unit,
    onScanComputer: () -> Unit,
    onTypeComputerCode: () -> Unit,
    /** The list is shown: MCP servers refresh their state. */
    onShown: () -> Unit = {},
    /** A sheet covers the page: the tab bar steps aside for it. */
    onCovered: (Boolean) -> Unit = {},
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val context = LocalContext.current
    val connections by state.connections.collectAsStateWithLifecycle()
    val services by state.services.collectAsStateWithLifecycle()
    val mcp by state.mcpServers.collectAsStateWithLifecycle()
    val autopilot by state.autopilot.collectAsStateWithLifecycle()
    var connectOpen by remember { mutableStateOf(false) }
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { onShown() }
    LaunchedEffect(connectOpen) { onCovered(connectOpen) }
    DisposableEffect(Unit) { onDispose { onCovered(false) } }
    val mcpUrl = serverUrl?.let(AccountRules::mcpUrl)
    // AI apps first, then the computers, which pinned their key when they paired.
    val askers = remember(connections) { connections.sortedBy { it.keyFingerprint != null } }
    val (connected, quiet) = remember(services) { services.partition { it.accounts.isNotEmpty() || it.service == "vault" } }

    Box(Modifier.fillMaxSize().background(c.paper).testTag("connections")) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            RootHeader("Connections", mood = mood(null, askersLine(connections))) {
                RoundIconButton(Glyph.Plus, "Connect something", Modifier.testTag("connect")) { connectOpen = true }
            }
            Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(top = 18.dp)) {
                LabelText("Who can ask", c.inkMeta)
                askers.forEach { connection ->
                    AskerRow(connection, autopilot) { onConnection(connection.id) }
                }
                if (askers.isEmpty()) {
                    QuietRow("Connect an AI or a computer", "connectEmpty") { connectOpen = true }
                }
                LabelText("What they can reach", c.inkMeta, Modifier.padding(top = 22.dp))
                connected.forEach { service -> ServiceRow(service) { onService(service.service) } }
                mcp.forEach { server -> McpRow(server) { onMcp(server.id) } }
                quiet.forEach { service -> ServiceRow(service, quiet = true) { onService(service.service) } }
                if (mcp.isEmpty()) {
                    Box(Modifier.testTag("noMcp")) { QuietRow("MCP server", "addMcpRow", glyph = Glyph.Server, onClick = onAddMcp) }
                }
            }
            Spacer(Modifier.height(32.dp).testTag("pageEnd"))
        }

        BottomSheet(connectOpen, "Connect something", onClose = { connectOpen = false }, tag = "connectSheet") {
            ConnectSheetContent(
                mcpUrl = mcpUrl,
                copied = copied,
                onCopy = {
                    if (mcpUrl != null) {
                        copyText(context, "Reins MCP address", mcpUrl)
                        feedback.play(Event.Copied)
                        // Android 13 and later confirm a copy themselves.
                        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) copied = true
                    }
                },
                onScan = {
                    connectOpen = false
                    onScanComputer()
                },
                onType = {
                    connectOpen = false
                    onTypeComputerCode()
                },
                onAddMcp = {
                    connectOpen = false
                    onAddMcp()
                },
            )
        }
    }
}

/** "Three AIs and a computer can ask." */
internal fun askersLine(connections: List<ConnectionView>): String {
    val computers = connections.count { it.keyFingerprint != null }
    val ais = connections.size - computers
    val parts = buildList {
        if (ais > 0) add("${numberWord(ais)} ${if (ais == 1) "AI" else "AIs"}")
        if (computers > 0) add(if (computers == 1) "a computer" else "${numberWord(computers)} computers")
    }
    if (parts.isEmpty()) return "Nothing can ask yet."
    return parts.joinToString(" and ").replaceFirstChar { it.uppercase() } + " can ask."
}

/** An AI app or a computer (its icon: a logo or its blobatar): name, where and when it was last used, its own mode. */
@Composable
private fun AskerRow(connection: ConnectionView, autopilot: AutopilotSettings?, onClick: () -> Unit) {
    val c = LocalColors.current
    val own = autopilot?.connections?.firstOrNull { it.connectionId == connection.id }
    ConnectionRowFrame(Modifier.testTag("connection:${connection.id}"), onClick) {
        androidx.compose.runtime.CompositionLocalProvider(LocalColors provides dev.reins.android.design.RColors.Light) {
            ConnectionAvatar(connection, Modifier.border(1.dp, c.rule, CircleShape).testTag("avatar:${connection.id}:${shownIcon(connection)}"), size = 36.dp)
        }
        Column(Modifier.weight(1f)) {
            RText(untrusted(connection.label), RType.row, c.ink, maxLines = 1)
            RText(connectionLine(connection), RType.meta, c.inkMeta, Modifier.testTag("connectionDetail:${connection.id}"), maxLines = 1, ltr = true)
        }
        val mode = own?.mode?.takeIf { own.baseMode != null || AutopilotText.isTimed(it) }
        when (mode) {
            AutopilotMode.AUTO, AutopilotMode.OVERNIGHT -> Pill(AutopilotText.name(mode), c.violet)
            AutopilotMode.BYPASS -> Pill(AutopilotText.name(mode), c.danger)
            AutopilotMode.LOCKDOWN -> Pill(AutopilotText.name(mode), c.warning)
            AutopilotMode.MANUAL -> Pill(AutopilotText.name(mode), c.inkMeta)
            null -> Unit
        }
        GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 16.dp)
    }
}

/** A service: its mark, its name, its accounts in a word; a quiet one (nothing connected yet) offers Connect. */
@Composable
private fun ServiceRow(service: ServiceView, quiet: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    ConnectionRowFrame(Modifier.testTag("service:${service.service}"), onClick) {
        BrandTile(service.service, size = 36.dp, dim = quiet)
        Column(Modifier.weight(1f)) {
            RText(service.name, RType.row, if (quiet) c.ink2 else c.ink, maxLines = 1)
            serviceLine(service)?.let { RText(it, RType.meta, c.inkMeta, maxLines = 1) }
        }
        if (quiet && service.available) RText("Connect", RType.sans(13.5f, FontWeight.Medium), c.violet, maxLines = 1)
        GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 16.dp)
    }
}

/** What a service's row says under its name: its one account, how many, "This phone"; nothing when none. */
internal fun serviceLine(service: ServiceView): String? = when {
    !service.available -> "Not available"
    service.accounts.isEmpty() -> null
    service.service == "vault" -> null
    service.kind == "device" -> "This phone"
    service.accounts.size == 1 -> untrusted(service.accounts.single().account)
    else -> "${service.accounts.size} accounts"
}

/** An MCP server the user added: "MCP · 12 tools", and a word when it needs something ("Needs sign-in"). */
@Composable
private fun McpRow(server: McpServerView, onClick: () -> Unit) {
    val c = LocalColors.current
    ConnectionRowFrame(Modifier.testTag("mcp:${server.id}"), onClick) {
        BrandTile("mcp", size = 36.dp)
        Column(Modifier.weight(1f)) {
            RText(untrusted(server.name), RType.row, c.ink, maxLines = 1)
            RText("MCP · ${toolCount(server.tools.size)}", RType.meta, c.inkMeta, maxLines = 1)
        }
        if (server.status != "ok") {
            Pill(if (server.status == "needs_sign_in") "Sign in" else mcpStatusLabel(server.status), if (server.status == "needs_sign_in") c.warning else c.danger, Modifier.testTag("mcpStatus:${server.id}").semantics(mergeDescendants = true) {})
        }
        GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 16.dp)
    }
}

/** A quiet row that offers to add something, in violet. */
@Composable
private fun QuietRow(title: String, tag: String, glyph: Glyph = Glyph.Plus, onClick: () -> Unit) {
    val c = LocalColors.current
    ConnectionRowFrame(Modifier.testTag(tag), onClick) {
        Box(Modifier.size(36.dp).clip(RoundedCornerShape(11.dp)).background(c.paperSunken), contentAlignment = Alignment.Center) {
            GlyphIcon(glyph, c.ink2, size = 19.dp)
        }
        RText(title, RType.row, c.ink2, Modifier.weight(1f), maxLines = 1)
        RText("Add", RType.sans(13.5f, FontWeight.Medium), c.violet, maxLines = 1)
    }
}

/** A hairline row of this list (min 56, 12 between its parts). */
@Composable
private fun ConnectionRowFrame(modifier: Modifier, onClick: () -> Unit, content: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .hairlineBelow(c.rule)
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

/**
 * Connect something: an AI app by the address to paste (copied here), a computer by the code it shows, or an MCP
 * server by its address. Settings' connect sheet, moved here.
 */
@Composable
private fun ConnectSheetContent(mcpUrl: String?, copied: Boolean, onCopy: () -> Unit, onScan: () -> Unit, onType: () -> Unit, onAddMcp: () -> Unit) {
    val c = LocalColors.current
    var help by remember { mutableStateOf(false) }
    Column(Modifier.padding(horizontal = 24.dp)) {
        Column(Modifier.fillMaxWidth().hairlineBelow(c.rule).padding(vertical = 14.dp).testTag("connectAi")) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Box(Modifier.width(56.dp)) {
                    Box(Modifier.padding(start = 22.dp)) { dev.reins.android.design.ConnectionAvatar("ChatGPT", "openai", size = 34.dp) }
                    Box(Modifier.border(2.5.dp, c.paperRaised, CircleShape)) { dev.reins.android.design.ConnectionAvatar("Claude", "claude", size = 34.dp) }
                }
                Column(Modifier.weight(1f)) {
                    RText("AI app", RType.row, c.ink, maxLines = 1)
                    RText("Claude, ChatGPT and others", RType.meta, c.inkMeta, maxLines = 1)
                }
                dev.reins.android.ui.settings.HelpIcon(help, "AI apps") { help = it }
            }
            dev.reins.android.design.HelpText(
                "In Claude.ai, ChatGPT or another AI app, add a custom connector (MCP) and paste this address. It then asks you here before it reads or sends anything.",
                help,
                Modifier.padding(top = 10.dp),
            )
            if (mcpUrl != null) {
                Row(
                    Modifier
                        .padding(top = 12.dp)
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(14.dp))
                        .background(c.paperSunken)
                        .pressable(shape = RoundedCornerShape(14.dp), label = "Copy MCP address", onClick = onCopy)
                        .padding(start = 14.dp, end = 12.dp, top = 12.dp, bottom = 12.dp)
                        .testTag("copyMcpUrl"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RText(mcpUrl.removePrefix("https://"), RType.value, c.ink, Modifier.weight(1f), maxLines = 1, ltr = true)
                    Spacer(Modifier.width(10.dp))
                    GlyphIcon(if (copied) Glyph.Check else Glyph.Copy, c.violet, size = 16.dp)
                    Spacer(Modifier.width(6.dp))
                    RText(if (copied) "Copied" else "Copy", RType.sans(14f, FontWeight.SemiBold), c.violet, maxLines = 1)
                }
            }
        }
        Column(Modifier.fillMaxWidth().hairlineBelow(c.rule).padding(vertical = 14.dp).testTag("connectComputerCard")) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                KindTile(Field.Pair, Glyph.Laptop, size = 36.dp)
                Column(Modifier.weight(1f)) {
                    RText("Computer", RType.row, c.ink, maxLines = 1)
                    Row {
                        RText("Desktop app or ", RType.meta, c.inkMeta, maxLines = 1)
                        RText("reins login", RType.mono(12.5f), c.inkMeta, maxLines = 1, ltr = true)
                    }
                }
            }
            Row(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SignalButton("Scan QR code", Modifier.weight(1f).testTag("sheetScanQr"), size = ButtonSize.Small, glyph = Glyph.QrCode, onClick = onScan)
                SignalButton("Type code", Modifier.testTag("sheetTypeCode"), style = ButtonStyle.Soft, size = ButtonSize.Small, onClick = onType)
            }
        }
        Row(
            Modifier
                .fillMaxWidth()
                .heightIn(min = 56.dp)
                .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onAddMcp)
                .testTag("addMcp")
                .padding(vertical = 10.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            BrandTile("mcp", size = 36.dp)
            Column(Modifier.weight(1f)) {
                RText("MCP server", RType.row, c.ink, maxLines = 1)
                RText("Its tools, through Reins", RType.meta, c.inkMeta, maxLines = 1)
            }
            GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 16.dp)
        }
    }
}
