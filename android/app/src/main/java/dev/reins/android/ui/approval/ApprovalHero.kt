package dev.reins.android.ui.approval

import dev.reins.core.ApprovalKind

import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.ui.common.findConnection

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.List
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.People
import androidx.compose.material.icons.rounded.QuestionMark
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.ShoppingCart
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.TimerOff
import androidx.compose.material.icons.rounded.VerifiedUser
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material.icons.rounded.WarningAmber
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.DownloadJob
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SvgAssets
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowState
import dev.reins.android.design.serviceColor
import dev.reins.android.design.urgency
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.autopilot.verdictTint
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ApprovalView
import dev.reins.core.AskRuleView
import dev.reins.core.ModelState
import dev.reins.core.SuggestionView
import dev.reins.core.Verdict
import dev.reins.android.ui.autopilot.RiskChip

/** The hero's colour on paper: its Signal field's kind ink. */
fun toneColor(tone: HeroTone, c: RColors): Color = tone.field.kindInk(c)

fun badgeIcon(badge: Badge): ImageVector = when (badge) {
    Badge.Pen -> Icons.Rounded.Edit
    Badge.Search -> Icons.Rounded.Search
    Badge.Read -> Icons.Rounded.Visibility
    Badge.List -> Icons.AutoMirrored.Rounded.List
    Badge.Send -> Icons.AutoMirrored.Rounded.Send
    Badge.Trash -> Icons.Rounded.Delete
    Badge.Shield -> Icons.Rounded.VerifiedUser
    Badge.People -> Icons.Rounded.People
    Badge.Key -> Icons.Rounded.Key
    Badge.Terminal -> Icons.Rounded.Terminal
    Badge.Question -> Icons.Rounded.QuestionMark
    Badge.Cart -> Icons.Rounded.ShoppingCart
}

/**
 * The ask as a field (Reins Signal): who asks and how long it waits, the label line ("GITHUB · PUSH"), the ask in the
 * serif with its target in italic, and one verdict line: Autopilot's opinion, why it is always asked, the risk
 * Overnight saw.
 */
@Composable
internal fun Hero(
    view: ApprovalView,
    ui: ApprovalUi,
    viewModel: ApprovalViewModel,
    onClose: () -> Unit,
    back: Boolean = false,
    entered: androidx.compose.animation.core.MutableTransitionState<Boolean>? = null,
) {
    val nav = LocalApprovalNav.current
    val host = findConnection(dev.reins.android.ui.common.LocalConnections.current, view.connectionId, view.connectionLabel)?.clientHost?.let(::untrusted)
    // A waiting request starts with the verb; the asker is in the top row.
    val label = viewModel.pendingLabel()
    val sentence = androidx.compose.runtime.remember(view, label) {
        dev.reins.android.ui.common.askSentence(view, label = label)
    }
    val field = askField(view)
    AskField(
        field,
        sentence.kind,
        sentence.text,
        sentence.target,
        details = sentence.details,
        onClose = onClose,
        askTag = if (view.ask != null) "askQuestion" else "what",
        closeGlyph = if (back) Glyph.ArrowBack else Glyph.Close,
        closeLabel = if (back) "Back" else "Close",
        top = {
            AskWho(untrusted(view.connectionLabel), host) { ConnectionIcon(view.connectionId, view.connectionLabel, size = 28.dp) }
            TimerPill(view)
        },
        verdict = { HeroVerdict(view, ui, viewModel, nav) },
        entered = entered,
    )
}

/**
 * The field's verdict lines. Every warning the request carries is shown here, in this order and never cut short:
 * what makes it dangerous or asked every time (with its (?) for the longer reason), then the risk Overnight's model
 * saw and whether Overnight held it (two lines at most; if longer, the risk's word stays here and its reason becomes
 * the first fact row). Then Autopilot's opinion (not for what can't be undone), or the offer to get its model.
 * History entries use their own paper header.
 */
@Composable
private fun HeroVerdict(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel, nav: ApprovalNav) {
    val feedback = LocalFeedback.current
    val danger = dangerTag(view)
    val s = ui.suggestion
    val model = ui.model
    val risk = s?.risk
    val openAutopilot = {
        nav.overlay = ApprovalOverlay.Autopilot
        feedback.play(Event.Open)
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        // Every warning, each on its own line (see fieldWarnings).
        fieldWarnings(view, s, risk, nav.riskInFacts).forEach { w ->
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                val isRisk = w.tag == "risk"
                FieldVerdict(
                    w.text,
                    Modifier.weight(1f, fill = false).testTag(w.tag),
                    glyph = w.glyph,
                    maxLines = w.maxLines,
                    textTag = if (isRisk) "riskLine" else null,
                    onClick = if (isRisk && s != null && s.judged) openAutopilot else null,
                    onOverflow = if (isRisk) { over -> if (over && !nav.riskInFacts) nav.riskInFacts = true } else { _ -> },
                )
                if (w.help != null) HelpDot(w.tag, w.help, size = 26.dp, onField = true)
            }
        }
        if (view.kind == ApprovalKind.GRANT) FieldVerdict("Nothing is shared now. Matches are answered for you.", Modifier.testTag("grantLine"))
        if (heldByDanger(view)) return@Column
        when {
            s != null && s.judged && !s.floor -> {
                val p = if (s.verdict == Verdict.DENY) s.pDeny else s.pApprove
                val words = when (s.verdict) {
                    Verdict.APPROVE -> "Autopilot would allow"
                    Verdict.DENY -> "Autopilot would deny"
                    Verdict.ASK -> "Autopilot is unsure"
                }
                FieldVerdict(
                    "$words · ${AutopilotText.percent(p)}",
                    Modifier.testTag("suggestion"),
                    glyph = if (s.verdict == Verdict.ASK) null else Glyph.Autopilot,
                    ring = if (s.verdict == Verdict.ASK) p else null,
                    chevron = true,
                    textTag = "suggestionHeadline",
                    onClick = openAutopilot,
                )
            }
            s != null && s.floor && danger == null && alwaysAskReason(view) == null ->
                FieldVerdict("Always asks", Modifier.testTag("suggestion"), glyph = Glyph.Lock, chevron = true, onClick = openAutopilot)
            model != null && !AutopilotText.modelUsable(model) && s?.floor != true -> DownloadLine(ui, viewModel::downloadModel)
            else -> Unit
        }
    }
}

/** A warning on the field: "Always asks · can't be undone", "Always asks · rewrites history", or the tag itself. */
internal fun dangerLine(d: DangerTag): String = if (d.destructive) "Always asks · ${dangerWords(d)}" else d.label

/** "can't be undone", "rewrites history": what a destructive tag says after "Always asks". */
private fun dangerWords(d: DangerTag): String = when (d.tag) {
    "onceWarning", "mcpDestructive" -> "can't be undone"
    else -> d.label.replaceFirstChar { it.lowercase() }
}

/** No model on this phone: "Get Autopilot's score" and a white Download; while it downloads, how far. */
@Composable
private fun DownloadLine(ui: ApprovalUi, onDownload: () -> Unit) {
    val c = LocalColors.current
    val model = ui.model ?: return
    val downloading = model.state == ModelState.DOWNLOADING
    val waiting = !downloading && ui.downloadJob == DownloadJob.Waiting
    Row(Modifier.testTag("getAutopilot"), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        when {
            downloading -> {
                val total = model.sizeBytes.toLong()
                val pct = if (total > 0) (model.downloadedBytes.toLong() * 100 / total).toInt().coerceIn(0, 100) else null
                FieldVerdict(if (pct != null) "Downloading Autopilot · $pct%" else "Downloading Autopilot…", Modifier.testTag("modelProgress"), glyph = Glyph.Download)
            }
            waiting -> FieldVerdict("Autopilot waits for Wi-Fi", Modifier.testTag("modelProgress"), glyph = Glyph.Wifi)
            else -> {
                FieldVerdict("Get Autopilot's score", glyph = Glyph.Autopilot)
                dev.reins.android.design.SignalButton(
                    "Download",
                    Modifier.testTag("downloadModel"),
                    style = dev.reins.android.design.ButtonStyle.OnFieldWhite,
                    size = dev.reins.android.design.ButtonSize.Compact,
                    onClick = onDownload,
                )
            }
        }
    }
}

/** A tag on the hero for what cannot be undone (red, held to approve) or a guard rule that matched (amber), and what its (?) says. */
data class DangerTag(
    val label: String,
    val tag: String,
    val help: String,
    val destructive: Boolean = true,
    val icon: ImageVector = Icons.Rounded.Warning,
)

/** The guard rule a desktop question matched, and what matched it: ".env files · .env". */
fun ruleTag(rule: AskRuleView, everyTime: Boolean = false, unread: Boolean = false): DangerTag {
    val matched = untrusted(rule.matched)
    val label = if (matched.isEmpty()) rule.label else "${rule.label} · $matched"
    val help = buildString {
        append("Matched the rule ${untrusted(rule.pattern)} on your computer.")
        if (rule.reported) append(" Your computer says so; this phone doesn't know the rule.")
        if (unread) append(" This phone couldn't read all of the command.")
        append(
            when {
                rule.destructive -> " It can't be undone, so it's asked every time."
                everyTime -> " It's asked every time."
                else -> " Autopilot can learn it."
            },
        )
    }
    return DangerTag(label, "guardRule", help, rule.destructive, Icons.Rounded.Shield)
}

/** The first of [dangerTags]: what the row in a list and Autopilot go by. */
fun dangerTag(view: ApprovalView): DangerTag? = dangerTags(view).firstOrNull()

/**
 * Every warning the request carries, most telling first: the guard rule it matched, a command this phone couldn't
 * read, a push that rewrites history (or that couldn't be checked), hidden characters, a destructive tool, and what
 * can't be undone. The field shows them all, in full.
 */
fun dangerTags(view: ApprovalView): List<DangerTag> = buildList {
    val ask = view.ask
    val unread = DangerTag(
        "Couldn't read this",
        "askUncertain",
        "This phone couldn't read the command for sure, so it's asked every time.",
        destructive = false,
        icon = Icons.Rounded.QuestionMark,
    )
    ask?.rule?.let { add(ruleTag(it, everyTime = view.noStanding, unread = ask.uncertain)) }
    if (ask != null && ask.uncertain && ask.rule == null && !ask.destructive) add(unread)
    val push = view.git
    if (push != null) {
        val forced = push.refs.indexOfFirst { it.force && !it.forceUnknown }
        if (forced >= 0) add(DangerTag("Rewrites history", "forceWarning:$forced", "This push replaces commits that are already there. What they held is lost on the server."))
        val unknown = push.refs.indexOfFirst { it.forceUnknown }
        if (unknown >= 0) add(DangerTag("History not checked", "forceUnknown:$unknown", "Reins could not check that this push only adds commits, so it is treated as one that rewrites history."))
        if (push.refs.any { r -> r.commits.any { it.hiddenChars } || r.files.any { it.hiddenChars } }) {
            add(DangerTag("Hidden characters", "hiddenChars", "Some text in this push has invisible characters that can make code read unlike what it does. They show as �."))
        }
    }
    val mcp = view.mcp
    if (mcp != null && mcp.destructive) {
        add(DangerTag("Destructive", "mcpDestructive", "The server marks this tool as destructive. It is asked for every time and can never be allowed in advance."))
    }
    if (destructive(view) && none { it.destructive }) {
        add(
            DangerTag(
                "Can't be undone",
                "onceWarning",
                if (deletes(view)) "Removes it for good. Autopilot never allows this; you're always asked." else "This reaches far or can't be undone. It's asked every time and never remembered.",
            ),
        )
    }
    // A command this phone couldn't read is said on its own line too, not only behind the rule's (?).
    if (ask != null && ask.uncertain && (ask.rule != null || ask.destructive)) add(unread)
}

/** A tinted tag with a real icon (the warning sign unless said otherwise). */
@Composable
internal fun WarnTag(
    text: String,
    tint: Color,
    modifier: Modifier = Modifier,
    small: Boolean = false,
    icon: ImageVector = Icons.Rounded.Warning,
    maxLines: Int = 1,
) {
    Row(
        modifier
            .clip(if (maxLines > 1) RoundedCornerShape(if (small) 11.dp else 16.dp) else CircleShape)
            .background(tint.copy(alpha = 0.14f))
            .padding(start = if (small) 6.dp else 11.dp, end = if (small) 8.dp else 13.dp, top = if (small) 4.dp else 7.dp, bottom = if (small) 4.dp else 7.dp)
            .semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(if (small) 12.dp else 15.dp), tint = tint)
        Spacer(Modifier.width(if (small) 4.dp else 6.dp))
        RText(text, RType.sans(if (small) 11.5f else 13.5f, FontWeight.SemiBold), tint, maxLines = maxLines)
    }
}

/**
 * What deserves a closer look in one commit, file or email: the warning sign and its 1 to 3 reasons. Close is the
 * filled sign in the warning colour; Look is quieter, the outline sign in a muted ink, so "Changes CI" reads as more
 * serious than "Runs on build". Red stays for danger. TalkBack reads "Look closer: Changes CI".
 */
@Composable
internal fun AttentionTag(mark: AttentionMark, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    WarnTag(
        mark.text,
        if (mark.level == dev.reins.core.AttentionLevel.CLOSE) c.warning else c.secondary,
        modifier.clearAndSetSemantics { contentDescription = mark.spoken },
        small = true,
        icon = if (mark.level == dev.reins.core.AttentionLevel.CLOSE) Icons.Rounded.Warning else Icons.Rounded.WarningAmber,
        maxLines = 2,
    )
}

/** Approve, Deny, or Unsure when Autopilot would ask. */
fun verdictLabel(v: Verdict): String = when (v) {
    Verdict.APPROVE -> "Approve"
    Verdict.DENY -> "Deny"
    Verdict.ASK -> "Unsure"
}

/**
 * The time the AI still waits, on the field: a ring that drains each second and m:ss; a white pill with red digits in
 * the last 15 s; "Stopped" once it gave up (a tap says what that means). Its own clock.
 */
@Composable
internal fun TimerPill(view: ApprovalView) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    val now by rememberNowState(500, untilMillis = view.waitUntil?.let { it * 1000 })
    val u = urgency(view.createdAt, view.waitUntil, now) ?: return
    val label = untrusted(view.connectionLabel)
    var bottom by remember { mutableFloatStateOf(0f) }
    if (u.stale) {
        Row(
            Modifier
                .clip(CircleShape)
                .background(c.onFieldWash)
                .onGloballyPositioned { bottom = it.positionInRoot().y + it.size.height }
                .pressable(shape = CircleShape, label = "Stopped waiting") {
                    nav.help = if (nav.help?.key == "late") null else HelpTip("late", "$label stopped waiting. You can still approve; then ask $label to try again.", bottom)
                }
                .padding(start = 9.dp, end = 11.dp, top = 6.dp, bottom = 6.dp)
                .testTag("lateBanner"),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            GlyphIcon(Glyph.TimerOff, c.onField, size = 16.dp)
            RText("Stopped", RType.sans(13.5f, FontWeight.SemiBold), c.onField, maxLines = 1)
        }
        return
    }
    Box(Modifier.testTag("waitLine").semantics(mergeDescendants = true) {}) {
        dev.reins.android.design.TimerPill(u.remainingSeconds, ((view.waitUntil ?: 0) - view.createdAt).coerceAtLeast(1))
    }
}

/** 40 → "40s", 125 → "2m", 7200 → "2h". */
fun secondsLabel(secs: Long): String = when {
    secs < 100 -> "${secs}s"
    secs < 3_600 -> "${(secs + 59) / 60}m"
    else -> "${(secs + 3_599) / 3_600}h"
}

@Composable
internal fun Ring(fraction: Float, color: Color, track: Color, size: Dp, stroke: Dp = 2.5.dp) {
    Canvas(Modifier.size(size)) {
        val w = stroke.toPx()
        val arc = Size(this.size.width - w * 2, this.size.height - w * 2)
        val at = Offset(w, w)
        drawArc(track, 0f, 360f, false, at, arc, style = Stroke(w))
        drawArc(color, -90f, 360f * fraction.coerceIn(0f, 1f), false, at, arc, style = Stroke(w, cap = StrokeCap.Round))
    }
}
