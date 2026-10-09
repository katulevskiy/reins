package dev.reins.android.ui.approval

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.ui.graphics.graphicsLayer
import dev.reins.android.design.StrikeText
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Slider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.ActionKind
import dev.reins.android.design.ActionTile
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.Card
import dev.reins.android.design.serviceName
import dev.reins.android.design.CheckRow
import dev.reins.android.design.ConnectorTags
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.SelectChip
import dev.reins.android.design.Spinner
import dev.reins.android.design.Tag
import dev.reins.android.design.Toggle
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowMillis
import dev.reins.android.design.urgency
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.detentAction
import dev.reins.android.feedback.feedbackAction
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.formatTime
import dev.reins.android.ui.common.operationTitle
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.EmailView
import dev.reins.core.GrantRequestView
import dev.reins.core.MessageView
import dev.reins.core.ResourceView

/** The content of the approval sheet: what is asked, one tap to decide, and everything else under "More". */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ApprovalSheet(viewModel: ApprovalViewModel, authenticator: Authenticator, onDone: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(ui.finished) { if (ui.finished) onDone() }
    val view = ui.view
    if (view == null) {
        Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            if (ui.loading) Spinner(c.secondary, size = 32.dp)
            ui.error?.let {
                Banner(it, Modifier.padding(top = 32.dp), BannerKind.Error)
                CapsuleButton("Close", Modifier.padding(top = 16.dp), style = ButtonStyle.Secondary, onClick = onDone)
            }
        }
        return
    }
    val draft = ui.draft
    val isGrant = view.kind == ApprovalKind.GRANT
    val isAccounts = view.kind == ApprovalKind.ACCOUNTS
    val isWrite = view.kind == ApprovalKind.WRITE
    val send = view.kind == ApprovalKind.SEND
    val scroll = rememberScrollState()
    // Opening "More options" brings them into view instead of leaving them below the buttons.
    LaunchedEffect(ui.moreOpen) {
        if (ui.moreOpen) {
            kotlinx.coroutines.delay(280)
            scroll.animateScrollTo(scroll.maxValue)
        }
    }

    Column(Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f).verticalScroll(scroll)) {
            RequestHeader(view)
            ui.suggestion?.let { dev.reins.android.ui.autopilot.SuggestionStrip(it, Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) }
            when {
                isGrant -> view.grant?.let { GrantRequestCard(it, view.connectionLabel) }
                isAccounts -> AccountsCard(view, ui, viewModel)
                send -> view.email?.let { EmailPreview(it) }
                isWrite -> {
                    // What the change is, shown in its own terms instead of the core's summary lines where there are
                    // some: a push ref by ref, a call to an MCP server's tool, a question, secrets, an SSH sign-in.
                    val git = view.git
                    val mcp = view.mcp
                    val ask = view.ask
                    val secrets = view.secrets
                    val ssh = view.ssh
                    when {
                        git != null -> GitPushSection(git, serviceName(view.service))
                        mcp != null -> McpCallSection(mcp)
                        ask != null -> AskSection(ask)
                        secrets != null -> SecretsSection(secrets)
                        ssh != null -> SshSection(ssh)
                        else -> WritePreview(view)
                    }
                    view.blob?.let { AttachedFileSection(it) }
                    // A destructive MCP tool says so in its own section.
                    if (view.noStanding && mcp == null) {
                        Banner(
                            "This changes something that cannot be undone or reaches far. It is always asked for and can never be allowed in advance, so read the details above before you approve.",
                            Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                            BannerKind.Warning,
                            tag = "onceWarning",
                        )
                    }
                }
                else -> MessagesSection(view, viewModel, ui)
            }
            MoreSection(view, ui, viewModel)
            ui.error?.let { Banner(it, Modifier.padding(16.dp), BannerKind.Error) }
            val preview = buildChoice(view, draft)
            if (preview is BuildResult.Ok) {
                val warnings = publicDomainWarnings(preview.choice)
                if (warnings.isNotEmpty()) {
                    Banner(
                        "Allowing everyone at ${warnings.joinToString(", ")} covers millions of unrelated people. Prefer specific addresses.",
                        Modifier.padding(16.dp),
                        BannerKind.Warning,
                    )
                }
            }
            Spacer(Modifier.height(12.dp))
        }
        Column(
            Modifier
                .fillMaxWidth()
                .background(c.background)
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            // "More options" says how long itself; the shortcut would only contradict it.
            if (!ui.moreOpen) QuickAllow(view, ui) { viewModel.approve(authenticator, allow = true) }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                // A question from the desktop app is answered, not approved.
                val question = view.ask != null
                CapsuleButton(if (question) "No" else "Deny", Modifier.weight(1f).testTag("deny"), style = ButtonStyle.Secondary, enabled = !ui.busy, onClick = viewModel::deny)
                CapsuleButton(
                    when {
                        question -> "Yes"
                        isGrant || isAccounts -> "Allow"
                        else -> "Approve"
                    },
                    Modifier.weight(1.3f).testTag("approve"),
                    style = if (isGrant || isAccounts) ButtonStyle.Accent else ButtonStyle.Primary,
                    enabled = !ui.busy,
                    busy = ui.busy,
                ) { viewModel.approve(authenticator) }
            }
        }
    }
}

/**
 * "Approve and allow for 1 hour": the approval plus a permission for the same AI, the same kind of request and the same
 * target, which the core works out (never offered for what is asked every time). After a few identical approvals it
 * says so and offers a longer period.
 */
@Composable
private fun QuickAllow(view: ApprovalView, ui: ApprovalUi, onAllow: () -> Unit) {
    val c = LocalColors.current
    val quick = view.quick ?: return
    val secs = quick.allow?.durationSecs?.toLong() ?: return
    val repeated = quick.repeats >= REPEATS_FOR_HINT
    Column(Modifier.fillMaxWidth().padding(bottom = 10.dp).testTag("quickAllow")) {
        if (repeated) {
            Row(Modifier.padding(bottom = 8.dp).testTag("repeatHint"), verticalAlignment = Alignment.CenterVertically) {
                GlyphIcon(Glyph.Sparkle, c.accent, size = 14.dp, weight = 1.9f)
                Spacer(Modifier.width(6.dp))
                RText(
                    "You approved this ${quick.repeats} times in the last 24 hours.",
                    RType.sans(13.5f, FontWeight.Medium),
                    c.accent,
                )
            }
        }
        CapsuleButton(
            "Approve and allow for ${hoursLabel(secs)}",
            Modifier.fillMaxWidth().testTag("approveAllow"),
            style = if (repeated) ButtonStyle.Accent else ButtonStyle.Secondary,
            enabled = !ui.busy,
            onClick = onAllow,
        )
        RText(
            "${untrusted(view.connectionLabel)} can then do the same without asking: ${untrusted(quick.allowWhat)}. Revoke it any time in Grants.",
            RType.sans(12.5f, lineHeight = 17f),
            c.tertiary,
            Modifier.padding(start = 4.dp, end = 4.dp, top = 6.dp).testTag("allowWhat"),
            maxLines = 3,
        )
    }
}

/** How many earlier identical approvals make the sheet point them out (the core then offers 8 hours). */
private const val REPEATS_FOR_HINT = 2u

/** 3600 → "1 hour", 28800 → "8 hours"; anything else as [durationLabel] says it. */
fun hoursLabel(secs: Long): String = when {
    secs == 3_600L -> "1 hour"
    secs % 3_600L == 0L && secs < 86_400L -> "${secs / 3_600} hours"
    else -> durationLabel(secs)
}

@Composable
private fun RequestHeader(view: ApprovalView) {
    val c = LocalColors.current
    val action = when (view.kind) {
        ApprovalKind.SEARCH -> ActionKind.Search
        ApprovalKind.READ -> ActionKind.Read
        ApprovalKind.SEND -> ActionKind.Send
        ApprovalKind.GRANT -> ActionKind.Grant
        ApprovalKind.ACCOUNTS -> ActionKind.Accounts
        // The core says what kind of thing it is ("list", "read", "search", "write", "send").
        ApprovalKind.FETCH -> ActionKind.of(view.action).takeIf { it != ActionKind.Other } ?: ActionKind.Read
        ApprovalKind.WRITE -> ActionKind.of(view.action).takeIf { it != ActionKind.Other } ?: ActionKind.Write
    }
    Column(Modifier.padding(start = 20.dp, end = 60.dp, top = 8.dp, bottom = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            ConnectionIcon(view.connectionId, view.connectionLabel, size = 40.dp)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText(untrusted(view.connectionLabel), RType.sans(17f, FontWeight.SemiBold), c.text, maxLines = 1)
                RText(formatTime(view.createdAt), RType.sans(12.5f), c.tertiary, maxLines = 1)
            }
        }
        Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            ActionTile(action, if (action == ActionKind.Grant) 1 else view.count(), size = 48.dp)
            Spacer(Modifier.width(14.dp))
            RText(
                operationTitle(action.key, view.count(), view.service, view.mcp?.title?.let(::untrusted) ?: view.opTitle, view.op),
                RType.sans(26f, FontWeight.SemiBold, lineHeight = 32f),
                c.text,
                Modifier.weight(1f).testTag("what"),
                maxLines = 2,
            )
        }
        // What approving does, in one sentence, before any detail.
        if (view.headline.isNotBlank()) {
            RText(untrusted(view.headline), RType.sans(16f, lineHeight = 22f), c.text, Modifier.padding(top = 12.dp).testTag("headline"), maxLines = 4)
        }
        ConnectorTags(view.service, view.account, Modifier.padding(top = 12.dp), name = view.mcp?.serverName?.let(::untrusted))
        view.query?.let { RText(it, RType.mono(14f), c.secondary, Modifier.padding(top = 12.dp), maxLines = 3, ltr = true) }
        WaitLine(view)
    }
}

/** How long the AI is still waiting, or that it stopped (approving is still possible). */
@Composable
private fun WaitLine(view: ApprovalView) {
    val c = LocalColors.current
    val now = rememberNowMillis()
    val u = urgency(view.createdAt, view.waitUntil, now) ?: return
    val label = untrusted(view.connectionLabel)
    if (u.stale) {
        Banner(
            "$label stopped waiting. You can still approve; then ask $label to try again and it will go through.",
            Modifier.padding(top = 14.dp),
            BannerKind.Warning,
            tag = "lateBanner",
        )
    } else {
        RText(
            "$label is waiting · ${u.remainingSeconds} s left",
            RType.sans(13.5f, FontWeight.Medium),
            if (u.urgent) c.danger else c.secondary,
            Modifier.padding(top = 12.dp).testTag("waitLine"),
        )
    }
}

/** What the request covers: the emails found for a search, else what the core says was asked for. */
private fun ApprovalView.count(): Int = when (kind) {
    ApprovalKind.SEARCH, ApprovalKind.FETCH -> messages.size
    ApprovalKind.WRITE -> 1
    else -> count.toInt()
}

@Composable
private fun MessagesSection(view: ApprovalView, viewModel: ApprovalViewModel, ui: ApprovalUi) {
    val c = LocalColors.current
    val draft = ui.draft
    val allIds = view.messages.map { it.id }
    val allTicked = allIds.isNotEmpty() && view.messages.all { it.coveredByGrant || it.id in draft.selected }
    Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 14.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        RText(
            (if (view.kind == ApprovalKind.FETCH) "Found" else "Emails found") + " (${view.messages.size})",
            RType.sans(13f, FontWeight.SemiBold),
            c.secondary,
            Modifier.weight(1f),
        )
        if (view.messages.isNotEmpty()) {
            if (allTicked) {
                CapsuleButton("Clear", Modifier.testTag("clearAll"), style = ButtonStyle.Ghost, compact = true) {
                    viewModel.edit { it.copy(selected = emptySet(), allMail = null) }
                }
            } else {
                CapsuleButton("Select all", Modifier.testTag("selectAll"), style = ButtonStyle.Secondary, compact = true, glyph = Glyph.Check) {
                    viewModel.edit { it.copy(selected = allIds.toSet(), allMail = null) }
                }
            }
        }
    }
    if (view.messages.isEmpty()) {
        RText("Nothing matched.", RType.sans(15f), c.secondary, Modifier.padding(horizontal = 20.dp, vertical = 8.dp))
        return
    }
    Group {
        view.messages.forEachIndexed { i, message ->
            if (i > 0) Hairline(inset = 52.dp)
            MessageRow(
                message = message,
                // "All mail" ticks everything but what looks like a code, which stays the user's own pick.
                checked = message.coveredByGrant || message.id in draft.selected || (draft.allMail != null && !message.sensitive),
                enabled = !message.coveredByGrant && (draft.allMail == null || message.sensitive),
                onChecked = { on ->
                    viewModel.edit { d -> d.copy(selected = if (on) d.selected + message.id else d.selected - message.id) }
                },
            )
        }
    }
}

@Composable
private fun MessageRow(message: MessageView, checked: Boolean, enabled: Boolean, onChecked: (Boolean) -> Unit) {
    val c = LocalColors.current
    Box(Modifier.testTag("message:${message.id}")) {
        CheckRow(
            checked = checked,
            onChange = onChecked,
            enabled = enabled,
            tag = "check:${message.id}",
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Column(Modifier.weight(1f)) {
                if (message.from.isNotEmpty()) RText(message.from, RType.sans(15f, FontWeight.SemiBold), c.text, maxLines = 1, ltr = true)
                if (message.subject.isNotEmpty()) {
                    RText(untrusted(message.subject), RType.sans(14.5f), c.text, Modifier.padding(top = 1.dp), maxLines = 2)
                }
                if (message.snippet.isNotEmpty()) {
                    RText(untrusted(message.snippet), RType.sans(13.5f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 3)
                }
                val when_ = (if (message.date != 0L) formatTime(message.date) else "") +
                    (if (message.coveredByGrant) (if (message.date != 0L) " · " else "") + "already allowed by a grant" else "")
                if (when_.isNotEmpty()) RText(when_, RType.sans(12f), c.tertiary, Modifier.padding(top = 4.dp))
                if (message.sensitive) {
                    Tag("Looks like a code or a password", Modifier.padding(top = 6.dp).testTag("sensitive:${message.id}"), tint = c.warning)
                }
            }
        }
    }
}

@Composable
private fun EmailPreview(email: EmailView) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        Card(Modifier.testTag("emailPreview")) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                RText("To", RType.sans(12f, FontWeight.Medium), c.tertiary)
                email.to.forEach { RText(untrusted(it), RType.mono(14f), c.text, ltr = true) }
                if (email.cc.isNotEmpty()) {
                    RText("Cc", RType.sans(12f, FontWeight.Medium), c.tertiary, Modifier.padding(top = 6.dp))
                    email.cc.forEach { RText(untrusted(it), RType.mono(14f), c.text, ltr = true) }
                }
                RText("Subject", RType.sans(12f, FontWeight.Medium), c.tertiary, Modifier.padding(top = 6.dp))
                RText(untrusted(email.subject), RType.sans(16f, FontWeight.SemiBold), c.text)
                RText("Message", RType.sans(12f, FontWeight.Medium), c.tertiary, Modifier.padding(top = 6.dp))
                RText(untrusted(email.body), RType.sans(15f, lineHeight = 21f), c.text)
            }
        }
    }
}

/** What will be done in another integration, spelled out; nothing happens until the user approves. */
@Composable
private fun WritePreview(view: ApprovalView) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        Card(Modifier.testTag("writePreview")) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                view.resources.firstOrNull()?.let { RText(untrusted(it.label), RType.sans(12f, FontWeight.Medium), c.tertiary) }
                view.preview.forEachIndexed { i, line ->
                    RText(
                        untrusted(line),
                        if (i == 0) RType.sans(17f, FontWeight.SemiBold, lineHeight = 23f) else RType.sans(15f, lineHeight = 21f),
                        if (i == 0) c.text else c.secondary,
                        Modifier.testTag("previewLine:$i"),
                    )
                }
            }
        }
    }
}

/** How long to remember an approval in another integration and what it covers; a password is never remembered. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ConnectorOptions(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val draft = ui.draft
    val write = view.kind == ApprovalKind.WRITE
    if (view.noStanding) {
        RText(
            if (write) {
                "This is asked for every time. A change like this is never remembered."
            } else {
                "This is asked for every time. Passwords, codes and one-time values are never remembered."
            },
            RType.sans(14f, lineHeight = 20f),
            c.secondary,
            Modifier.padding(horizontal = 20.dp, vertical = 8.dp).testTag("noStanding"),
        )
        return
    }
    val service = serviceName(view.service)
    if (!write) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) {
            Card {
                Column(Modifier.padding(16.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        GlyphIcon(Glyph.Clock, c.accent, size = 20.dp)
                        Spacer(Modifier.width(10.dp))
                        RText("Allow all of $service for a while", RType.sans(16f, FontWeight.SemiBold), c.text, Modifier.weight(1f))
                    }
                    RText(
                        "This AI can list, read and search $service without asking again, then asks again when time is up. Changing things is never included.",
                        RType.sans(13.5f, lineHeight = 19f),
                        c.secondary,
                        Modifier.padding(top = 6.dp, bottom = 12.dp),
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        EVERYTHING_LIFETIMES.forEach { kind ->
                            SelectChip(kind.label, draft.allMail == kind, Modifier.testTag("allMail:${kind.name}")) {
                                viewModel.edit {
                                    if (it.allMail == kind) it.copy(allMail = null) else it.copy(allMail = kind, selected = view.messages.filter { m -> !m.sensitive }.map { m -> m.id }.toSet())
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if (draft.allMail != null) return
    RText("Remember this for", RType.sans(13f, FontWeight.SemiBold), c.secondary, Modifier.padding(start = 20.dp, top = 14.dp, bottom = 8.dp))
    FlowRow(
        Modifier.padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        LifetimeKind.entries.filter { it != LifetimeKind.MONTH }.forEach { kind ->
            SelectChip(kind.label, draft.lifetime == kind, Modifier.testTag("lifetime:${kind.name}")) {
                viewModel.edit { it.copy(lifetime = kind) }
            }
        }
    }
    if (draft.lifetime == LifetimeKind.USES) {
        RTextField(
            draft.uses.toString(),
            { text -> text.filter(Char::isDigit).take(4).toIntOrNull()?.let { n -> viewModel.edit { it.copy(uses = n) } } },
            "Number of uses (1–$MAX_USES)",
            Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
            tag = "uses",
            mono = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        )
    }
    if (draft.lifetime == LifetimeKind.ONCE) {
        RText("Only this request. Nothing is remembered.", RType.sans(13.5f), c.tertiary, Modifier.padding(start = 20.dp, top = 8.dp))
        return
    }
    if (write && view.classes.isNotEmpty()) {
        RText("Allow these kinds of change", RType.sans(13f, FontWeight.SemiBold), c.secondary, Modifier.padding(start = 20.dp, top = 18.dp, bottom = 8.dp))
        FlowRow(
            Modifier.padding(horizontal = 16.dp).testTag("classes"),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            view.classes.forEach { kind ->
                val on = kind.id in draft.classes
                SelectChip(kind.label, on, Modifier.testTag("class:${kind.id}")) {
                    viewModel.edit { it.copy(classes = toggleClass(it.classes, kind.id, !on)) }
                }
            }
        }
        RText(
            "At least one stays ticked. Only what you tick is allowed without asking.",
            RType.sans(12.5f),
            c.tertiary,
            Modifier.padding(start = 20.dp, top = 6.dp),
        )
    }
    RText("What should it cover?", RType.sans(13f, FontWeight.SemiBold), c.secondary, Modifier.padding(start = 20.dp, top = 18.dp, bottom = 8.dp))
    val (wide, narrow) = view.resources.partition { it.wider }
    ResourceGroup(narrow, draft, viewModel)
    if (wide.isNotEmpty()) {
        RText(
            "Or a wider permission",
            RType.sans(12.5f, FontWeight.Medium),
            c.tertiary,
            Modifier.padding(start = 20.dp, top = 14.dp, bottom = 8.dp).testTag("widerCaption"),
        )
        ResourceGroup(wide, draft, viewModel)
    }
}

/** The things a permission can cover, each with a tick; ticking one that contains or is inside another swaps them. */
@Composable
private fun ResourceGroup(resources: List<ResourceView>, draft: ApprovalDraft, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    if (resources.isEmpty()) return
    Group {
        resources.forEachIndexed { i, resource ->
            if (i > 0) Hairline()
            CheckRow(
                checked = resource.id in draft.resources,
                onChange = { on -> viewModel.edit { d -> d.copy(resources = toggleResource(d.resources, resource.id, on)) } },
                tag = "resource:${resource.id}",
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            ) { RText(untrusted(resource.label), RType.sans(15.5f), c.text, maxLines = 2) }
        }
    }
}

/** A permission an AI asks for: set apart in the accent colour so it cannot be mistaken for a one-off request. */
@Composable
private fun GrantRequestCard(grant: GrantRequestView, label: String) {
    val c = LocalColors.current
    val tone = when (grant.breadth) {
        "everything" -> c.danger
        "broad" -> c.warning
        else -> c.success
    }
    val toneText = when (grant.breadth) {
        "everything" -> "Everything"
        "broad" -> "Broad"
        else -> "Narrow"
    }
    Column(
        Modifier
            .padding(horizontal = 16.dp, vertical = 10.dp)
            .fillMaxWidth()
            .testTag("grantCard")
            .background(c.accent.copy(alpha = 0.09f), RoundedCornerShape(22.dp))
            .border(1.5.dp, c.accent.copy(alpha = 0.55f), RoundedCornerShape(22.dp))
            .padding(18.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            GlyphIcon(Glyph.ShieldCheck, c.accent, size = 22.dp)
            Spacer(Modifier.width(8.dp))
            RText("PERMISSION REQUEST", RType.sans(12.5f, FontWeight.SemiBold).copy(letterSpacing = 0.8.sp), c.accent, Modifier.weight(1f))
            Tag(toneText, tint = tone)
        }
        RText(grantSentence(grant), RType.sans(19f, FontWeight.SemiBold, lineHeight = 25f), c.text)
        grant.lines.forEach { RText("• $it", RType.sans(15f), c.text, ltr = true) }
        RText("For ${durationLabel(grant.durationSecs.toLong())}" + (grant.maxUses?.let { " · at most $it uses" } ?: ""), RType.sans(15f, FontWeight.Medium), c.text)
        if (grant.reason.isNotBlank()) {
            Column(Modifier.fillMaxWidth().background(c.controlFill, RoundedCornerShape(14.dp)).padding(12.dp)) {
                RText("${untrusted(label)} says", RType.sans(12f, FontWeight.Medium), c.tertiary)
                RText("“${untrusted(grant.reason)}”", RType.sans(14.5f, lineHeight = 20f), c.text, Modifier.padding(top = 2.dp))
            }
        }
        RText(
            "Nothing is shared now. Requests that match this are answered without asking until it ends; you can revoke it any time in Grants.",
            RType.sans(13f, lineHeight = 18f),
            c.secondary,
        )
    }
}

private fun grantSentence(g: GrantRequestView): String =
    if (g.action == "send") "Send emails without asking" else "Read emails without asking"

fun durationLabel(secs: Long): String = when {
    secs < 3_600 -> "${secs / 60} min"
    secs < 86_400 -> "${secs / 3_600} h"
    secs == 86_400L -> "24 hours"
    else -> "${secs / 86_400} days"
}

/** Everything beyond "approve or deny": how long, how wide, all mail. Collapsed by default. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun MoreSection(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val rotation by animateFloatAsState(if (ui.moreOpen) 90f else 0f, label = "more")
    Column(Modifier.fillMaxWidth().animateContentSize().padding(top = 8.dp)) {
        Row(
            Modifier
                .fillMaxWidth()
                .testTag("moreToggle")
                .pressable(shape = RoundedCornerShape(0.dp), onClick = feedbackAction(Event.expand(!ui.moreOpen), viewModel::toggleMore))
                .padding(horizontal = 20.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText("More options", RType.sans(15.5f, FontWeight.Medium), c.accent, Modifier.weight(1f))
            GlyphIcon(Glyph.ChevronRight, c.accent, size = 16.dp, modifier = Modifier.rotate(rotation))
        }
        AnimatedVisibility(ui.moreOpen) {
            Column {
                when (view.kind) {
                    ApprovalKind.GRANT -> ShortenGrant(view, ui, viewModel)
                    ApprovalKind.ACCOUNTS -> AccountsPeriod(ui, viewModel)
                    ApprovalKind.FETCH, ApprovalKind.WRITE -> ConnectorOptions(view, ui, viewModel)
                    else -> StandardOptions(view, ui, viewModel)
                }
            }
        }
    }
}

/**
 * The accounts an AI would be shown, each with a round tick. Unticking one crosses it out and greys it: the AI does not
 * get that address (it is only told how many it did not get). Accounts it can already see are ticked and locked.
 */
@Composable
private fun AccountsCard(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val who = untrusted(view.connectionLabel)
    val service = serviceName(view.service)
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("accountsCard")) {
        RText(
            if (view.sharedAccounts.isEmpty()) {
                "$who will see the $service addresses you tick. It will not see anything in them, and it is told how many it did not get."
            } else {
                "$who can already see ${view.sharedAccounts.size} of your $service addresses. Tick the ones to add."
            },
            RType.sans(14.5f, lineHeight = 20f),
            c.secondary,
            Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
        )
        Card {
            view.accounts.forEachIndexed { i, address ->
                if (i > 0) Hairline(inset = 68.dp)
                val locked = address in view.sharedAccounts
                val ticked = locked || address in ui.draft.selected
                val dim by animateFloatAsState(if (ticked) 1f else 0.42f, tween(280), label = "dim")
                Box(Modifier.testTag("shared:$i")) {
                    CheckRow(
                        checked = ticked,
                        onChange = { on ->
                            viewModel.edit { d -> d.copy(selected = if (on) d.selected + address else d.selected - address) }
                        },
                        enabled = !locked,
                        tag = "acct:$address",
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
                    ) {
                        Row(Modifier.weight(1f).graphicsLayer { alpha = dim }, verticalAlignment = Alignment.CenterVertically) {
                            BlobAvatar(address, size = 36.dp)
                            Spacer(Modifier.width(14.dp))
                            Column(Modifier.weight(1f)) {
                                StrikeText(address, RType.sans(16f, FontWeight.Medium), c.text, struck = !ticked)
                                if (locked) RText("Already shared", RType.sans(12f), c.tertiary, Modifier.padding(top = 2.dp))
                            }
                        }
                    }
                }
            }
        }
        RText(
            "Allowing it here also lets $who ask again later without bothering you, for the time you choose under More options (a month to start with). Asking for an address you kept private still needs your OK.",
            RType.sans(13f, lineHeight = 18f),
            c.tertiary,
            Modifier.padding(horizontal = 4.dp, vertical = 10.dp),
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AccountsPeriod(ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    RText("Let it see them for", RType.sans(13f, FontWeight.SemiBold), c.secondary, Modifier.padding(start = 20.dp, top = 6.dp, bottom = 8.dp))
    FlowRow(
        Modifier.padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        ACCOUNTS_LIFETIMES.forEach { kind ->
            SelectChip(if (kind == LifetimeKind.ONCE) "Just this once" else kind.label, ui.draft.lifetime == kind, Modifier.testTag("lifetime:${kind.name}")) {
                viewModel.edit { it.copy(lifetime = kind) }
            }
        }
    }
}

@Composable
private fun ShortenGrant(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val asked = view.grant?.durationSecs?.toLong() ?: return
    // Steps up to what was asked; the user can only ever shorten it.
    val steps = (listOf(60L, 600L, 3_600L, 6 * 3_600L, 86_400L, 7 * 86_400L, 30 * 86_400L).filter { it < asked } + asked)
    val current = ui.draft.grantSeconds?.coerceAtMost(asked) ?: asked
    val index = steps.indexOfLast { it <= current }.coerceAtLeast(0)
    Column(Modifier.padding(horizontal = 20.dp, vertical = 6.dp)) {
        RText("Allow for", RType.sans(13f, FontWeight.SemiBold), c.secondary)
        RText(durationLabel(steps[index]), RType.sans(22f, FontWeight.SemiBold), c.text, Modifier.padding(top = 2.dp).testTag("grantDuration"))
        if (steps.size > 1) {
            Slider(
                value = index.toFloat(),
                onValueChange = detentAction(index, { it.toInt() }) { v -> viewModel.edit { it.copy(grantSeconds = steps[v.toInt().coerceIn(0, steps.lastIndex)]) } },
                valueRange = 0f..steps.lastIndex.toFloat(),
                steps = (steps.size - 2).coerceAtLeast(0),
                modifier = Modifier.testTag("grantSlider"),
            )
            RText("The request asked for ${durationLabel(asked)}. You can only make it shorter.", RType.sans(12.5f), c.tertiary)
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun StandardOptions(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val draft = ui.draft
    val send = view.kind == ApprovalKind.SEND
    if (!send) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) {
            Card {
                Column(Modifier.padding(16.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        GlyphIcon(Glyph.Clock, c.accent, size = 20.dp)
                        Spacer(Modifier.width(10.dp))
                        RText("Allow all mail for a while", RType.sans(16f, FontWeight.SemiBold), c.text, Modifier.weight(1f))
                    }
                    RText(
                        "Release these and let this AI search and read any of your mail without asking again, then ask again when time is up. Sending is never included.",
                        RType.sans(13.5f, lineHeight = 19f),
                        c.secondary,
                        Modifier.padding(top = 6.dp, bottom = 12.dp),
                    )
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        ALL_MAIL_LIFETIMES.forEach { kind ->
                            SelectChip(kind.label, draft.allMail == kind, Modifier.testTag("allMail:${kind.name}")) {
                                viewModel.edit {
                                    if (it.allMail == kind) it.copy(allMail = null) else it.copy(allMail = kind, selected = view.messages.filter { m -> !m.sensitive }.map { m -> m.id }.toSet())
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if (draft.allMail != null) return
    RText("Remember this for", RType.sans(13f, FontWeight.SemiBold), c.secondary, Modifier.padding(start = 20.dp, top = 14.dp, bottom = 8.dp))
    FlowRow(
        Modifier.padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        LifetimeKind.entries.filter { it != LifetimeKind.MONTH }.forEach { kind ->
            SelectChip(kind.label, draft.lifetime == kind, Modifier.testTag("lifetime:${kind.name}")) {
                viewModel.edit { it.copy(lifetime = kind) }
            }
        }
    }
    if (draft.lifetime == LifetimeKind.USES) {
        RTextField(
            draft.uses.toString(),
            { text -> text.filter(Char::isDigit).take(4).toIntOrNull()?.let { n -> viewModel.edit { it.copy(uses = n) } } },
            "Number of uses (1–$MAX_USES)",
            Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
            tag = "uses",
            mono = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        )
    }
    if (draft.lifetime == LifetimeKind.ONCE) {
        RText("Only this request. Nothing is remembered.", RType.sans(13.5f), c.tertiary, Modifier.padding(start = 20.dp, top = 8.dp))
    } else {
        ScopeBuilder(view, draft, viewModel)
    }
}

@Composable
private fun ScopeBuilder(view: ApprovalView, draft: ApprovalDraft, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val send = view.kind == ApprovalKind.SEND
    RText("What should it cover?", RType.sans(13f, FontWeight.SemiBold), c.secondary, Modifier.padding(start = 20.dp, top = 18.dp, bottom = 8.dp))
    Group {
        if (send) {
            val recipients = recipientAddresses(view)
            recipients.forEachIndexed { i, address ->
                if (i > 0) Hairline()
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        RText(address, RType.mono(14f), c.text, ltr = true)
                        RText(
                            if (address in draft.domainRecipients) "Anyone at @${domainOf(address)}" else "This address only",
                            RType.sans(12.5f),
                            c.secondary,
                            Modifier.padding(top = 2.dp),
                        )
                    }
                    Toggle(address in draft.domainRecipients, Modifier.testTag("domain:$address")) { on ->
                        viewModel.edit { d ->
                            d.copy(domainRecipients = if (on) d.domainRecipients + address else d.domainRecipients - address)
                        }
                    }
                }
            }
        } else {
            CheckRow(
                checked = !draft.similar,
                onChange = { viewModel.edit { it.copy(similar = false) } },
                tag = "onlySelected",
                modifier = Modifier.fillMaxWidth().padding(16.dp),
            ) { RText("Only the emails I ticked", RType.sans(15.5f), c.text) }
            Hairline()
            CheckRow(
                checked = draft.similar,
                onChange = { viewModel.edit { it.copy(similar = true) } },
                tag = "similar",
                modifier = Modifier.fillMaxWidth().padding(16.dp),
            ) { RText("Also allow similar mail", RType.sans(15.5f), c.text) }
            if (draft.similar) {
                val senders = senderAddresses(view, draft.selected)
                if (senders.isEmpty()) {
                    RText("Tick an email to offer its sender.", RType.sans(13f), c.tertiary, Modifier.padding(start = 16.dp, bottom = 12.dp))
                }
                senders.forEach { address ->
                    Hairline()
                    CheckRow(
                        checked = address in draft.senderAddresses,
                        onChange = { on ->
                            viewModel.edit { it.copy(senderAddresses = if (on) it.senderAddresses + address else it.senderAddresses - address) }
                        },
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
                    ) { RText(address, RType.mono(14f), c.text, ltr = true) }
                }
                senders.map(::domainOf).distinct().forEach { domain ->
                    Hairline()
                    CheckRow(
                        checked = domain in draft.senderDomains,
                        onChange = { on ->
                            viewModel.edit { it.copy(senderDomains = if (on) it.senderDomains + domain else it.senderDomains - domain) }
                        },
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
                    ) { RText("Anyone at @$domain", RType.sans(15f), c.text, ltr = true) }
                }
            }
        }
    }
    RTextField(
        draft.subject,
        { text -> viewModel.edit { it.copy(subject = text.take(200)) } },
        "Subject contains (optional)",
        Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
        tag = "subject",
    )
}
