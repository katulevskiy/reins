package dev.reins.android.ui.autopilot

import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import dev.reins.android.design.BackArrow
import dev.reins.android.design.MenuItem
import dev.reins.android.design.NextPill
import dev.reins.android.design.PaneGroup
import dev.reins.android.design.PaneRow
import dev.reins.android.design.SectionHeader
import dev.reins.android.design.SignalMenu
import dev.reins.android.design.fieldSurface
import dev.reins.android.design.pane
import dev.reins.android.design.pressable
import dev.reins.android.ui.activity.rowAction
import dev.reins.android.ui.activity.ruleAbove
import dev.reins.android.ui.common.IntegrationIcon
import dev.reins.android.ui.common.actionField
import dev.reins.android.ui.common.serifLine
import dev.reins.android.ui.common.untrustedName
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.sp
import dev.reins.android.design.AskText
import dev.reins.android.design.LabelText
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalButton
import dev.reins.android.design.SignalChip
import dev.reins.android.ui.main.HeaderInsets
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SectionLabel
import dev.reins.android.design.SelectChip
import dev.reins.android.ui.common.untrusted
import dev.reins.core.SuggestionView
import dev.reins.core.Verdict

/**
 * "Try it" (Autopilot 06): pick a case (or type one in the situation format the model reads, spec §4), see the card as
 * it would arrive, and what the profile would do with it: allow, deny or leave it to you, and why. Nothing is kept.
 */
@Composable
fun TryItScreen(viewModel: AutopilotViewModel, initialProfileId: String?, onBack: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.showingProfiles() }
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    var text by rememberSaveable { mutableStateOf(AutopilotText.examples.first().situation) }
    var example by rememberSaveable { mutableStateOf(AutopilotText.examples.first().title) }
    var profileId by rememberSaveable { mutableStateOf(initialProfileId) }
    var profileMenu by remember { mutableStateOf(false) }
    val profile = ui.profiles.firstOrNull { it.id == profileId } ?: ui.profiles.firstOrNull { it.isDefault }
    val modelReady = AutopilotText.modelUsable(ui.model ?: settings?.model)
    val case = remember(example, text) { AutopilotText.examples.firstOrNull { it.title == example } ?: AutopilotText.caseOf(text) }
    val who = remember(text) { AutopilotText.caseWho(text) }
    LaunchedEffect(Unit) { viewModel.clearEvaluation() }

    Column(Modifier.fillMaxSize().background(c.paper).verticalScroll(rememberScrollState()).windowInsetsPadding(HeaderInsets)) {
        Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 16.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            BackArrow(onBack)
            Spacer(Modifier.weight(1f))
            if (profile != null) {
                Box {
                    Row(
                        Modifier
                            .clip(CircleShape)
                            .background(c.paperSunken)
                            .pressable(shape = CircleShape, label = "Profile", enabled = ui.profiles.size > 1) { profileMenu = true }
                            .testTag("tryProfile")
                            .padding(horizontal = 12.dp, vertical = 7.dp),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        GlyphIcon(Glyph.Person, c.ink2, size = 16.dp)
                        RText(untrusted(profile.name), RType.sans(13.5f, FontWeight.Medium), c.ink, maxLines = 1)
                        if (ui.profiles.size > 1) GlyphIcon(Glyph.ChevronDown, c.inkMeta, size = 16.dp)
                    }
                    SignalMenu(
                        expanded = profileMenu,
                        onDismiss = { profileMenu = false },
                        width = 220.dp,
                        items = ui.profiles.map { p ->
                            MenuItem(untrusted(p.name), glyph = Glyph.Person, selected = p.id == profile.id, tag = "tryProfile:${p.id}") {
                                profileId = p.id
                                viewModel.clearEvaluation()
                            }
                        },
                    )
                }
            }
        }
        RText("Try it", RType.pageTitle, c.ink, Modifier.padding(start = 20.dp, end = 20.dp, top = 12.dp).semantics { heading() }.testTag("pageTitle"))
        if (!modelReady) {
            Banner("Download the model first.", Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), BannerKind.Warning, tag = "tryNoModel")
        }
        Row(
            Modifier.padding(top = 14.dp).horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            AutopilotText.examples.forEach { e ->
                SignalChip(e.short, example == e.title, Modifier.testTag("example:${e.title}")) {
                    example = e.title
                    text = e.situation
                    viewModel.clearEvaluation()
                }
            }
        }

        SectionHeader("Comes in", glyph = Glyph.Tray)
        CaseCard(case, who, ui.evaluation, Modifier.padding(horizontal = 16.dp))

        ui.evaluation?.let { s ->
            Column(Modifier.testTag("verdict")) { Outcome(s, case, who) }
        }
        ui.error?.let { Banner(it, Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), BannerKind.Error, tag = "tryError") }

        // The case as Autopilot reads it, to change before asking again.
        SectionHeader("As Autopilot reads it", glyph = Glyph.FileCode)
        Box(Modifier.padding(horizontal = 16.dp).fillMaxWidth().pane().padding(14.dp)) {
            BasicTextField(
                text,
                {
                    text = it.take(4_000)
                    example = ""
                },
                Modifier.fillMaxWidth().defaultMinSize(minHeight = 120.dp).testTag("situation"),
                textStyle = RType.mono(12.5f).copy(color = c.ink, lineHeight = androidx.compose.ui.unit.TextUnit(18f, androidx.compose.ui.unit.TextUnitType.Sp)),
                cursorBrush = SolidColor(c.violet),
            )
        }
        SignalButton(
            "Ask Autopilot",
            Modifier.padding(horizontal = 16.dp, vertical = 14.dp).fillMaxWidth().testTag("evaluate"),
            enabled = text.isNotBlank() && !ui.evaluating,
        ) { viewModel.evaluate(profile?.id, text) }
        Spacer(Modifier.height(40.dp))
    }
}

/**
 * The case as it would arrive: on its field, its icon and who asks, the ask large, the detail in mono, and the card's
 * buttons as they would read (Allow with Autopilot's sureness once asked). Only a picture: nothing here answers.
 */
@Composable
private fun CaseCard(case: AutopilotText.Example, who: String, s: SuggestionView?, modifier: Modifier = Modifier) {
    val field = actionField(case.action)
    val sure = s?.takeIf { it.judged && it.verdict == Verdict.APPROVE }?.let { " " + AutopilotText.percent(it.pApprove) }.orEmpty()
    Column(
        modifier
            .fillMaxWidth()
            .fieldSurface(field, RoundedCornerShape(24.dp))
            .clearAndSetSemantics { contentDescription = listOf(serifLine(who), case.ask, case.detail.orEmpty()).filter { it.isNotBlank() }.joinToString(". ") }
            .padding(16.dp)
            .testTag("tryCase"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            IntegrationIcon(case.provider, case.action, size = 28.dp, ring = field.fill(), field = field)
            if (who.isNotBlank()) RText(untrustedName(who, 32), RType.sans(13.5f, FontWeight.Medium), Color.White.copy(alpha = 0.86f), maxLines = 1)
        }
        RText(serifLine(case.ask), RType.display(28f), Color.White, Modifier.padding(top = 10.dp), maxLines = 3)
        case.detail?.let { RText(serifLine(it), RType.mono(12.5f), Color.White.copy(alpha = 0.8f), Modifier.padding(top = 4.dp), maxLines = 2, ltr = true) }
        Row(Modifier.padding(top = 14.dp).fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.weight(1f).height(38.dp).clip(CircleShape).background(Color.White), contentAlignment = Alignment.Center) {
                RText("Allow$sure", RType.sans(14f, FontWeight.SemiBold), Color(0xFF16151B), maxLines = 1)
            }
            Box(Modifier.weight(1f).height(38.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.2f)), contentAlignment = Alignment.Center) {
                RText("Review", RType.sans(14f, FontWeight.SemiBold), Color.White, maxLines = 1)
            }
        }
    }
}

/**
 * What Autopilot does with it: "Autopilot allows it" (or denies it, or leaves it to you), the row exactly as Activity
 * would show it, and Why: its reason, what held it back, and the decisions of yours it is like.
 */
@Composable
private fun Outcome(s: SuggestionView, case: AutopilotText.Example, who: String) {
    val c = LocalColors.current
    val verdict = if (s.judged) s.verdict else Verdict.ASK
    val (head, tint) = when (verdict) {
        Verdict.APPROVE -> "Autopilot allows it" to c.violet
        Verdict.DENY -> "Autopilot denies it" to c.danger
        Verdict.ASK -> "Not sure: it waits for you" to c.inkMeta
    }
    val sure = if (verdict == Verdict.DENY) s.pDeny else s.pApprove
    SectionHeader(
        head,
        glyph = when (verdict) {
            Verdict.APPROVE -> Glyph.ArrowDown
            Verdict.DENY -> Glyph.ArrowDown
            Verdict.ASK -> Glyph.Help
        },
        modifier = Modifier.testTag("verdictWord"),
        ink = tint,
    )
    PaneGroup {
        val field = actionField(case.action)
        PaneRow(
            title = rowAction(serifLine(case.ask)),
            titleLead = untrustedName(who, 32),
            leading = { IntegrationIcon(case.provider, case.action, size = 36.dp, ring = c.paperRaised, field = field) },
            trailing = {
                Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    RText("now", RType.sans(12.5f, FontWeight.Medium), c.inkMeta, maxLines = 1)
                    when (verdict) {
                        Verdict.APPROVE -> NextPill("Auto " + AutopilotText.percent(sure), c.violet, glyph = Glyph.AutopilotFilled)
                        Verdict.DENY -> NextPill("Auto " + AutopilotText.percent(sure), c.danger, glyph = Glyph.Close)
                        Verdict.ASK -> NextPill("Waits", null, glyph = Glyph.Timer)
                    }
                }
            },
            first = true,
        )
    }
    val notes = AutopilotText.suggestionNotes(s)
    if (s.reason.isNotBlank() || notes.isNotEmpty() || s.neighbours.isNotEmpty()) {
        SectionHeader("Why")
        PaneGroup {
            var first = true
            if (s.reason.isNotBlank() && s.judged) {
                PaneRow(untrusted(s.reason), leading = { GlyphIcon(Glyph.Info, c.ink2, size = 18.dp) }, first = first, tag = "verdictReason")
                first = false
            }
            notes.forEach { note ->
                PaneRow(note, leading = { GlyphIcon(Glyph.Warning, c.warning, size = 18.dp) }, first = first, tag = if (!s.judged && note == s.reason) "verdictReason" else null)
                first = false
            }
            s.neighbours.forEach { n ->
                Box(Modifier.then(if (first) Modifier else Modifier.ruleAbove(c, 14.dp)).padding(horizontal = 14.dp)) { NeighbourRow(n) }
                first = false
            }
        }
    }
}
