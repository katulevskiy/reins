package dev.reins.android.design

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.reins.core.McpServerView

/** Which connector and account an operation concerns, as small tags. [name] overrides the service's name. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ConnectorTags(
    service: String,
    account: String?,
    modifier: Modifier = Modifier,
    extra: List<String> = emptyList(),
    name: String? = null,
) {
    val c = LocalColors.current
    FlowRow(modifier, horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (service.isNotBlank()) Tag(name ?: serviceName(service), tint = c.accent)
        if (!account.isNullOrBlank()) Tag(account, mono = true)
        extra.forEach { Tag(it) }
    }
}

fun serviceName(service: String): String = when {
    service.startsWith(MCP_PREFIX) -> McpNames.server(service.removePrefix(MCP_PREFIX)) ?: "MCP server"
    else -> when (service) {
        "gmail" -> "Gmail"
        "telegram" -> "Telegram"
        "gcalendar" -> "Google Calendar"
        "gcontacts" -> "Google Contacts"
        "device_calendar" -> "Phone calendar"
        "device_contacts" -> "Phone contacts"
        "sms" -> "Text messages"
        "github" -> "GitHub"
        "gitlab" -> "GitLab"
        "codeberg" -> "Codeberg"
        "bitbucket" -> "Bitbucket"
        "desktop" -> "Desktop app"
        "files" -> "Files"
        "vault" -> "Password vault"
        else -> service.replaceFirstChar { it.uppercase() }
    }
}

/** The service id of an MCP server's calls: `mcp:<server id>`. */
const val MCP_PREFIX = "mcp:"

/**
 * The names of the MCP servers the user added and of their tools, so that lists and notifications (which only carry
 * `mcp:<id>` and the tool's name) can say "Linear" and "Create issue". Kept up to date from the app state.
 */
object McpNames {
    @Volatile private var servers: Map<String, McpServerView> = emptyMap()

    fun update(list: List<McpServerView>) {
        servers = list.associateBy { it.id }
    }

    fun server(id: String): String? = servers[id]?.name

    /** A tool's title, when the server and the tool are known. */
    fun tool(serverId: String, tool: String): String? = servers[serverId]?.tools?.firstOrNull { it.name == tool }?.title
}
