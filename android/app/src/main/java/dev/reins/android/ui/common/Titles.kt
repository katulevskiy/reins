package dev.reins.android.ui.common

import dev.reins.android.design.ActionKind
import dev.reins.android.design.MCP_PREFIX
import dev.reins.android.design.McpNames
import dev.reins.android.design.serviceName

/**
 * "Search Gmail", "Read 3 emails", "Send email to 2": what an operation is, in a few words. [title] is the name the
 * core gives an operation of another integration ("Read Telegram messages"); without it a generic one is made up.
 * A call to an MCP server's tool has no title from the core: it reads as the tool's title ([op] is the tool's name).
 */
fun operationTitle(action: String, count: Int, service: String, title: String = "", op: String = ""): String {
    if (title.isNotBlank()) return title
    if (service.startsWith(MCP_PREFIX)) return mcpTitle(service.removePrefix(MCP_PREFIX), op)
    val what = serviceName(service.ifBlank { "gmail" })
    val mail = service.isBlank() || service == "gmail"
    return when (ActionKind.of(action)) {
        ActionKind.Search -> "Search $what"
        ActionKind.List -> "List $what"
        ActionKind.Write -> "Change $what"
        ActionKind.Read -> if (!mail) "Read $what" else when (count) {
            0 -> "Read email"
            1 -> "Read 1 email"
            else -> "Read $count emails"
        }
        ActionKind.Send -> if (!mail) "Send with $what" else if (count > 1) "Send email to $count" else "Send email"
        ActionKind.Grant -> "Ask for access"
        ActionKind.Accounts -> if (service.isBlank()) "See integrations" else "See $what accounts"
        ActionKind.Pair -> "Connect"
        ActionKind.Join -> "Add a phone"
        ActionKind.Upload -> "Share a file"
        ActionKind.Other -> "Request"
    }
}

/** "Create issue" (the tool's title), "Use create_issue" (a tool the app does not know), "Use a tool of Linear". */
private fun mcpTitle(serverId: String, tool: String): String {
    if (tool.isBlank()) return "Use a tool of ${serviceName(MCP_PREFIX + serverId)}"
    return McpNames.tool(serverId, tool)?.let(::untrusted)?.takeIf { it.isNotBlank() } ?: "Use ${untrusted(tool)}"
}

/** "Claude: Search Gmail". */
fun fullTitle(label: String, action: String, count: Int, service: String, title: String = "", op: String = ""): String =
    "${untrusted(label)}: ${operationTitle(action, count, service, title, op)}"

/** The headline of an activity entry; a permission reads as what happened to it. */
fun entryTitle(label: String, action: String, count: Int, service: String, outcome: String, title: String = "", op: String = ""): String {
    val what = if (ActionKind.of(action) == ActionKind.Grant) {
        when (outcome) {
            "granted" -> "Access granted"
            "denied" -> "Access refused"
            else -> "Access request"
        }
    } else {
        operationTitle(action, count, service, title, op)
    }
    return "${untrusted(label)}: $what"
}
