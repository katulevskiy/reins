package dev.reins.android.ui.autopilot

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.background
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
import dev.reins.core.ModelState
import dev.reins.core.SuggestionView
import dev.reins.core.Verdict

/**
 * "Try it": a request typed in the situation format the model reads (spec §4), and what a profile would do with it:
 * the verdict, how sure, the decisions it is like, and why. Nothing is kept.
 */
@Composable
fun TryItScreen(viewModel: AutopilotViewModel, initialProfileId: String?, onBack: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    var text by rememberSaveable { mutableStateOf(AutopilotText.examples.first().situation) }
    var example by rememberSaveable { mutableStateOf(AutopilotText.examples.first().title) }
    var profileId by rememberSaveable { mutableStateOf(initialProfileId) }
    val profile = ui.profiles.firstOrNull { it.id == profileId } ?: ui.profiles.firstOrNull { it.isDefault }
    val modelReady = (ui.model ?: settings?.model)?.state == ModelState.INSTALLED
    LaunchedEffect(Unit) { viewModel.clearEvaluation() }

    Screen(title = "Try it", subtitle = profile?.let { untrusted(it.name) }, onBack = onBack) {
        RText(
            "A request as Autopilot reads it: the facts first, then what the AI wrote. Edit anything; nothing is kept.",
            RType.sans(14f, lineHeight = 19f),
            c.secondary,
            Modifier.padding(start = 20.dp, end = 20.dp, top = 8.dp),
        )
        if (!modelReady) {
            Banner(
                "Download the model in Autopilot first; until then Autopilot cannot judge.",
                Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp),
                BannerKind.Warning,
                tag = "tryNoModel",
            )
        }

        SectionLabel("Examples")
        Row(
            Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            AutopilotText.examples.forEach { e ->
                SelectChip(e.title, example == e.title, Modifier.testTag("example:${e.title}")) {
                    example = e.title
                    text = e.situation
                    viewModel.clearEvaluation()
                }
            }
        }

        if (ui.profiles.size > 1) {
            SectionLabel("Profile")
            Row(
                Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ui.profiles.forEach { p ->
                    SelectChip("${AutopilotText.profileIcon(p)}  ${untrusted(p.name)}", p.id == profile?.id, Modifier.testTag("tryProfile:${p.id}")) {
                        profileId = p.id
                        viewModel.clearEvaluation()
                    }
                }
            }
        }

        SectionLabel("The request")
        Box(
            Modifier
                .padding(horizontal = 16.dp)
                .fillMaxWidth()
                .clip(RoundedCornerShape(18.dp))
                .background(c.elevated)
                .padding(14.dp),
        ) {
            BasicTextField(
                text,
                {
                    text = it.take(4_000)
                    example = ""
                },
                Modifier.fillMaxWidth().defaultMinSize(minHeight = 180.dp).testTag("situation"),
                textStyle = RType.mono(13f).copy(color = c.text, lineHeight = androidx.compose.ui.unit.TextUnit(19f, androidx.compose.ui.unit.TextUnitType.Sp)),
                cursorBrush = SolidColor(c.accent),
            )
        }
        Column(Modifier.padding(horizontal = 16.dp, vertical = 14.dp)) {
            CapsuleButton(
                "Ask Autopilot",
                Modifier.fillMaxWidth().testTag("evaluate"),
                style = ButtonStyle.Accent,
                busy = ui.evaluating,
                enabled = text.isNotBlank(),
                glyph = Glyph.Sparkle,
            ) { viewModel.evaluate(profile?.id, text) }
            ui.error?.let { Banner(it, Modifier.padding(top = 12.dp), BannerKind.Error, tag = "tryError") }
        }

        AnimatedVisibility(
            ui.evaluation != null,
            enter = fadeIn() + slideInVertically(spring(dampingRatio = 0.85f, stiffness = 400f)) { it / 6 },
        ) {
            ui.evaluation?.let { VerdictCard(it) }
        }
        Spacer(Modifier.height(40.dp))
    }
}

/** The answer: a big verdict, the probabilities as bars, the reason, the neighbours, and what held it back. */
@Composable
private fun VerdictCard(s: SuggestionView) {
    val c = LocalColors.current
    val tint = if (s.judged) verdictTint(s.verdict, c) else c.tertiary
    Column(
        Modifier
            .padding(horizontal = 16.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(22.dp))
            .background(c.elevated)
            .padding(18.dp)
            .testTag("verdict"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            ProgressRing(if (s.verdict == Verdict.DENY) s.pDeny else s.pApprove, tint, size = 64.dp, stroke = 6.dp) {
                GlyphIcon(verdictGlyph(s.verdict), tint, size = 24.dp, weight = 2.3f)
            }
            Spacer(Modifier.width(16.dp))
            Column(Modifier.weight(1f)) {
                Caption("Autopilot would")
                RText(
                    if (s.judged) AutopilotText.verdictWord(s.verdict) else "Leave it to you",
                    RType.sans(26f, FontWeight.SemiBold),
                    tint,
                    Modifier.padding(top = 2.dp).testTag("verdictWord"),
                    maxLines = 1,
                )
                if (s.judged) RText("${AutopilotText.percent(s.confidence)} confident", RType.sans(13.5f), c.secondary, maxLines = 1)
            }
        }
        if (s.judged) {
            Column(Modifier.padding(top = 18.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                ProbabilityRow("Approve", s.pApprove, c.success)
                ProbabilityRow("Deny", s.pDeny, c.danger)
                ProbabilityRow("Confidence", s.confidence, c.accent)
            }
        }
        if (s.reason.isNotBlank()) {
            RText(untrusted(s.reason), RType.sans(14.5f, lineHeight = 20f), c.text, Modifier.padding(top = 16.dp).testTag("verdictReason"))
        }
        if (s.neighbours.isNotEmpty()) {
            Caption("Like these decisions of yours", Modifier.padding(top = 16.dp, bottom = 2.dp))
            s.neighbours.forEach { NeighbourRow(it) }
        }
        AutopilotText.suggestionNotes(s).forEach { note ->
            Row(Modifier.padding(top = 12.dp), verticalAlignment = Alignment.Top) {
                GlyphIcon(Glyph.Info, c.secondary, size = 16.dp)
                Spacer(Modifier.width(8.dp))
                RText(note, RType.sans(13.5f, lineHeight = 18f), c.secondary)
            }
        }
    }
}
