package dev.reins.android.ui.connections

import androidx.compose.ui.draw.drawBehind
import dev.reins.android.design.PaneRow
import dev.reins.android.design.PaneGroup
import dev.reins.android.design.SectionHeader
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
import androidx.compose.foundation.layout.widthIn
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
    val hidden by state.connectionsHidden.collectAsStateWithLifecycle()
    val services by state.services.collectAsStateWithLifecycle()
    val mcp by state.mcpServers.collectAsStateWithLifecycle()
    val autopilot by state.autopilot.collectAsStateWithLifecycle()
    var connectOpen by remember { mutableStateOf(false) }
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { onShown() }
    LaunchedEffect(connectOpen) { onCovered(connectOpen) }
    DisposableEffect(Unit) { onDispose { onCovered(false) } }
    val mcpUrl = serverUrl?.let(AccountRules::mcpUrl)
    // Next (Connections 01): AI apps, computers (they pinned a key when they paired) and services, each its own group
    // under a header whose + adds to it.
    val ais = remember(connections) { connections.filter { it.keyFingerprint == null } }
    val computers = remember(connections) { connections.filter { it.keyFingerprint != null } }
    val (connected, quiet) = remember(services) { services.partition { it.accounts.isNotEmpty() || it.service == "vault" } }
    var servicesOpen by remember { mutableStateOf(false) }
    var allAis by remember { mutableStateOf(false) }
    var aisHelp by remember { mutableStateOf(false) }
    var computersHelp by remember { mutableStateOf(false) }
    LaunchedEffect(servicesOpen) { onCovered(connectOpen || servicesOpen) }

    Box(Modifier.fillMaxSize().background(c.paper).testTag("connections")) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            RootHeader("Connections")
            // Not this phone's to read (it neither approves nor views): where they are, never "none" and no count.
            SectionHeader("AI apps", glyph = Glyph.Globe, count = if (hidden) null else ais.size.toString(), onAdd = { connectOpen = true }, addLabel = "Connect something", addTag = "connect")
            PaneGroup {
                val shown = if (allAis || ais.size <= AI_ROWS + 1) ais else ais.take(AI_ROWS)
                shown.forEachIndexed { i, connection -> AskerRow(connection, autopilot, first = i == 0) { onConnection(connection.id) } }
                if (shown.size < ais.size) {
                    PaneRow("Show all", value = ais.size.toString(), chevron = true, tag = "showAllAis") { allAis = true }
                }
                if (hidden) {
                    HiddenRow("aisHidden", aisHelp) { aisHelp = it }
                } else if (ais.isEmpty()) {
                    QuietRow("Connect an AI app", "connectEmpty", first = true) { connectOpen = true }
                }
            }
            HiddenHelp(hidden && aisHelp)
            SectionHeader("Computers", glyph = Glyph.Laptop, count = if (hidden) null else computers.size.toString(), onAdd = { connectOpen = true }, addLabel = "Connect a computer", addTag = "connectComputer")
            PaneGroup {
                computers.forEachIndexed { i, connection -> AskerRow(connection, autopilot, first = i == 0) { onConnection(connection.id) } }
                if (hidden) {
                    HiddenRow("computersHidden", computersHelp) { computersHelp = it }
                } else if (computers.isEmpty()) {
                    QuietRow("Connect a computer", "connectComputerEmpty", first = true) { connectOpen = true }
                }
            }
            HiddenHelp(hidden && computersHelp)
            SectionHeader("Services", glyph = Glyph.Link, count = (connected.size + mcp.size).toString(), onAdd = { servicesOpen = true }, addLabel = "Add a service", addTag = "addService")
            PaneGroup {
                connected.forEachIndexed { i, service -> ServiceRow(service, first = i == 0) { onService(service.service) } }
                mcp.forEachIndexed { i, server -> McpRow(server, first = connected.isEmpty() && i == 0) { onMcp(server.id) } }
                // An MCP server is added from here until there is one (then from the +).
                if (mcp.isEmpty()) {
                    Box(Modifier.testTag("noMcp")) { QuietRow("MCP server", "addMcpRow", glyph = Glyph.Server, first = connected.isEmpty(), onClick = onAddMcp) }
                }
            }
            Spacer(Modifier.height(32.dp).testTag("pageEnd"))
        }

        // The services not connected yet, and an MCP server, behind the Services header's +.
        BottomSheet(servicesOpen, "Add", onClose = { servicesOpen = false }, tag = "servicesSheet") {
            PaneGroup {
                quiet.forEachIndexed { i, service ->
                    ServiceRow(service, quiet = true, first = i == 0) {
                        servicesOpen = false
                        onService(service.service)
                    }
                }
                QuietRow("MCP server", "addMcpSheet", glyph = Glyph.Server, first = quiet.isEmpty()) {
                    servicesOpen = false
                    onAddMcp()
                }
            }
            Spacer(Modifier.height(8.dp))
        }

        BottomSheet(connectOpen, "Connect", onClose = { connectOpen = false }, tag = "connectSheet") {
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

/** How many AI apps show before "Show all". */
private const val AI_ROWS = 4

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
private fun AskerRow(connection: ConnectionView, autopilot: AutopilotSettings?, first: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    val own = autopilot?.connections?.firstOrNull { it.connectionId == connection.id }
    ConnectionRowFrame(Modifier.testTag("connection:${connection.id}"), first, onClick) {
        androidx.compose.runtime.CompositionLocalProvider(LocalColors provides dev.reins.android.design.RColors.Light) {
            ConnectionAvatar(connection, Modifier.border(1.dp, c.rule, CircleShape).testTag("avatar:${connection.id}:${shownIcon(connection)}"), size = 32.dp)
        }
        Column(Modifier.weight(1f)) {
            RText(untrusted(connection.label), RType.row, c.ink, maxLines = 1)
            RText(connectionLine(connection), RType.meta, c.inkMeta, Modifier.testTag("connectionDetail:${connection.id}"), maxLines = 1, ltr = true)
        }
        val mode = own?.mode?.takeIf { own.baseMode != null || AutopilotText.isTimed(it) }
        when (mode) {
            AutopilotMode.AUTO -> dev.reins.android.ui.activity.OutcomeTag(AutopilotText.name(mode), Glyph.AutopilotFilled, c.violet)
            AutopilotMode.OVERNIGHT -> dev.reins.android.ui.activity.OutcomeTag(AutopilotText.name(mode), Glyph.Moon, c.violet)
            AutopilotMode.BYPASS -> dev.reins.android.ui.activity.OutcomeTag(AutopilotText.name(mode), Glyph.Bypass, c.danger)
            AutopilotMode.LOCKDOWN -> dev.reins.android.ui.activity.OutcomeTag(AutopilotText.name(mode), Glyph.Block, c.warning)
            AutopilotMode.MANUAL -> dev.reins.android.ui.activity.OutcomeTag(AutopilotText.name(mode), Glyph.Person, c.inkMeta)
            null -> Unit
        }
        GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 18.dp)
    }
}

/** A service: its mark, its name, its accounts in a word; a quiet one (nothing connected yet) offers Connect. */
@Composable
private fun ServiceRow(service: ServiceView, quiet: Boolean = false, first: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    ConnectionRowFrame(Modifier.testTag("service:${service.service}"), first, onClick) {
        BrandTile(service.service, size = 32.dp, dim = quiet && !service.available)
        RText(service.name, RType.row, c.ink, Modifier.weight(1f), maxLines = 1)
        // Next: the account on the right ("octo-cat"), quiet.
        serviceLine(service)?.let { RText(it, RType.sans(15f), c.inkMeta, Modifier.widthIn(max = 160.dp), maxLines = 1, ltr = true) }
        if (quiet && service.available) RText("Connect", RType.sans(14f, FontWeight.SemiBold), c.violet, maxLines = 1)
        GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 18.dp)
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
private fun McpRow(server: McpServerView, first: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    ConnectionRowFrame(Modifier.testTag("mcp:${server.id}"), first, onClick) {
        BrandTile("mcp", size = 32.dp)
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

/**
 * In place of a group's connections when the server keeps them from this phone: they are on the approving phone (a
 * phone sees them once that phone enrolls it as a viewer). The (?) says why.
 */
@Composable
private fun HiddenRow(tag: String, help: Boolean, onHelp: (Boolean) -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag(tag).padding(start = 14.dp, end = 6.dp, top = 8.dp, bottom = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(32.dp).clip(RoundedCornerShape(9.dp)).background(c.paperSunken), contentAlignment = Alignment.Center) {
            GlyphIcon(Glyph.Phone, c.ink2, size = 18.dp)
        }
        RText(NOT_A_VIEWER, RType.row, c.ink2, Modifier.weight(1f), maxLines = 2)
        dev.reins.android.ui.settings.HelpIcon(help, tag) { onHelp(it) }
    }
}

@Composable
private fun HiddenHelp(open: Boolean) {
    dev.reins.android.design.HelpText(NOT_A_VIEWER_HELP, open, Modifier.padding(horizontal = 24.dp).padding(top = 6.dp))
}

/** Where a phone that neither approves nor views finds the AI apps and computers. */
internal const val NOT_A_VIEWER = "Visible on the approving phone"

/** Why ([NOT_A_VIEWER]'s (?)). */
internal const val NOT_A_VIEWER_HELP =
    "Only the phone that approves, and phones it adds as viewers, can see what is connected. This phone is signed in but is not one of them."

/** A quiet row that offers to add something, in violet. */
@Composable
private fun QuietRow(title: String, tag: String, glyph: Glyph = Glyph.Plus, first: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    ConnectionRowFrame(Modifier.testTag(tag), first, onClick) {
        Box(Modifier.size(32.dp).clip(RoundedCornerShape(9.dp)).background(c.paperSunken), contentAlignment = Alignment.Center) {
            GlyphIcon(glyph, c.ink2, size = 18.dp)
        }
        RText(title, RType.row, c.ink2, Modifier.weight(1f), maxLines = 1)
        RText("Add", RType.sans(13.5f, FontWeight.Medium), c.violet, maxLines = 1)
    }
}

/** A row of a group (`.r`: min 56, 12 between its parts, a hairline above unless [first]). */
@Composable
private fun ConnectionRowFrame(modifier: Modifier, first: Boolean, onClick: () -> Unit, content: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .then(if (!first) Modifier.drawBehind { drawLine(c.rule, androidx.compose.ui.geometry.Offset(58.dp.toPx(), 0f), androidx.compose.ui.geometry.Offset(size.width, 0f), 1.dp.toPx()) } else Modifier)
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 8.dp),
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
    // Next (Pair and join 01): the address an AI app pastes, a computer's Scan or Type, and an MCP server.
    SectionHeader("AI app", glyph = Glyph.Link, quiet = true, trailing = { dev.reins.android.ui.settings.HelpIcon(help, "AI apps") { help = it } })
    dev.reins.android.design.HelpText(
        "In Claude.ai, ChatGPT or another AI app, add a custom connector (MCP) and paste this address. It then asks you here before it reads or sends anything.",
        help,
        Modifier.padding(horizontal = 24.dp).padding(bottom = 8.dp),
    )
    PaneGroup(Modifier.testTag("connectAi")) {
        if (mcpUrl != null) {
            Row(
                Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(start = 14.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RText(mcpUrl.removePrefix("https://"), RType.mono(14f), c.ink, Modifier.weight(1f), maxLines = 1, ltr = true)
                Box(
                    Modifier.size(40.dp).clip(CircleShape).background(c.paperRaised).border(1.dp, c.rule, CircleShape)
                        .pressable(shape = CircleShape, label = "Copy MCP address", onClick = onCopy).testTag("copyMcpUrl"),
                    contentAlignment = Alignment.Center,
                ) { GlyphIcon(if (copied) Glyph.Check else Glyph.Copy, c.ink, size = 18.dp) }
            }
        }
    }
    SectionHeader("Computer", glyph = Glyph.Laptop, quiet = true)
    PaneGroup(Modifier.testTag("connectComputerCard")) {
        Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SignalButton("Scan", Modifier.weight(1f).testTag("sheetScanQr"), size = ButtonSize.Small, glyph = Glyph.QrCode, onClick = onScan)
            SignalButton("Type", Modifier.weight(1f).testTag("sheetTypeCode"), style = ButtonStyle.Soft, size = ButtonSize.Small, glyph = Glyph.Pencil, onClick = onType)
        }
    }
    SectionHeader("MCP server", glyph = Glyph.Server, quiet = true)
    PaneGroup {
        PaneRow("Add a server", leading = { BrandTile("mcp", size = 32.dp) }, chevron = true, first = true, tag = "addMcp", onClick = onAddMcp)
    }
    Spacer(Modifier.height(8.dp))
}
