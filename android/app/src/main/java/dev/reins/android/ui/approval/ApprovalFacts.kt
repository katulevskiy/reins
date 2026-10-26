package dev.reins.android.ui.approval

import dev.reins.android.autopilot.AutopilotText
import androidx.compose.ui.draw.drawBehind
import dev.reins.android.design.hairlineBelow

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.ServiceAvatar
import dev.reins.android.design.StrikeText
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.design.pane
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.formatDate
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.mcp.mcpHost
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.MessageView

/** The main page: the hero, then the facts of the request, by kind. */
@Composable
internal fun MainPage(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val nav = LocalApprovalNav.current
    Column(Modifier.fillMaxWidth().padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        // The risk's reason, when the field had room only for its word.
        val risk = ui.suggestion?.risk?.takeIf { LocalApprovalRecord.current == null && nav.riskInFacts }
        if (risk != null) {
            FactGroup(tag = "riskFact", rows = listOf({ FactRow("Risk", untrusted(risk.why).ifBlank { AutopilotText.riskWord(risk.level) }, valueTag = "riskWhy", maxLines = Int.MAX_VALUE) }))
        }
        val purchase = view.purchase
        val git = view.git
        when {
            view.kind == ApprovalKind.GRANT -> GrantFacts(view)
            view.kind == ApprovalKind.ACCOUNTS -> AccountsFacts(view, ui, viewModel)
            view.kind == ApprovalKind.SEND -> SendFacts(view)
            view.kind == ApprovalKind.WRITE -> {
                when {
                    purchase != null -> PurchaseSection(purchase, ui.purchase, viewModel)
                    git != null -> GitFacts(view, git)
                    view.envRelease != null -> EnvReleaseFacts(view.envRelease!!, ui, viewModel)
                    view.secretsStore != null -> SecretsStoreFacts(view.secretsStore!!)
                    view.tokenStore != null -> TokenStoreFacts(view.tokenStore!!)
                    view.mcp != null -> McpFacts(view.mcp!!)
                    view.ask != null -> AskFacts(view.ask!!, LocalApprovalRecord.current?.label ?: viewModel.pendingLabel(), LocalApprovalRecord.current?.cwd)
                    view.secrets != null -> SecretsFacts(view.secrets!!)
                    view.ssh != null -> SshFacts(view.ssh!!)
                    else -> WriteFacts(view)
                }
                view.blob?.let { AttachedFileSection(it) }
            }
            else -> MessagesFacts(view, ui, viewModel)
        }
        PublicDomainNote(view, ui)
    }
}

// ---- the grouped card ------------------------------------------------------------------------------------------

/**
 * Facts under the field (Next): a raised group 16 in from the edges (radius 20, a hairline ring in light), its rows
 * 16 in, a hairline between them. [borderTint] marks a group that needs a look (a warning) with a bar of that colour
 * on its left.
 */
@Composable
internal fun FactGroup(modifier: Modifier = Modifier, tag: String? = null, borderTint: Color? = null, rows: List<@Composable () -> Unit>) {
    val c = LocalColors.current
    Column(
        modifier
            .padding(horizontal = 16.dp)
            .fillMaxWidth()
            .pane()
            .then(
                if (borderTint != null) {
                    Modifier.drawBehind { drawRect(borderTint, size = androidx.compose.ui.geometry.Size(3.dp.toPx(), size.height)) }
                } else {
                    Modifier
                },
            )
            .then(if (tag != null) Modifier.testTag(tag) else Modifier),
    ) {
        rows.forEachIndexed { i, row ->
            Box(
                Modifier
                    .fillMaxWidth()
                    .then(
                        if (i > 0) {
                            Modifier.drawBehind { drawLine(c.rule, androidx.compose.ui.geometry.Offset(16.dp.toPx(), 0f), androidx.compose.ui.geometry.Offset(size.width, 0f), 1.dp.toPx()) }
                        } else {
                            Modifier
                        },
                    )
                    .padding(horizontal = 16.dp),
            ) { row() }
        }
    }
}

/** "Branch   feature/passkeys": a key on the left, its value on the right. */
@Composable
internal fun FactRow(
    key: String,
    value: String,
    modifier: Modifier = Modifier,
    mono: Boolean = false,
    valueTag: String? = null,
    valueColor: Color? = null,
    maxLines: Int = 1,
    trailing: (@Composable () -> Unit)? = null,
) {
    val c = LocalColors.current
    // Next (`.kv`): the key in meta on the left (at least 62 wide), the value right after it.
    Row(
        modifier.fillMaxWidth().defaultMinSize(minHeight = 50.dp).padding(vertical = if (maxLines > 1) 13.dp else 9.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(key, RType.sans(13.5f, FontWeight.Medium), c.inkMeta, Modifier.defaultMinSize(minWidth = 62.dp), maxLines = 1)
        Spacer(Modifier.width(12.dp))
        RText(
            value,
            if (mono) RType.mono(14f) else RType.sans(15.5f, FontWeight.Medium),
            valueColor ?: c.ink,
            Modifier.weight(1f).then(if (valueTag != null) Modifier.testTag(valueTag) else Modifier),
            maxLines = maxLines,
            ltr = mono,
            overflow = if (mono) TextOverflow.MiddleEllipsis else TextOverflow.Ellipsis,
        )
        if (trailing != null) {
            Spacer(Modifier.width(8.dp))
            trailing()
        }
    }
}

/** A row that opens a page: a title, a short detail on the right, a chevron. */
@Composable
internal fun DrillRow(title: String, tag: String, onClick: () -> Unit, detail: @Composable () -> Unit = {}) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        Modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 56.dp)
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp)) {
                onClick()
                feedback.play(Event.Open)
            }
            .padding(vertical = 9.dp)
            .testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(title, RType.sans(15f, FontWeight.Medium), c.ink, maxLines = 1)
        Spacer(Modifier.width(12.dp))
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterEnd) { detail() }
        Spacer(Modifier.width(6.dp))
        dev.reins.android.design.GlyphIcon(dev.reins.android.design.Glyph.ChevronRight, c.inkFaint, size = 16.dp)
    }
}

/** "+412 −96" in green and red. */
@Composable
internal fun Counts(additions: ULong?, deletions: ULong?, size: Float = 14f, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        additions?.let { RText("+$it", RType.mono(size, FontWeight.Medium), c.success, maxLines = 1) }
        deletions?.let { RText("−$it", RType.mono(size, FontWeight.Medium), c.danger, maxLines = 1) }
    }
}

/** A small header above a group: a few words on the left, an action or a (?) on the right. */
@Composable
internal fun GroupHeader(text: String, modifier: Modifier = Modifier, trailing: @Composable () -> Unit = {}) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().padding(start = 24.dp, end = 20.dp, top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        dev.reins.android.design.LabelText(text, c.ink2, Modifier.weight(1f))
        trailing()
    }
}

// ---- emails and items found ----------------------------------------------------------------------------------------

/** The emails (or items) found: ticking is the decision; a code is never ticked for you, a covered one is locked. */
@Composable
private fun MessagesFacts(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    val draft = ui.draft
    if (view.messages.isEmpty()) {
        FactGroup(rows = listOf { FactRow("Found", "Nothing") })
    } else {
        val allIds = view.messages.map { it.id }
        val allTicked = view.messages.all { it.coveredByGrant || it.id in draft.selected }
        val shared = if (ui.readOnly) view.messages.count { it.coveredByGrant || it.id in draft.selected } else null
        GroupHeader(mailHeader(view.messages, view.service, shared), Modifier.testTag("messagesHeader")) {
            if (!ui.readOnly) {
                TextAction(if (allTicked) "Clear" else "Select all", if (allTicked) "clearAll" else "selectAll") {
                    viewModel.edit { it.copy(selected = if (allTicked) emptySet() else allIds.toSet(), allMail = null) }
                }
            }
        }
        FactGroup(
            // Flagged first: rows are keyed by id, so the order on screen is all that changes.
            rows = attentionOrder(view.messages) { mailMark(it, view.service)?.level }.map { (_, m) ->
                {
                    MailRow(
                        m,
                        checked = m.coveredByGrant || m.id in draft.selected || (draft.allMail != null && !m.sensitive),
                        enabled = !ui.readOnly && !m.coveredByGrant && (draft.allMail == null || m.sensitive),
                        service = view.service,
                    ) { on -> viewModel.edit { d -> d.copy(selected = if (on) d.selected + m.id else d.selected - m.id) } }
                }
            },
        )
    }
    val query = view.query?.let(::untrusted)?.takeIf { it.isNotBlank() }
    if (query != null) {
        FactGroup(
            rows = listOf {
                DrillRow("Search", "queryRow", { nav.push(ApprovalPage.Text("Search", query, mono = true, tag = "queryText")) }) {
                    RText(query, RType.mono(13f), c.tertiary, maxLines = 1, ltr = true)
                }
            },
        )
    }
    val account = view.account?.takeIf { it.isNotBlank() && view.service != "payments" }
    if (account != null && view.kind == ApprovalKind.FETCH) FactGroup(rows = listOf { FactRow("Account", untrusted(account), mono = true) })
}

@Composable
private fun TextAction(text: String, tag: String, onClick: () -> Unit) {
    val c = LocalColors.current
    RText(
        text,
        RType.sans(14f, FontWeight.SemiBold),
        c.accent,
        Modifier.clip(CircleShape).pressable(shape = CircleShape, onClick = onClick).padding(horizontal = 8.dp, vertical = 6.dp).testTag(tag),
        maxLines = 1,
    )
}

/** One email: its tick, subject, sender and date, and a flag when it looks like a code. */
@Composable
private fun MailRow(m: MessageView, checked: Boolean, enabled: Boolean, service: String, onChecked: (Boolean) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val dim by animateFloatAsState(if (checked) 1f else 0.45f, tween(220), label = "dim")
    val title = untrusted(m.subject).ifBlank { untrusted(m.snippet) }.ifBlank { untrusted(m.from) }
    // Mail says who and when; an item elsewhere (a login, a file) says what it is.
    val gmail = service.isBlank() || service == "gmail"
    val meta = if (!gmail && m.subject.isNotBlank() && m.snippet.isNotBlank()) {
        untrusted(m.snippet)
    } else {
        listOfNotNull(
            displayName(m.from).takeIf { it.isNotBlank() && m.subject.isNotBlank() }?.let(::untrusted),
            if (m.date != 0L) formatDate(m.date) else null,
        ).joinToString(" · ")
    }
    Box(Modifier.testTag("message:${m.id}")) {
        Row(
            Modifier
                .fillMaxWidth()
                .toggleable(checked, enabled = enabled, role = Role.Checkbox) {
                    onChecked(it)
                    feedback.play(Event.toggle(it))
                }
                .padding(vertical = 13.dp)
                .testTag("check:${m.id}"),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TickBox(checked, enabled)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Column(Modifier.graphicsLayer { alpha = dim }) {
                    RText(
                        title,
                        RType.row.copy(textDecoration = if (checked) null else TextDecoration.LineThrough),
                        c.text,
                        maxLines = 1,
                    )
                    if (meta.isNotEmpty()) RText(meta, RType.meta, c.inkMeta, maxLines = 1)
                }
                // One chip: a code or a password keeps its word ("Code") among the other reasons.
                mailMark(m, service)?.let {
                    AttentionTag(it, Modifier.padding(top = 6.dp).testTag(if (m.sensitive) "sensitive:${m.id}" else "attention:${m.id}"))
                }
            }
            if (m.coveredByGrant) {
                Spacer(Modifier.width(8.dp))
                Icon(Icons.Rounded.Lock, "Already allowed", Modifier.size(15.dp).testTag("covered:${m.id}"), tint = c.tertiary)
            }
        }
    }
}

/** A square tick (Material) in the accent colour. */
@Composable
internal fun TickBox(checked: Boolean, enabled: Boolean = true) {
    // Signal's tick box: violet-fill with a white tick, or a faint ring.
    dev.reins.android.design.SignalCheckbox(checked, Modifier.graphicsLayer { alpha = if (enabled) 1f else 0.55f })
}

// ---- an email to send --------------------------------------------------------------------------------------------

@Composable
private fun SendFacts(view: ApprovalView) {
    val c = LocalColors.current
    val email = view.email ?: return
    FactGroup(
        tag = "emailPreview",
        rows = buildList<@Composable () -> Unit> {
            add { FactRow("To", email.to.joinToString(", ") { untrusted(it) }, mono = true, maxLines = 3) }
            if (email.cc.isNotEmpty()) add { FactRow("Cc", email.cc.joinToString(", ") { untrusted(it) }, mono = true, maxLines = 3) }
            add { FactRow("Subject", untrusted(email.subject), maxLines = 2) }
            add { RText(untrusted(email.body), RType.sans(15f, lineHeight = 21f), c.text, Modifier.padding(vertical = 14.dp).testTag("emailBody")) }
        },
    )
    view.account?.let { FactGroup(rows = listOf { FactRow("From", untrusted(it), mono = true) }) }
}

// ---- a change elsewhere --------------------------------------------------------------------------------------------

/** What will be done in another integration, line by line; nothing happens until approved. */
@Composable
private fun WriteFacts(view: ApprovalView) {
    val c = LocalColors.current
    // A line that only repeats the ask on the field ("Delete the repository octo/app") is left out: the consequence
    // under it comes first.
    val lines = previewLines(view)
    if (lines.isNotEmpty()) {
        FactGroup(
            tag = "writePreview",
            rows = listOf {
                Column(Modifier.padding(vertical = 14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    lines.forEachIndexed { shown, (i, line) ->
                        RText(
                            untrusted(line),
                            if (shown == 0) RType.sans(16f, FontWeight.SemiBold, lineHeight = 22f) else RType.sans(15f, lineHeight = 21f),
                            if (shown == 0) c.text else c.secondary,
                            Modifier.testTag("previewLine:$i"),
                        )
                    }
                }
            },
        )
    }
    view.account?.takeIf { it.isNotBlank() }?.let { FactGroup(rows = listOf { FactRow("Account", untrusted(it), mono = true) }) }
}

/** The preview lines with their index, without one that only says the ask again. */
internal fun previewLines(view: ApprovalView): List<Pair<Int, String>> {
    fun norm(t: String) = untrusted(t).lowercase().replace(Regex("\\s+"), " ").trimEnd('.', ' ')
    val ask = norm(askWords(view).first)
    return view.preview.withIndex().filter { (_, line) -> norm(line) != ask }.map { it.index to it.value }
}

// ---- a permission, accounts ---------------------------------------------------------------------------------------

@Composable
private fun GrantFacts(view: ApprovalView) {
    val c = LocalColors.current
    val grant = view.grant ?: return
    val (tone, reach) = when (grant.breadth) {
        "everything" -> c.danger to "Everything"
        "broad" -> c.warning to "Broad"
        else -> c.success to "Narrow"
    }
    FactGroup(
        tag = "grantCard",
        rows = buildList<@Composable () -> Unit> {
            grant.lines.forEach { line -> add { FactRow("Only", untrusted(line), maxLines = 2) } }
            add { FactRow("For", periodWords(grant.durationSecs.toLong()) + (grant.maxUses?.let { " · at most $it uses" } ?: "")) }
            // Reach is a word, never only a colour.
            add { FactRow("Reach", "", trailing = { dev.reins.android.design.Pill(reach, tone) }) }
            if (grant.reason.isNotBlank()) add { FactRow("Says", "“${untrusted(grant.reason)}”", maxLines = 3) }
        },
    )
    view.account?.let { FactGroup(rows = listOf { FactRow("Account", untrusted(it), mono = true) }) }
}

/** The accounts an AI would see: tick the ones to share; those it already sees are ticked and locked. */
@Composable
private fun AccountsFacts(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val who = untrusted(view.connectionLabel)
    GroupHeader("Addresses") {
        HelpDot("accounts", "$who sees only the addresses you tick, nothing in them, and how many it did not get.")
    }
    FactGroup(
        tag = "accountsCard",
        rows = view.accounts.mapIndexed { i, address ->
            {
                val locked = address in view.sharedAccounts
                val ticked = locked || address in ui.draft.selected
                val dim by animateFloatAsState(if (ticked) 1f else 0.42f, tween(280), label = "dim")
                Box(Modifier.testTag("shared:$i")) {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .toggleable(ticked, enabled = !locked && !ui.readOnly, role = Role.Checkbox) { on ->
                                viewModel.edit { d -> d.copy(selected = if (on) d.selected + address else d.selected - address) }
                                feedback.play(Event.toggle(on))
                            }
                            .padding(vertical = 12.dp)
                            .testTag("acct:$address"),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        TickBox(ticked, !locked && !ui.readOnly)
                        Spacer(Modifier.width(14.dp))
                        Row(Modifier.weight(1f).graphicsLayer { alpha = dim }, verticalAlignment = Alignment.CenterVertically) {
                            BlobAvatar(address, size = 32.dp)
                            Spacer(Modifier.width(12.dp))
                            StrikeText(address, RType.sans(15.5f, FontWeight.Medium), c.text, struck = !ticked, modifier = Modifier.weight(1f, fill = false))
                        }
                        if (locked) {
                            Spacer(Modifier.width(8.dp))
                            Icon(Icons.Rounded.Lock, "Already shared", Modifier.size(15.dp), tint = c.tertiary)
                        }
                    }
                }
            }
        },
    )
}

// ---- a tool of an MCP server, the desktop app ---------------------------------------------------------------------

@Composable
private fun McpFacts(call: dev.reins.core.McpCallView) {
    val c = LocalColors.current
    val args = untrusted(call.argumentsJson).ifEmpty { "{}" }
    Box(Modifier.testTag("mcpCall")) {
        FactGroup(
            rows = listOf(
                {
                    Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        ServiceAvatar("mcp", size = 34.dp)
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            RText(untrusted(call.serverName), RType.sans(15.5f, FontWeight.SemiBold), c.text, Modifier.testTag("mcpServer"), maxLines = 1)
                            RText(mcpHost(call.serverUrl), RType.mono(12.5f), c.tertiary, Modifier.testTag("mcpHost"), maxLines = 1, ltr = true)
                        }
                        Spacer(Modifier.width(8.dp))
                        RText(
                            if (call.readOnly) "Read only" else "Changes things",
                            RType.sans(12.5f, FontWeight.SemiBold),
                            if (call.readOnly) c.success else c.send,
                            Modifier.clip(CircleShape).background((if (call.readOnly) c.success else c.send).copy(alpha = 0.12f)).padding(horizontal = 10.dp, vertical = 5.dp).testTag("mcpEffect"),
                            maxLines = 1,
                        )
                    }
                },
                {
                    Column(Modifier.fillMaxWidth().padding(vertical = 13.dp)) {
                        RText(untrusted(call.title).ifEmpty { untrusted(call.tool) }, RType.sans(16f, FontWeight.SemiBold), c.text, Modifier.testTag("mcpTool"), maxLines = 2)
                        if (call.title != call.tool) RText(untrusted(call.tool), RType.mono(12.5f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 1, ltr = true)
                        if (call.description.isNotBlank()) {
                            RText(untrusted(call.description), RType.sans(13.5f, lineHeight = 19f), c.secondary, Modifier.padding(top = 4.dp), maxLines = 3)
                        }
                    }
                },
                {
                    Column(Modifier.fillMaxWidth().padding(vertical = 13.dp)) {
                        RText("Arguments", RType.sans(13f, FontWeight.Medium), c.tertiary)
                        RText(args, RType.mono(13f), c.text, Modifier.padding(top = 6.dp).testTag("mcpArguments"), maxLines = 12, ltr = true)
                    }
                },
            ),
        )
    }
}

@Composable
private fun AskFacts(ask: dev.reins.core.AskView, label: dev.reins.core.RequestLabel?, cwd: String?) {
    val detail = ask.detail?.let(::untrusted)?.takeIf { it.isNotBlank() }
    val topic = ask.topic?.let(::untrusted)?.takeIf { it.isNotBlank() }
    label?.command?.let { presentation ->
        if (LocalApprovalRecord.current != null) return // History owns these facts and the original below.
        val nav = LocalApprovalNav.current
        CommandFacts(presentation, cwd)
        val original = presentation.original.takeIf { it.isNotBlank() }?.let(::untrusted) ?: detail
        if (original != null) FactGroup(rows = listOf {
            DrillRow("Original command", "fullInvocation", {
                nav.push(ApprovalPage.Text("Original command · may contain secrets", original, mono = true, tag = "fullInvocationText"))
            }) { RText("Includes setup and environment values", RType.sans(12f), LocalColors.current.ink2, maxLines = 2) }
        })
        return
    }
    // A command as the core read it (its label), never re-read here (ASK-TEXT.md): the whole of it as a mono fact row,
    // wrapped, never cut; where it ran as the core kept it; the guard's rule.
    val command = label?.takeIf { it.provider == "terminal" }?.subject?.let(::untrusted)?.trim()?.takeIf { it.isNotEmpty() }
    if (command != null) {
        FactGroup(
            tag = "ask",
            rows = buildList<@Composable () -> Unit> {
                add { CommandRow(command) }
                cwd?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { add { FactRow("In", it, mono = true, valueTag = "askIn", maxLines = 3) } }
                ask.rule?.pattern?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { add { FactRow("Rule", it, mono = true, valueTag = "askRule", maxLines = 3) } }
            },
        )
        return
    }
    if (detail == null && topic == null) return
    FactGroup(
        tag = "ask",
        rows = buildList<@Composable () -> Unit> {
            if (detail != null) add { MonoBlock(detail, "askDetail") }
            if (topic != null) add { FactRow("About", topic, mono = true, valueTag = "askTopic") }
        },
    )
}

/** Core-derived command facts. No parsing and no authorization decisions happen in the UI. */
@Composable
internal fun CommandFacts(presentation: dev.reins.core.CommandPresentation, cwd: String? = null) {
    val c = LocalColors.current
    var showSetup by remember(presentation, cwd) { mutableStateOf(false) }
    val setup = (presentation.setup + listOfNotNull(cwd?.takeIf { it.isNotBlank() }?.let { "Requested directory: $it" })).distinct()
    FactGroup(tag = "commandPresentation", rows = buildList<@Composable () -> Unit> {
        if (presentation.actionable.isNotBlank()) add { CommandRow(untrusted(presentation.actionable)) }
        presentation.notices.filter { it.isNotBlank() }.distinct().forEachIndexed { i, notice -> add {
            RText(untrusted(notice), RType.sans(14f, lineHeight = 20f), c.ink2,
                Modifier.fillMaxWidth().padding(vertical = 12.dp).testTag("commandNotice:$i"))
        } }
        if (setup.isNotEmpty()) add {
            Column(Modifier.fillMaxWidth().pressable(label = if (showSetup) "Hide technical details" else "Show technical details") { showSetup = !showSetup }
                .semantics { stateDescription = if (showSetup) "Expanded" else "Collapsed" }
                .padding(vertical = 13.dp).testTag("commandTechnicalDetails")) {
                RText(if (showSetup) "Technical details −" else "Technical details +", RType.sans(15.5f, FontWeight.Medium), c.ink)
                RText("Environment and tool configuration · ${setup.size} items", RType.sans(12f), c.ink2,
                    Modifier.padding(top = 3.dp))
            }
        }
        if (showSetup) add {
            RText(untrusted(setup.distinct().joinToString("\n")), RType.mono(13f), c.ink,
                Modifier.fillMaxWidth().padding(vertical = 12.dp).testTag("commandSetup"), ltr = true)
        }
    })
}

/** "Command" over the whole command in mono, wrapped. */
@Composable
private fun CommandRow(command: String) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().padding(top = 12.dp)) {
        RText("Command", RType.sans(15f), c.ink2, maxLines = 1)
        RText(command, RType.mono(13.5f).copy(lineBreak = androidx.compose.ui.text.style.LineBreak.Paragraph), c.ink,
            Modifier.fillMaxWidth().padding(top = 6.dp, bottom = 12.dp).testTag("askCommand"), ltr = true)
    }
}

@Composable
private fun SecretsFacts(secrets: dev.reins.core.SecretReleaseView) {
    val c = LocalColors.current
    FactGroup(
        tag = "secrets",
        rows = buildList<@Composable () -> Unit> {
            add { FactRow("For", untrusted(secrets.command), mono = true, valueTag = "secretsCommand", maxLines = 3) }
            secrets.purpose?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { add { FactRow("Says", "“$it”", valueTag = "secretsPurpose", maxLines = 2) } }
            secrets.items.forEachIndexed { i, item ->
                add {
                    Row(Modifier.fillMaxWidth().padding(vertical = 13.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Rounded.Key, null, Modifier.size(17.dp), tint = c.accent)
                        Spacer(Modifier.width(12.dp))
                        RText(untrusted(item), RType.sans(15.5f, FontWeight.Medium), c.text, Modifier.testTag("secret:$i"), maxLines = 2)
                    }
                }
            }
            add { FactRow("Kept", leaseLabel(secrets.leaseSecs).removePrefix("for "), valueTag = "secretsLease") }
        },
    )
}

@Composable
private fun SshFacts(ssh: dev.reins.core.SshSignView) {
    FactGroup(
        tag = "ssh",
        rows = buildList<@Composable () -> Unit> {
            add { FactRow("Server", untrusted(sshTarget(ssh.host, ssh.hostKey)), mono = true, valueTag = "sshWhat") }
            add { FactRow("Key", untrusted(ssh.keyName)) }
            add { MonoBlock(untrusted(ssh.keyFingerprint), "sshKey") }
            ssh.hostKey?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { add { FactRow("Host key", it, mono = true, valueTag = "sshHostKey", maxLines = 3) } }
        },
    )
}

/** Text shown exactly as it is (a command, arguments), monospaced. */
@Composable
internal fun MonoBlock(text: String, tag: String, maxLines: Int = Int.MAX_VALUE) {
    RText(text, RType.mono(13f), LocalColors.current.text, Modifier.fillMaxWidth().padding(vertical = 13.dp).testTag(tag), maxLines = maxLines, ltr = true)
}

/** A long text folded out of the main page, in full. */
@Composable
internal fun TextPage(page: ApprovalPage.Text) {
    val c = LocalColors.current
    FactGroup(
        modifier = Modifier.padding(top = 6.dp),
        rows = listOf {
            RText(
                page.body,
                if (page.mono) RType.mono(14f) else RType.sans(15f, lineHeight = 21f),
                c.text,
                Modifier.fillMaxWidth().padding(vertical = 14.dp).testTag(page.tag),
                ltr = page.mono,
            )
        },
    )
}

/** A remembered permission would let in everyone at a free mail provider. */
@Composable
private fun PublicDomainNote(view: ApprovalView, ui: ApprovalUi) {
    val c = LocalColors.current
    val choice = (buildDecision(view, ui.draft) as? BuildResult.Ok)?.choice ?: return
    val domains = publicDomainWarnings(choice)
    if (domains.isEmpty()) return
    Row(Modifier.fillMaxWidth().padding(horizontal = 22.dp).testTag("publicDomainWarning"), verticalAlignment = Alignment.CenterVertically) {
        WarnLine("Anyone at ${domains.joinToString(", ")}", Modifier.weight(1f))
        Spacer(Modifier.width(8.dp))
        HelpDot("publicDomain", "These are free mail providers: allowing the whole domain covers millions of unrelated people. Prefer specific addresses.")
    }
}
