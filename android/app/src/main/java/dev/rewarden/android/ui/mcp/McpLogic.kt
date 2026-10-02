package dev.rewarden.android.ui.mcp

import dev.rewarden.core.McpToolView
import java.net.URI

/** Where an MCP server's sign-in page sends the browser back to (the core registers it with the server). */
const val MCP_REDIRECT_SCHEME = "dev.rewarden.android"
const val MCP_REDIRECT_HOST = "mcp-oauth"

/** Longest redirect address passed on to the core; a real one is a code and a state. */
private const val MAX_REDIRECT_CHARS = 8_192

/** The host (and port) of a server's address, as the list shows it; never its path or query. */
fun mcpHost(url: String): String = try {
    val uri = URI(url.trim())
    val host = uri.host
    if (host.isNullOrEmpty()) url else if (uri.port != -1) "$host:${uri.port}" else host
} catch (e: java.net.URISyntaxException) {
    url
}

fun mcpStatusLabel(status: String): String = when (status) {
    "ok" -> "Connected"
    "needs_sign_in" -> "Needs sign-in"
    else -> "Error"
}

fun toolCount(n: Int): String = when (n) {
    0 -> "No tools"
    1 -> "1 tool"
    else -> "$n tools"
}

/** What a tool's badges say about it. */
enum class ToolBadge(val key: String, val label: String) {
    ReadOnly("readOnly", "Read only"),
    Changes("changes", "Changes things"),
    AsksEveryTime("asksEveryTime", "Asks every time"),
    Heavy("heavy", "Large results"),
}

fun toolBadges(tool: McpToolView): List<ToolBadge> = buildList {
    add(if (tool.readOnly) ToolBadge.ReadOnly else ToolBadge.Changes)
    if (tool.destructive) add(ToolBadge.AsksEveryTime)
    if (tool.heavy) add(ToolBadge.Heavy)
}

/** The sign-in page sent the browser to the app's own redirect address (anything else is never handed to the core). */
fun isMcpRedirect(uri: String?): Boolean {
    if (uri == null || uri.length > MAX_REDIRECT_CHARS) return false
    return try {
        val parsed = URI(uri)
        parsed.scheme == MCP_REDIRECT_SCHEME && parsed.rawAuthority == MCP_REDIRECT_HOST
    } catch (e: java.net.URISyntaxException) {
        false
    }
}

/** A sign-in page may only be a web page (never an `intent:` or `file:` address). */
fun webPage(url: String): Boolean = try {
    val uri = URI(url)
    (uri.scheme == "https" || uri.scheme == "http") && !uri.host.isNullOrEmpty()
} catch (e: java.net.URISyntaxException) {
    false
}
