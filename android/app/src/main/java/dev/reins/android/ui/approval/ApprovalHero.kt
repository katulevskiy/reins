package dev.reins.android.ui.approval

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

fun toneColor(tone: HeroTone, c: RColors): Color = when (tone) {
    HeroTone.Write -> c.send
    HeroTone.Search -> c.search
    HeroTone.Send -> c.send
    HeroTone.Grant -> c.accent
    HeroTone.Danger -> c.danger
}

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

/** The hero: tinted by what is asked, the AI → the service, one sentence, and Autopilot's score (or why there is none). */
@Composable
internal fun Hero(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val tone = toneColor(heroTone(view), c)
    val danger = heroTone(view) == HeroTone.Danger
    val shape = RoundedCornerShape(28.dp)
    Column(
        Modifier
            .padding(start = 12.dp, end = 12.dp, top = 4.dp)
            .fillMaxWidth()
            .clip(shape)
            .background(tone.copy(alpha = if (danger) 0.13f else 0.12f))
            .border(1.dp, tone.copy(alpha = if (danger) 0.32f else 0.26f), shape)
            .padding(start = 22.dp, end = 22.dp, top = 26.dp, bottom = 20.dp)
            .testTag("hero"),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Stage(view, tone)
        Sentence(view)
        HeroPills(view, ui, viewModel)
    }
}

/** The AI's avatar, three dots (the last in the hero's colour), the service's logo with the action's badge. */
@Composable
private fun Stage(view: ApprovalView, tone: Color) {
    val c = LocalColors.current
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
        ConnectionIcon(view.connectionId, view.connectionLabel, size = 58.dp)
        Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            repeat(3) { i -> Box(Modifier.size(5.dp).clip(CircleShape).background(if (i == 2) tone else c.tertiary)) }
        }
        ServiceTile(heroService(view), badgeIcon(badge(view)), tone)
    }
}

@Composable
private fun ServiceTile(service: String, badge: ImageVector, tone: Color) {
    val c = LocalColors.current
    val context = LocalContext.current
    val size = 58.dp
    val mark = 30.dp
    val px = with(LocalDensity.current) { mark.roundToPx() }
    val logo = remember(service, px) { service.takeIf { it.isNotBlank() }?.let { SvgAssets.bitmap(context, "services/$it.svg", px)?.asImageBitmap() } }
    val shape = RoundedCornerShape(size * 0.3f)
    Box(Modifier.size(size)) {
        Box(
            Modifier
                .size(size)
                .clip(shape)
                .background(if (c.dark) Color(0xFF1C1C20) else Color.White)
                .border(1.dp, if (c.dark) Color(0xFF2C2C31) else c.hairline, shape)
                .testTag("serviceTile"),
            contentAlignment = Alignment.Center,
        ) {
            if (logo != null) {
                Image(logo, dev.reins.android.design.serviceName(service), Modifier.size(mark), colorFilter = ColorFilter.tint(serviceColor(service, c)))
            } else {
                Icon(Icons.Rounded.Computer, null, Modifier.size(mark), tint = c.secondary)
            }
        }
        val plate = if (c.dark) Color(0xFF111111) else Color.White
        Box(
            Modifier
                .align(Alignment.BottomEnd)
                .offset(x = 6.dp, y = 6.dp)
                .size(25.dp)
                .clip(RoundedCornerShape(8.dp))
                .background(lerp(plate, tone, 0.28f))
                .border(2.5.dp, lerp(plate, tone, 0.08f), RoundedCornerShape(8.dp)),
            contentAlignment = Alignment.Center,
        ) { Icon(badge, null, Modifier.size(13.dp), tint = tone) }
    }
}

/** "Wants to **push 8 commits**\nto **octo/app**": the key nouns bold, the rest quieter. */
@Composable
private fun Sentence(view: ApprovalView) {
    val c = LocalColors.current
    val spans = heroSentence(view)
    val text = buildAnnotatedString {
        spans.forEach { s ->
            if (s.bold) withStyle(SpanStyle(color = c.text, fontWeight = FontWeight.SemiBold)) { append(s.text) } else append(s.text)
        }
    }
    // A question is answered, so it is the question itself (tagged as such).
    val tag = if (view.ask != null) "askQuestion" else "what"
    Box(Modifier.padding(top = 20.dp)) {
        BasicText(
            text,
            Modifier.fillMaxWidth().testTag(tag).semantics(mergeDescendants = true) {},
            style = RType.sans(26f, FontWeight.Medium, lineHeight = 32.5f).copy(color = c.secondary, textAlign = TextAlign.Center),
            maxLines = 5,
        )
    }
}

/** Under the sentence: a danger tag with its (?), Autopilot's score, or the offer to download its model. */
@Composable
private fun HeroPills(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    val feedback = LocalFeedback.current
    val danger = dangerTag(view)
    val s = ui.suggestion
    val model = ui.model
    val record = LocalApprovalRecord.current
    if (record != null) {
        // An Activity entry: how it ended, beside what made it risky.
        Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            if (danger != null) {
                WarnTag(danger.label, if (danger.destructive) c.danger else c.warning, Modifier.testTag(danger.tag), icon = danger.icon)
                Spacer(Modifier.width(8.dp))
            }
            OutcomePill(record.outcome, record.icon, record.tint(c))
        }
        RiskChip(record.risk, Modifier.padding(top = 10.dp))
        return
    }
    if (danger != null) {
        Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            WarnTag(danger.label, if (danger.destructive) c.danger else c.warning, Modifier.testTag(danger.tag), icon = danger.icon)
            Spacer(Modifier.width(8.dp))
            HelpDot(danger.tag, danger.help)
        }
    }
    // The model's risk, and whether Overnight held it: most often on just what cannot be undone.
    RiskChip(s?.risk, Modifier.padding(top = if (danger != null) 10.dp else 16.dp))
    // A guard rule that can be undone is an ordinary question: Autopilot's score shows under it.
    if (danger != null && danger.destructive) return
    val content: (@Composable () -> Unit)? = when {
        s != null && s.judged && !s.floor -> {
            { AutopilotPill(s) { nav.overlay = ApprovalOverlay.Autopilot; feedback.play(Event.Open) } }
        }
        model != null && model.state != ModelState.INSTALLED && s?.floor != true -> {
            { DownloadPill(ui, viewModel::downloadModel) }
        }
        s != null && s.floor -> {
            {
                QuietPill("Always asked", Modifier.testTag("suggestion")) { nav.overlay = ApprovalOverlay.Autopilot; feedback.play(Event.Open) }
            }
        }
        else -> null
    }
    if (content != null) {
        Row(Modifier.padding(top = if (danger != null) 10.dp else 16.dp), verticalAlignment = Alignment.CenterVertically) { content() }
    }
}

/** How an Activity entry ended ("Denied by you", "Allowed by Autopilot · 96%"), on its hero. */
@Composable
private fun OutcomePill(text: String, icon: ImageVector, tint: Color) {
    Row(
        Modifier.clip(RoundedCornerShape(10.dp)).background(tint.copy(alpha = 0.16f)).padding(start = 9.dp, end = 12.dp, top = 7.dp, bottom = 7.dp).testTag("entryOutcome"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(15.dp), tint = tint)
        Spacer(Modifier.width(6.dp))
        RText(text, RType.sans(13f, FontWeight.SemiBold), tint, Modifier.testTag("entryAutopilotTitle"), maxLines = 1)
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

fun dangerTag(view: ApprovalView): DangerTag? {
    view.ask?.let { ask ->
        ask.rule?.let { return ruleTag(it, everyTime = view.noStanding, unread = ask.uncertain) }
        if (ask.uncertain && !ask.destructive) {
            return DangerTag(
                "Couldn't read this",
                "askUncertain",
                "This phone couldn't read the command for sure, so it's asked every time.",
                destructive = false,
                icon = Icons.Rounded.QuestionMark,
            )
        }
    }
    val git = view.git
    if (git != null) {
        val forced = git.refs.indexOfFirst { it.force && !it.forceUnknown }
        if (forced >= 0) return DangerTag("Rewrites history", "forceWarning:$forced", "This push replaces commits that are already there. What they held is lost on the server.")
        val unknown = git.refs.indexOfFirst { it.forceUnknown }
        if (unknown >= 0) return DangerTag("History not checked", "forceUnknown:$unknown", "Reins could not check that this push only adds commits, so it is treated as one that rewrites history.")
        if (git.refs.any { r -> r.commits.any { it.hiddenChars } || r.files.any { it.hiddenChars } }) {
            return DangerTag("Hidden characters", "hiddenChars", "Some text in this push has invisible characters that can make code read unlike what it does. They show as �.")
        }
    }
    val mcp = view.mcp
    if (mcp != null && mcp.destructive) {
        return DangerTag("Destructive", "mcpDestructive", "The server marks this tool as destructive. It is asked for every time and can never be allowed in advance.")
    }
    if (destructive(view)) {
        return DangerTag(
            "Can't be undone",
            "onceWarning",
            if (deletes(view)) "Removes it for good. Autopilot never allows this; you're always asked." else "This reaches far or can't be undone. It's asked every time and never remembered.",
        )
    }
    return null
}

/** A tinted tag with a real icon (the warning sign unless said otherwise). */
@Composable
internal fun WarnTag(text: String, tint: Color, modifier: Modifier = Modifier, small: Boolean = false, icon: ImageVector = Icons.Rounded.Warning) {
    Row(
        modifier
            .clip(CircleShape)
            .background(tint.copy(alpha = 0.14f))
            .padding(start = if (small) 6.dp else 11.dp, end = if (small) 8.dp else 13.dp, top = if (small) 4.dp else 7.dp, bottom = if (small) 4.dp else 7.dp)
            .semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(if (small) 12.dp else 15.dp), tint = tint)
        Spacer(Modifier.width(if (small) 4.dp else 6.dp))
        RText(text, RType.sans(if (small) 11.5f else 13.5f, FontWeight.SemiBold), tint, maxLines = 1)
    }
}

/** "Approve ▬ 97% ›": what Autopilot would do, how sure, in the verdict's colour. Opens its reasons. */
@Composable
private fun AutopilotPill(s: SuggestionView, onOpen: () -> Unit) {
    val c = LocalColors.current
    val tint = verdictTint(s.verdict, c)
    val p = if (s.verdict == Verdict.DENY) s.pDeny else s.pApprove
    Row(
        Modifier
            .clip(CircleShape)
            .background(tint.copy(alpha = 0.12f))
            .pressable(shape = CircleShape, label = "Autopilot", onClick = onOpen)
            .padding(start = 13.dp, end = 8.dp, top = 7.dp, bottom = 7.dp)
            .testTag("suggestion")
            .semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(verdictLabel(s.verdict), RType.sans(13.5f, FontWeight.SemiBold), tint, Modifier.testTag("suggestionHeadline"), maxLines = 1)
        Spacer(Modifier.width(8.dp))
        MiniBar(p, tint)
        Spacer(Modifier.width(8.dp))
        RText(AutopilotText.percent(p), RType.sans(13.5f, FontWeight.SemiBold), tint, maxLines = 1)
        Spacer(Modifier.width(4.dp))
        Icon(Icons.Rounded.ChevronRight, null, Modifier.size(16.dp), tint = tint.copy(alpha = 0.67f))
    }
}

/** Approve, Deny, or Unsure when Autopilot would ask. */
fun verdictLabel(v: Verdict): String = when (v) {
    Verdict.APPROVE -> "Approve"
    Verdict.DENY -> "Deny"
    Verdict.ASK -> "Unsure"
}

@Composable
private fun MiniBar(p: Float, tint: Color) {
    Box(Modifier.width(28.dp).height(4.dp).clip(RoundedCornerShape(2.dp)).background(Color.White.copy(alpha = 0.12f))) {
        Box(Modifier.fillMaxHeight().fillMaxWidth(p.coerceIn(0f, 1f)).clip(RoundedCornerShape(2.dp)).background(tint))
    }
}

/** No model on this phone: "Get Autopilot's score [Download]"; while it downloads, how far. */
@Composable
private fun DownloadPill(ui: ApprovalUi, onDownload: () -> Unit) {
    val c = LocalColors.current
    val blue = c.search
    val model = ui.model ?: return
    val downloading = model.state == ModelState.DOWNLOADING
    val waiting = !downloading && ui.downloadJob == DownloadJob.Waiting
    Row(
        Modifier
            .clip(CircleShape)
            .background(blue.copy(alpha = 0.12f))
            .padding(start = 13.dp, end = 6.dp, top = 6.dp, bottom = 6.dp)
            .testTag("getAutopilot"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when {
            downloading -> {
                val total = model.sizeBytes.toLong()
                val pct = if (total > 0) (model.downloadedBytes.toLong() * 100 / total).toInt().coerceIn(0, 100) else null
                RText(if (pct != null) "Downloading… $pct%" else "Downloading…", RType.sans(13f, FontWeight.SemiBold), blue, Modifier.padding(end = 7.dp).testTag("modelProgress"), maxLines = 1)
            }
            waiting -> RText("Waiting for Wi-Fi", RType.sans(13f, FontWeight.SemiBold), blue, Modifier.padding(end = 7.dp).testTag("modelProgress"), maxLines = 1)
            else -> {
                RText("Get Autopilot's score", RType.sans(13f, FontWeight.SemiBold), blue, maxLines = 1)
                Spacer(Modifier.width(8.dp))
                val ink = if (c.dark) Color(0xFF04122A) else Color.White
                Row(
                    Modifier
                        .clip(CircleShape)
                        .background(blue)
                        .pressable(shape = CircleShape, label = "Download Autopilot's model", onClick = onDownload)
                        .padding(start = 8.dp, end = 11.dp, top = 4.dp, bottom = 4.dp)
                        .testTag("downloadModel"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Rounded.Download, null, Modifier.size(14.dp), tint = ink)
                    Spacer(Modifier.width(4.dp))
                    RText("Download", RType.sans(12.5f, FontWeight.SemiBold), ink, maxLines = 1)
                }
            }
        }
    }
}

@Composable
private fun QuietPill(text: String, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        modifier
            .clip(CircleShape)
            .background(c.text.copy(alpha = 0.08f))
            .pressable(shape = CircleShape, onClick = onClick)
            .padding(start = 13.dp, end = 8.dp, top = 7.dp, bottom = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(text, RType.sans(13.5f, FontWeight.SemiBold), c.secondary, maxLines = 1)
        Spacer(Modifier.width(4.dp))
        Icon(Icons.Rounded.ChevronRight, null, Modifier.size(16.dp), tint = c.tertiary)
    }
}

/** The time the AI still waits: a ring that empties and the seconds, red in the last 15; "Stopped" once it gave up. */
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
                .background(c.text.copy(alpha = 0.07f))
                .onGloballyPositioned { bottom = it.positionInRoot().y + it.size.height }
                .pressable(shape = CircleShape, label = "Stopped waiting") {
                    nav.help = if (nav.help?.key == "late") null else HelpTip("late", "$label stopped waiting. You can still approve; then ask $label to try again.", bottom)
                }
                .padding(start = 9.dp, end = 11.dp, top = 7.dp, bottom = 7.dp)
                .testTag("lateBanner"),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Rounded.TimerOff, null, Modifier.size(17.dp), tint = c.secondary)
            Spacer(Modifier.width(6.dp))
            RText("Stopped", RType.sans(14f, FontWeight.SemiBold), c.secondary, maxLines = 1)
        }
        return
    }
    val tint = if (u.urgent) c.danger else c.secondary
    Row(
        Modifier
            .clip(CircleShape)
            .background(if (u.urgent) c.danger.copy(alpha = 0.13f) else c.text.copy(alpha = 0.07f))
            .padding(start = 8.dp, end = 11.dp, top = 7.dp, bottom = 7.dp)
            .testTag("waitLine")
            .semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Ring(u.fraction, tint, if (u.urgent) c.danger.copy(alpha = 0.22f) else c.text.copy(alpha = 0.14f), 20.dp)
        Spacer(Modifier.width(7.dp))
        RText(secondsLabel(u.remainingSeconds), RType.sans(14f, FontWeight.SemiBold).copy(fontFeatureSettings = "tnum"), tint, maxLines = 1)
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
