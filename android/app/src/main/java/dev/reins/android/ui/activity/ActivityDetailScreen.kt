package dev.reins.android.ui.activity

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Undo
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.PageDock
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Timers
import dev.reins.android.design.ValueRow
import dev.reins.android.design.pressable
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.core.ActivityEntry
import dev.reins.core.Verdict
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.sp
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.reins.android.AppContainer
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.approval.ApprovalRecord
import dev.reins.android.ui.approval.ApprovalScreen
import dev.reins.android.ui.approval.ApprovalViewModel
import dev.reins.android.ui.autopilot.modeTint
import dev.reins.android.ui.autopilot.RiskChip
import dev.reins.android.ui.common.AskSentenceText
import dev.reins.android.ui.common.IntegrationIcon
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.LocalConnections
import dev.reins.android.ui.common.findConnection
import dev.reins.android.ui.common.untrustedLine
import dev.reins.core.ApprovalView
import dev.reins.core.AutopilotMode
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch

/**
 * One operation, opened: a quiet history header, then the request's facts and pages, read-only (commits, a commit's
 * lines, the emails), what was asked in full, and "This was wrong" where the decision dock was. An older entry uses
 * the same header and shows what it kept.
 */
@Composable
fun ActivityDetailScreen(
    entryId: Long,
    state: AppState,
    onBack: () -> Unit,
    onOpenGrant: (String) -> Unit,
    onOpenEmail: (entryId: Long, index: Int) -> Unit = { _, _ -> },
    /** "This was wrong": records what should have happened; returns an error to show, or null. */
    onCorrect: suspend (Long, Verdict) -> String? = { _, _ -> null },
    /** Reads the approval the entry kept; without it the entry shows only what it kept itself. */
    container: AppContainer? = null,
    authenticator: Authenticator? = null,
) {
    val c = LocalColors.current
    val entries by state.activity.collectAsStateWithLifecycle()
    val grants by state.grants.collectAsStateWithLifecycle()
    val entry = entries.firstOrNull { it.id == entryId }
    val scope = rememberCoroutineScope()
    var asking by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var thanked by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val correctable = entry?.autopilot?.correctable == true
    // The approval as it was shown: read once; until it is known nothing is drawn (no flash of the older page).
    var recorded by remember(entryId) { mutableStateOf<ApprovalView?>(null) }
    var looked by remember(entryId) { mutableStateOf(container == null) }
    LaunchedEffect(entryId, container) {
        val core = container?.core ?: return@LaunchedEffect
        recorded = try {
            val view = core.activityApproval(entryId)
            // A push: the best copy of it on this phone (with its changes line by line where they were kept).
            val push = if (view?.git == null) {
                null
            } else {
                try {
                    core.activityPush(entryId)
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    null
                }
            }
            if (view != null && push != null) view.copy(git = push.git) else view
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            null
        }
        looked = true
    }
    // An entry's commit whose changes are not on this phone loads from the git host, for this entry only.
    val commits = remember(entryId, container) {
        container?.core?.let { core -> dev.reins.android.ui.approval.EntryCommits { sha -> core.activityCommitChange(entryId, sha) } }
    }
    val dock: (@Composable () -> Unit)? = if (entry != null && (correctable || thanked || error != null)) {
        { EntryDock(correctable, thanked, busy, error) { asking = true } }
    } else {
        null
    }

    val shown = recorded
    if (entry != null && shown != null && container != null && authenticator != null) {
        val viewModel = viewModel(key = "entry:$entryId") {
            ApprovalViewModel(container, "entry:$entryId", shown, entry.info.messages.map { it.id }.toSet() + entry.info.accounts)
        }
        ApprovalScreen(
            viewModel = viewModel,
            authenticator = authenticator,
            onDone = onBack,
            record = ApprovalRecord(
                header = {
                    DecidedHeader(entry, askSentence(shown, dev.reins.android.ui.common.entryTense(entry.outcome), label = entry.label, cwd = entry.request?.cwd))
                },
                below = {
                    AskedSections(entry)
                    if (entry.grantId != null || entry.autopilot != null) {
                        SettingsGroup(gap = 14.dp) { DecisionDetails(entry, grants.any { it.id == entry.grantId }, onOpenGrant) }
                    }
                    shown.envRelease?.let { release ->
                        dev.reins.android.ui.grants.RevokeEverywhere(
                            release.project,
                            release.env,
                            { project, env -> dev.reins.android.ui.grants.revokeSecretsEverywhere(container, project, env) },
                            Modifier.padding(top = 18.dp),
                        )
                    }
                },
                footer = dock,
                label = entry.label,
                cwd = entry.request?.cwd,
                commits = commits.takeIf { shown.git != null },
            ),
        )
    } else if (looked) {
        Box(Modifier.fillMaxSize().background(c.background)) {
            Column(Modifier.fillMaxSize().imePadding()) {
                dev.reins.android.ui.approval.HistoryBar(entry?.connectionId.orEmpty(), entry?.connectionLabel.orEmpty(), onBack)
                Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).windowInsetsPadding(WindowInsets.navigationBars)) {
                    if (entry == null) {
                        EmptyState(Glyph.List, "Not found", null, tag = "entryGone")
                    } else {
                        DecidedHeader(entry)
                        AskedSections(entry)
                        Details(entry, grants.any { it.id == entry.grantId }, onOpenGrant, onOpenEmail)
                        Spacer(Modifier.height(if (dock != null) 120.dp else 32.dp))
                    }
                }
            }
            if (dock != null) Box(Modifier.align(Alignment.BottomCenter)) { dock() }
        }
    } else {
        Box(Modifier.fillMaxSize().background(c.background))
    }

    if (asking && entry != null) {
        val should = dev.reins.android.ui.autopilot.correctionFor(entry)
        ConfirmDialog(
            title = if (should == Verdict.DENY) "Should this have been denied?" else "Should this have been approved?",
            text = "Autopilot learns from it. What was done stays done.",
            confirmLabel = if (should == Verdict.DENY) "Deny next time" else "Approve next time",
            destructive = false,
            onConfirm = {
                asking = false
                busy = true
                scope.launch {
                    error = onCorrect(entry.id, should)
                    busy = false
                    thanked = error == null
                }
            },
            onDismiss = { asking = false },
        )
    }
}

/**
 * Where an entry's decision dock was: "This was wrong", a normal neutral button like Deny with Autopilot's violet undo
 * mark, or the thanks once it is said.
 */
@Composable
private fun EntryDock(correctable: Boolean, thanked: Boolean, busy: Boolean, error: String?, onWrong: () -> Unit) {
    val c = LocalColors.current
    PageDock(Modifier.testTag("entryDock")) {
        error?.let { Banner(it, Modifier.padding(bottom = 10.dp), BannerKind.Error) }
        if (thanked) {
            Banner("Learned. It asks about these again.", tag = "corrected")
        } else if (correctable) {
            PillButton(
                "This was wrong",
                Modifier.fillMaxWidth().testTag("thisWasWrong"),
                icon = Icons.AutoMirrored.Rounded.Undo,
                iconTint = modeTint(AutopilotMode.AUTO, c),
                background = if (c.dark) Color(0xFF232327) else Color(0xFFE7E7EB),
                foreground = c.text,
                height = 56.dp,
                textSize = 16f,
                busy = busy,
                onClick = onWrong,
            )
        }
    }
}

/** The glyph beside how an entry ended on its field: from its own outcome, never a tick for one that was refused. */
internal fun outcomeGlyph(entry: ActivityEntry): Glyph = when {
    entry.outcome == "denied" -> if (entry.decidedBy == "lockdown") Glyph.Lock else Glyph.CircleClose
    dev.reins.android.ui.common.entryTense(entry.outcome) == dev.reins.android.ui.common.Tense.Done -> Glyph.CircleCheck
    else -> Glyph.CircleAlert
}

/** A completed request uses paper: its kind and time, the past-tense action, then who decided and any risk. */
@Composable
private fun DecidedHeader(entry: ActivityEntry, sentence: dev.reins.android.ui.common.AskSentence = askSentence(entry)) {
    val c = LocalColors.current
    Column(
        Modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, top = 8.dp, bottom = 12.dp)
            .testTag("entryHero"),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.testTag("entryIcon")) { IntegrationIcon(sentence.provider, sentence.action, size = 40.dp) }
            LabelText("${sentence.kind} · ${whenLine(entry.at).replace(",", "")}", c.inkMeta, Modifier.weight(1f).testTag("detailKind"), maxLines = 2)
        }
        AskSentenceText(sentence, style = RType.sheetTitle, color = c.ink, detailsColor = c.ink2, detailsStyle = RType.value,
            modifier = Modifier.semantics { heading() }, tag = "detailTitle", detailsTag = "detailDetails")
        if (entry.label == null) {
            untrusted(entry.detail).takeIf { it.isNotBlank() }?.let {
                RText(it, RType.sans(15f, lineHeight = 21f), c.ink2, Modifier.testTag("detailSummary"))
            }
        }
        Box(Modifier.testTag("entryAutopilot")) { OutcomeLine(entry) }
        // The risk explanation is never truncated on a history page.
        RiskChip(entry.autopilot?.risk, maxLines = Int.MAX_VALUE)
    }
}

/**
 * The "Who" of what an entry asked: with an agent, it and the computer (or app) it asked through, by name ("Claude
 * Code · M2A"); without, the connection and its [host] (where it connects from).
 */
internal fun askedWho(entry: ActivityEntry, host: String?): String {
    if (!entry.request?.agent?.let(::untrusted).isNullOrBlank()) return askerLine(entry)
    val where = host?.let(::untrusted)?.takeIf { it.isNotBlank() }
    return listOfNotNull(untrusted(entry.connectionLabel), where).distinct().joinToString(" · ")
}

/**
 * What the entry asked: who (the agent and its computer), where it ran, why it was asked and when (with how long the
 * answer took); then the full request, the whole command or tool call wrapped in a mono well with what the agent said,
 * and a copy button.
 */
@Composable
internal fun AskedSections(entry: ActivityEntry) {
    val r = entry.request ?: return
    val c = LocalColors.current
    val host = findConnection(LocalConnections.current, entry.connectionId, entry.connectionLabel)?.clientHost
    val who = askedWho(entry, host)
    SettingsGroup(header = "What it asked", gap = 18.dp) {
        AskedRow("Who", who, Modifier.testTag("askedWho")) { ConnectionIcon(entry.connectionId, entry.connectionLabel, size = 18.dp) }
        if (r.cwd.isNotBlank()) AskedRow("In", untrusted(r.cwd), Modifier.testTag("askedIn"), mono = true)
        if (r.why.isNotBlank()) AskedRow("Why asked", untrusted(r.why), Modifier.testTag("askedWhy"))
        AskedRow("When", whenLine(entry.at) + answeredIn(r.askedAt, entry.at), Modifier.testTag("askedWhen"))
    }
    val full = untrustedLine(r.full).trimEnd()
    val purpose = untrustedLine(r.purpose).trim()
    if (full.isBlank() && purpose.isBlank()) return
    val clipboard = LocalClipboardManager.current
    val feedback = LocalFeedback.current
    Row(Modifier.fillMaxWidth().padding(start = 22.dp, end = 16.dp, top = 18.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        RText("Full request", RType.sans(14f, FontWeight.Medium), c.accent, Modifier.weight(1f), maxLines = 1)
        Box(
            Modifier
                .size(36.dp)
                .clip(CircleShape)
                .pressable(shape = CircleShape, label = "Copy") {
                    clipboard.setText(AnnotatedString(listOf(full, purpose).filter { it.isNotBlank() }.joinToString("\n\n")))
                    feedback.play(Event.Selection)
                }
                .semantics { contentDescription = "Copy the full request" }
                .testTag("copyFullRequest"),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Rounded.ContentCopy, null, Modifier.size(17.dp), tint = c.accent) }
    }
    Column(
        Modifier
            .padding(horizontal = 16.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(if (c.dark) Color(0xFF0A0A0C) else c.elevated)
            .border(1.dp, if (c.dark) Color(0xFF1C1C20) else c.hairline, RoundedCornerShape(16.dp))
            .padding(horizontal = 14.dp, vertical = 12.dp)
            .testTag("fullRequest"),
    ) {
        if (full.isNotBlank()) RText(full, RType.mono(12.5f).copy(lineHeight = 20.sp), c.text, Modifier.testTag("fullRequestText"), ltr = true)
        if (purpose.isNotBlank()) {
            val who = untrusted(r.agent).ifBlank { untrusted(entry.connectionLabel) }
            RText(
                "# $who: “$purpose”",
                RType.mono(12.5f).copy(lineHeight = 20.sp),
                c.tertiary,
                Modifier.padding(top = if (full.isNotBlank()) 12.dp else 0.dp).testTag("fullRequestPurpose"),
                ltr = true,
            )
        }
    }
}

/** A row of "What it asked": the label in a fixed column, the value beside it, wrapping. */
@Composable
private fun AskedRow(label: String, value: String, modifier: Modifier = Modifier, mono: Boolean = false, leading: (@Composable () -> Unit)? = null) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().defaultMinSize(minHeight = 48.dp).padding(horizontal = 16.dp, vertical = 12.dp).semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(label, RType.sans(15f), c.tertiary, Modifier.width(96.dp), maxLines = 1)
        if (leading != null) {
            leading()
            Spacer(Modifier.width(7.dp))
        }
        // Wrapped, never cut: the agent, the folder and the reason are what was asked.
        RText(value, if (mono) RType.mono(13.5f) else RType.sans(15f), c.text, Modifier.weight(1f), ltr = mono)
    }
}

/** " · answered in 9 s", from when it was asked to when it was decided; empty when not known. */
fun answeredIn(askedAt: Long, decidedAt: Long): String {
    if (askedAt <= 0 || decidedAt < askedAt) return ""
    val d = decidedAt - askedAt
    return " · answered in " + when {
        d < 60 -> "$d s"
        d < 3_600 -> "${d / 60} min"
        else -> "${d / 3_600} h"
    }
}

@Composable
private fun OutcomeLine(entry: ActivityEntry) {
    val c = LocalColors.current
    val tint = when (outcomeOf(entry)) {
        Outcome.Auto, Outcome.Overnight, Outcome.Grant -> c.accent
        Outcome.Denied, Outcome.AutoDenied, Outcome.OvernightDenied, Outcome.Bypass -> c.danger
        Outcome.Failed, Outcome.Lockdown, Outcome.Started, Outcome.Unknown, Outcome.Withheld -> c.warning
        Outcome.Approved -> c.ink2
    }
    Row(Modifier.testTag("entryOutcome"),
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
        if (entry.decidedBy.isNotEmpty()) {
            dev.reins.android.design.GlyphIcon(when (entry.decidedBy) {
                "overnight" -> Glyph.Moon
                "lockdown" -> Glyph.Lock
                else -> Glyph.Autopilot
            }, tint, size = 16.dp)
        }
        RText(outcomeLine(entry), RType.sans(14.5f, FontWeight.Medium), tint, Modifier.testTag("entryAutopilotTitle"))
    }
}

private fun whenLine(at: Long): String = dayTime(at, (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000)

@Composable
private fun Details(entry: ActivityEntry, grantKnown: Boolean, onOpenGrant: (String) -> Unit, onOpenEmail: (Long, Int) -> Unit) {
    val c = LocalColors.current
    val info = entry.info
    // Another integration's items keep their text: shown straight away. Emails open from a row.
    var messagesOpen by rememberSaveable { mutableStateOf(entry.op.isNotEmpty()) }
    var accountsOpen by rememberSaveable { mutableStateOf(false) }
    val denied = entry.outcome == "denied"
    SettingsGroup(gap = 14.dp) {
        // "What it asked" says when already.
        if (entry.request == null) ValueRow("When", value = whenLine(entry.at))
        entry.account?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { ValueRow("Account", Modifier.testTag("detailAccount"), value = it, mono = true, valueMaxLines = Int.MAX_VALUE) }
        info.query?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { ValueRow("Search", Modifier.testTag("detailQuery"), value = it, mono = true) }
        if (info.messages.isNotEmpty()) {
            val noun = if (entry.op.isNotEmpty()) "Items" else "Emails"
            ValueRow(
                if (denied) "$noun not shared" else if (dev.reins.android.ui.common.entryTense(entry.outcome) == dev.reins.android.ui.common.Tense.Done) "$noun shared" else "$noun requested",
                Modifier.testTag("emailsShared"),
                value = info.messages.size.toString(),
                open = messagesOpen,
            ) { messagesOpen = !messagesOpen }
            AnimatedVisibility(messagesOpen) { Messages(entry, onOpenEmail) }
        }
        if (info.accounts.isNotEmpty()) {
            ValueRow(
                if (denied) "Accounts not shared" else if (dev.reins.android.ui.common.entryTense(entry.outcome) == dev.reins.android.ui.common.Tense.Done) "Accounts shared" else "Accounts requested",
                Modifier.testTag("accountsShared"),
                value = info.accounts.size.toString(),
                open = accountsOpen,
            ) { accountsOpen = !accountsOpen }
            AnimatedVisibility(accountsOpen) {
                Column {
                    info.accounts.forEachIndexed { i, address ->
                        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("sharedAccount:$i"), verticalAlignment = Alignment.CenterVertically) {
                            BlobAvatar(address, size = 30.dp)
                            Spacer(Modifier.width(12.dp))
                            RText(address, RType.mono(14f), c.text, maxLines = 1, ltr = true)
                        }
                    }
                }
            }
        }
        DecisionDetails(entry, grantKnown, onOpenGrant)
    }

    info.email?.let { email ->
        SettingsGroup(header = "The email") {
            Column(Modifier.padding(16.dp).testTag("detailEmail"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
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
    info.note?.let { note ->
        SettingsGroup(header = "Note") {
            RText(untrusted(note), RType.sans(15f, lineHeight = 21f), c.text, Modifier.padding(16.dp))
        }
    }
    if (entry.grantId == null) {
        info.grantSummary?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { summary ->
            SettingsGroup(header = "Permission") { RText(summary, RType.sans(15f, lineHeight = 21f), c.text, Modifier.padding(16.dp)) }
        }
    }
}

/** The grant and the actual recorded model judgment, shared by recorded and older entries. */
@Composable
private fun DecisionDetails(entry: ActivityEntry, grantKnown: Boolean, onOpenGrant: (String) -> Unit) {
    val c = LocalColors.current
    var whyOpen by rememberSaveable(entry.id) { mutableStateOf(false) }
    entry.grantId?.let { id ->
        ValueRow(
            if (grantKnown) "Grant" else "A grant that ended",
            Modifier.testTag("openGrantFromEntry"),
            value = entry.info.grantSummary?.let(::untrusted),
            onClick = if (grantKnown) ({ onOpenGrant(id) }) else null,
        )
    }
    val note = entry.autopilot
    if (note != null && (note.reason.isNotBlank() || note.neighbours.isNotEmpty() || note.pApprove > 0f || note.pDeny > 0f)) {
        val title = when {
            entry.outcome in setOf("started", "unknown", "succeeded_withheld", "error") -> "What Autopilot thought"
            entry.decidedBy == "autopilot" && entry.outcome == "denied" -> "Why Autopilot denied it"
            entry.decidedBy == "autopilot" -> "Why Autopilot allowed it"
            else -> "What Autopilot thought"
        }
        ValueRow(title, Modifier.testTag("whyAutopilot"), open = whyOpen) { whyOpen = !whyOpen }
        AnimatedVisibility(whyOpen) {
            Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 14.dp).testTag("whyDetail"), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                RText(
                    "Approve ${AutopilotText.percent(note.pApprove)} · Deny ${AutopilotText.percent(note.pDeny)} · ${AutopilotText.name(note.mode)}",
                    RType.sans(13.5f, FontWeight.Medium),
                    c.secondary,
                )
                note.reason.takeIf { it.isNotBlank() }?.let { RText(untrusted(it), RType.sans(14.5f, lineHeight = 20f), c.text) }
                note.neighbours.forEach { RText("· " + untrusted(it), RType.sans(13.5f), c.tertiary, maxLines = 2) }
            }
        }
    }
}

/** What was shared (or held back): one line per email or item; an email opens from Gmail again. */
@Composable
private fun Messages(entry: ActivityEntry, onOpenEmail: (Long, Int) -> Unit) {
    val c = LocalColors.current
    val other = entry.op.isNotEmpty()
    Column(Modifier.padding(bottom = 6.dp)) {
        entry.info.messages.forEachIndexed { i, m ->
            // Only the id is kept, never the text: opening an email fetches it from Gmail again.
            val openable = m.id.isNotEmpty() && !other
            Row(
                Modifier
                    .fillMaxWidth()
                    .testTag("detailMessage:$i")
                    .then(if (openable) Modifier.pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp)) { onOpenEmail(entry.id, i) } else Modifier)
                    .padding(horizontal = 16.dp, vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    RText(untrusted(m.from), RType.sans(14.5f, FontWeight.SemiBold), c.text, maxLines = 1, ltr = true)
                    if (m.subject.isNotEmpty()) RText(untrusted(m.subject), RType.sans(14f), c.secondary, maxLines = 2)
                    // Another integration keeps what was shared, so it can be read here without asking anyone.
                    if (m.text.isNotEmpty()) {
                        RText(untrusted(m.text), RType.sans(14f, lineHeight = 19f), c.secondary, Modifier.padding(top = 2.dp).testTag("detailText:$i"))
                    }
                }
                if (openable) {
                    Spacer(Modifier.width(8.dp))
                    Icon(androidx.compose.material.icons.Icons.Rounded.ChevronRight, null, Modifier.size(18.dp), tint = c.tertiary)
                }
            }
        }
    }
}
