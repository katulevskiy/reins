package dev.rewarden.android.ui.gmail

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.Hairline
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen
import dev.rewarden.android.design.ServiceAvatar
import dev.rewarden.android.design.pressable
import dev.rewarden.android.state.AppState
import dev.rewarden.android.ui.mcp.McpServersSection
import dev.rewarden.core.ServiceView

/** Every service Rewarden can connect, with how many accounts each one has, and the MCP servers the user added. */
@Composable
fun IntegrationsScreen(
    state: AppState,
    onBack: () -> Unit,
    onOpen: (String) -> Unit,
    onOpenMcp: (String) -> Unit = {},
    onAddMcp: () -> Unit = {},
    onShown: () -> Unit = {},
) {
    val c = LocalColors.current
    val services by state.services.collectAsStateWithLifecycle()
    val mcp by state.mcpServers.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { onShown() }
    Screen(title = "Integrations", onBack = onBack) {
        Group(
            header = "Services",
            footer = "Each service can have as many accounts as you like. Your AIs can see which services and accounts exist and choose one; you approve what they do with it.",
        ) {
            services.forEachIndexed { i, service ->
                if (i > 0) Hairline(inset = 74.dp)
                ServiceRow(service) { onOpen(service.service) }
            }
        }
        McpServersSection(mcp, onOpen = onOpenMcp, onAdd = onAddMcp)
    }
}

@Composable
private fun ServiceRow(service: ServiceView, onClick: () -> Unit) {
    val c = LocalColors.current
    val count = service.accounts.size
    Row(
        Modifier
            .fillMaxWidth()
            .testTag("service:${service.service}")
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ServiceAvatar(service.service, size = 44.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(service.name, RType.sans(16f, FontWeight.Medium), c.text)
            RText(
                when {
                    !service.available -> "Not available"
                    count == 0 -> "Not connected"
                    count == 1 -> "1 account"
                    else -> "$count accounts"
                },
                RType.sans(13f),
                if (count == 0) c.tertiary else c.secondary,
                Modifier.padding(top = 2.dp),
            )
        }
        GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp)
    }
}
