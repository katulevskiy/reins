package dev.reins.android.ui.approval

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.Card
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.ServiceAvatar
import dev.reins.android.design.Tag
import dev.reins.android.ui.common.FileCard
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.mcp.mcpHost
import dev.reins.core.AskView
import dev.reins.core.BlobView
import dev.reins.core.McpCallView
import dev.reins.core.SecretReleaseView
import dev.reins.core.SshSignView

/** "for 30 minutes", "for 1 hour", "for 1 h 30 min": how long the desktop app may keep secrets. */
fun leaseLabel(secs: ULong): String {
    val minutes = (secs / 60u).toLong().coerceAtLeast(1)
    if (minutes < 60) return if (minutes == 1L) "for 1 minute" else "for $minutes minutes"
    val hours = minutes / 60
    val rest = minutes % 60
    return when {
        rest != 0L -> "for $hours h $rest min"
        hours == 1L -> "for 1 hour"
        else -> "for $hours hours"
    }
}

/** The server of an SSH sign-in: its name when the desktop app knows it, else its host key. */
fun sshTarget(host: String?, hostKey: String?): String =
    host?.trim()?.takeIf { it.isNotEmpty() } ?: hostKey?.trim()?.takeIf { it.isNotEmpty() } ?: "an unknown server"

/** Small caption above a value. */
@Composable
private fun Caption(text: String, modifier: Modifier = Modifier) {
    RText(text, RType.sans(12f, FontWeight.Medium), LocalColors.current.tertiary, modifier)
}

/** Text that is shown exactly as it is (arguments, a command): monospaced, on a tinted plate. */
@Composable
private fun MonoBox(text: String, tag: String, maxLines: Int = Int.MAX_VALUE) {
    val c = LocalColors.current
    RText(
        text,
        RType.mono(13f),
        c.text,
        Modifier
            .fillMaxWidth()
            .background(c.controlFill, RoundedCornerShape(12.dp))
            .padding(12.dp)
            .testTag(tag),
        maxLines = maxLines,
        ltr = true,
    )
}

// ---- a call to an MCP server's tool ----------------------------------------------------------------------------

/** Which server, which tool, what it says it does, the exact arguments, and whether it changes things. */
@Composable
internal fun McpCallSection(call: McpCallView) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("mcpCall"), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Card {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ServiceAvatar("mcp", size = 36.dp)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        RText(untrusted(call.serverName), RType.sans(15.5f, FontWeight.SemiBold), c.text, Modifier.testTag("mcpServer"), maxLines = 1)
                        RText(mcpHost(call.serverUrl), RType.mono(12.5f), c.tertiary, Modifier.testTag("mcpHost"), maxLines = 1, ltr = true)
                    }
                    Spacer(Modifier.width(8.dp))
                    Tag(
                        if (call.readOnly) "Read only" else "Changes things",
                        Modifier.testTag("mcpEffect").semantics(mergeDescendants = true) {},
                        tint = if (call.readOnly) c.success else c.send,
                    )
                }
                Hairline(Modifier.padding(vertical = 6.dp), inset = 0.dp)
                RText(untrusted(call.title).ifEmpty { call.tool }, RType.sans(17f, FontWeight.SemiBold, lineHeight = 23f), c.text, Modifier.testTag("mcpTool"))
                if (call.title != call.tool) RText(untrusted(call.tool), RType.mono(12.5f), c.tertiary, maxLines = 1, ltr = true)
                if (call.description.isNotBlank()) {
                    RText(untrusted(call.description), RType.sans(14f, lineHeight = 20f), c.secondary, maxLines = 6)
                }
                Caption("Arguments", Modifier.padding(top = 8.dp))
                MonoBox(untrusted(call.argumentsJson).ifEmpty { "{}" }, "mcpArguments")
                RText(
                    "The server describes its own tools. Read the arguments: they are exactly what is sent.",
                    RType.sans(12.5f, lineHeight = 17f),
                    c.tertiary,
                    Modifier.padding(top = 4.dp),
                )
            }
        }
        if (call.destructive) {
            Banner(
                "The server marks this tool as destructive. It is asked for every time and can never be allowed in advance.",
                kind = BannerKind.Warning,
                tag = "mcpDestructive",
            )
        }
    }
}

// ---- a file the change uses -----------------------------------------------------------------------------------------

/** The file an AI uploaded for this change, as the server saw it. */
@Composable
internal fun AttachedFileSection(blob: BlobView) {
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("attachedFile")) {
        RText(
            "ATTACHED FILE",
            RType.sans(12.5f, FontWeight.Medium),
            LocalColors.current.secondary,
            Modifier.padding(start = 4.dp, bottom = 8.dp),
        )
        FileCard(blob)
    }
}

// ---- the desktop app ------------------------------------------------------------------------------------------------

/** A yes-or-no question: the question large, what would happen exactly, and what a standing answer would cover. */
@Composable
internal fun AskSection(ask: AskView) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("ask")) {
        Card {
            Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                RText(untrusted(ask.question), RType.sans(22f, FontWeight.SemiBold, lineHeight = 28f), c.text, Modifier.testTag("askQuestion"))
                ask.detail?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { MonoBox(it, "askDetail", maxLines = 30) }
                ask.topic?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { topic ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Caption("About")
                        Spacer(Modifier.width(8.dp))
                        Tag(topic, Modifier.testTag("askTopic").semantics(mergeDescendants = true) {}, mono = true)
                    }
                }
            }
        }
    }
}

/** Secrets the desktop app wants for one command: names and fields only, never the values, and for how long. */
@Composable
internal fun SecretsSection(secrets: SecretReleaseView) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("secrets")) {
        Card {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Caption("For")
                MonoBox(untrusted(secrets.command), "secretsCommand", maxLines = 6)
                secrets.purpose?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let {
                    RText("“$it”", RType.sans(14.5f, lineHeight = 20f), c.secondary, Modifier.padding(top = 2.dp).testTag("secretsPurpose"))
                }
                Caption(if (secrets.items.size == 1) "Secret" else "Secrets (${secrets.items.size})", Modifier.padding(top = 8.dp))
                secrets.items.forEachIndexed { i, item ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        GlyphIcon(Glyph.Key, c.accent, size = 16.dp)
                        Spacer(Modifier.width(10.dp))
                        RText(untrusted(item), RType.sans(15.5f, FontWeight.Medium), c.text, Modifier.testTag("secret:$i"), maxLines = 2)
                    }
                }
                RText(
                    "The desktop app may keep them ${leaseLabel(secrets.leaseSecs)}, in memory only. Their values are never shown here or sent anywhere else.",
                    RType.sans(13.5f, lineHeight = 19f),
                    c.secondary,
                    Modifier.padding(top = 8.dp).testTag("secretsLease"),
                )
            }
        }
    }
}

/** An SSH sign-in signed on this phone: which server, with which key. */
@Composable
internal fun SshSection(ssh: SshSignView) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("ssh")) {
        Card {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                RText(
                    "Sign in to ${untrusted(sshTarget(ssh.host, ssh.hostKey))} with ${untrusted(ssh.keyName)}",
                    RType.sans(18f, FontWeight.SemiBold, lineHeight = 24f),
                    c.text,
                    Modifier.testTag("sshWhat"),
                )
                Caption("Key", Modifier.padding(top = 8.dp))
                MonoBox(untrusted(ssh.keyFingerprint), "sshKey")
                ssh.hostKey?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let {
                    Caption("Server's host key", Modifier.padding(top = 4.dp))
                    MonoBox(it, "sshHostKey")
                }
                RText(
                    "The private key stays on this phone; it signs this one sign-in.",
                    RType.sans(13f, lineHeight = 18f),
                    c.tertiary,
                    Modifier.padding(top = 4.dp),
                )
            }
        }
    }
}
