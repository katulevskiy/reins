package dev.reins.android.ui.autopilot

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.spring
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import dev.reins.android.design.Choice
import dev.reins.android.design.Chooser
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.HairlineRow
import dev.reins.android.ui.main.HeaderInsets
import dev.reins.android.design.LabelText
import dev.reins.android.design.Pill
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalChip
import dev.reins.android.design.hairlineBelow
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Science
import androidx.compose.material.icons.rounded.Star
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.HelpText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SelectChip
import dev.reins.android.design.Tag
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.ConnectedButtons
import dev.reins.android.ui.settings.HelpIcon
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.android.ui.settings.SettingsPage
import dev.reins.android.ui.settings.SettingsRow
import dev.reins.android.ui.settings.settingsCard
import dev.reins.core.ClassView
import dev.reins.core.Preset
import dev.reins.core.ProfileView
import dev.reins.core.ThresholdReplay

/**
 * One profile: who uses it and how it is doing, how sure it must be (a preset, or thresholds of the user's own with
 * what the last 30 days would have been at them), each kind of request on its way to running by itself, and the
 * profile's own settings.
 */
@Composable
fun ProfileScreen(
    profileId: String,
    viewModel: AutopilotViewModel,
    state: AppState,
    onBack: () -> Unit,
    onTryIt: (String) -> Unit,
) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.showingProfiles() }
    val connections by state.connections.collectAsStateWithLifecycle()
    val settings by state.autopilot.collectAsStateWithLifecycle()
    val profile = ui.profiles.firstOrNull { it.id == profileId }
    var rename by remember { mutableStateOf(false) }
    var reset by remember { mutableStateOf(false) }
    var delete by remember { mutableStateOf(false) }
    var help by rememberSaveable { mutableStateOf(false) }
    var unlocking by remember { mutableStateOf<ClassView?>(null) }

    Column(Modifier.fillMaxSize().background(c.paper).verticalScroll(rememberScrollState()).windowInsetsPadding(HeaderInsets).padding(horizontal = 22.dp)) {
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            RoundIconButton(Glyph.ChevronLeft, "Back", Modifier.testTag("back"), onClick = onBack)
            Spacer(Modifier.weight(1f))
            if (profile != null) RoundIconButton(Glyph.Pencil, "Rename", Modifier.testTag("renameProfile")) { rename = true }
        }
        if (profile == null) {
            if (ui.profilesLoaded) EmptyState(Glyph.People, "Not found", "This profile no longer exists.", tag = "profileGone")
            return@Column
        }
        val ais = AutopilotText.profileAis(profile, settings, connections.map { it.id to it.label })
        ProfileHeader(profile, ais)
        ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "profileError") }
        ui.notice?.let { Banner(it, Modifier.padding(top = 10.dp), tag = "profileNotice") }
        StatTiles(
            listOf(
                Stat("${profile.memoryCount}", "decisions", tag = "statMemory"),
                Stat(AutopilotText.share(profile.agreement), "matched you", c.success, tag = "statMatched"),
                Stat("${profile.classes.count { it.autoApprove }}", "kinds on Auto", suffix = "/${profile.classes.size}", tag = "statKinds"),
            ),
            Modifier.padding(top = 18.dp),
        )

        Row(Modifier.padding(top = 26.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            LabelText("How sure before it acts", c.inkMeta, Modifier.weight(1f))
            HelpIcon(help, "How sure") { help = it }
        }
        HelpText(AutopilotText.HOW_SURE_HELP, help, Modifier.padding(bottom = 8.dp))
        Chooser(
            Preset.entries.map { Choice(it, AutopilotText.presetName(it), "preset:${it.name}") },
            profile.preset,
            { viewModel.setPreset(profile.id, it) },
        )
        AnimatedVisibility(
            profile.preset == Preset.CUSTOM,
            enter = expandVertically(spring(dampingRatio = 0.9f, stiffness = 500f)) + fadeIn(),
            exit = shrinkVertically(spring(dampingRatio = 0.9f, stiffness = 500f)) + fadeOut(),
        ) {
            CustomThresholds(profile, ui.replay?.takeIf { it.first == profile.id }?.second, viewModel)
        }
        if (profile.preset != Preset.CUSTOM) {
            RText(AutopilotText.presetLine(profile), RType.meta, c.inkMeta, Modifier.padding(top = 10.dp).testTag("presetLine"))
        }

        LabelText("Kinds of request", c.inkMeta, Modifier.padding(top = 28.dp, bottom = 4.dp))
        if (profile.classes.isEmpty()) {
            EmptyState(Glyph.Sparkle, "Nothing learned yet", "Your answers teach it.", tag = "noClasses")
        }
        profile.classes.forEach { cls ->
            KindRow(cls) { locked ->
                if (locked == false) unlocking = cls else viewModel.setClassLock(profile.id, cls.classKey, locked)
            }
        }

        Column(Modifier.padding(top = 22.dp)) {
            HairlineRow("Try this profile", Modifier.testTag("tryProfile"), leading = { PlainTile(Glyph.Flask, 34.dp) }, trailing = { GlyphIcon(Glyph.ChevronRight, c.inkMeta, size = 16.dp) }) { onTryIt(profile.id) }
            if (!profile.isDefault) {
                HairlineRow("Use for every AI", Modifier.testTag("makeDefault"), leading = { PlainTile(Glyph.Star, 34.dp) }) { viewModel.makeDefault(profile.id) }
            }
            HairlineRow(
                "Forget what it learned",
                Modifier.testTag("resetProfile"),
                subtitle = "${profile.memoryCount} remembered",
                leading = { PlainTile(Glyph.Refresh, 34.dp) },
            ) { reset = true }
            if (ui.profiles.size > 1) {
                HairlineRow(
                    "Delete profile",
                    Modifier.testTag("deleteProfile"),
                    leading = { GlyphIcon(Glyph.Trash, c.danger, size = 20.dp, modifier = Modifier.padding(horizontal = 7.dp)) },
                    divider = false,
                ) { delete = true }
            }
        }
        Spacer(Modifier.height(40.dp))
    }

    if (profile != null) {
        if (rename) {
            ProfileDialog(
                title = "Rename profile",
                initialName = profile.name,
                initialIcon = profile.icon,
                confirmLabel = "Save",
                onConfirm = { name, icon ->
                    rename = false
                    viewModel.renameProfile(profile.id, name, icon)
                },
                onDismiss = { rename = false },
            )
        }
        if (reset) {
            ConfirmDialog(
                title = "Forget what ${untrusted(profile.name)} learned?",
                text = "Its ${profile.memoryCount} decisions go and it starts over.",
                confirmLabel = "Forget",
                onConfirm = {
                    reset = false
                    viewModel.resetProfile(profile.id)
                },
                onDismiss = { reset = false },
            )
        }
        if (delete) {
            ConfirmDialog(
                title = "Delete ${untrusted(profile.name)}?",
                text = "Its AIs move to the default profile.",
                confirmLabel = "Delete",
                onConfirm = {
                    delete = false
                    viewModel.deleteProfile(profile.id, onBack)
                },
                onDismiss = { delete = false },
            )
        }
        unlocking?.let { cls ->
            ConfirmDialog(
                title = "Let Auto approve ${cls.label}?",
                text = "Auto approves these when it is sure. The riskiest still ask.",
                confirmLabel = "Unlock",
                onConfirm = {
                    unlocking = null
                    viewModel.setClassLock(profile.id, cls.classKey, false)
                },
                onDismiss = { unlocking = null },
            )
        }
    }
}

/** The profile's glyph and name in the serif, Default when it is, and who uses it with how it is doing. */
@Composable
private fun ProfileHeader(profile: ProfileView, ais: List<String>) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().padding(top = 18.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            ProfileTile(profile, 40.dp)
            RText(untrusted(profile.name), RType.serif(38f, 42f), c.ink, Modifier.weight(1f, fill = false).semantics { heading() }.testTag("profileName"), maxLines = 1)
            if (profile.isDefault) Pill("Default", c.violet, Modifier.testTag("defaultTag"))
        }
        RText(
            if (ais.isEmpty()) "No AI yet" else ais.joinToString(", "),
            RType.meta,
            c.inkMeta,
            Modifier.padding(top = 8.dp).testTag("profileAis"),
            maxLines = 2,
        )
    }
}

/**
 * "Approve at" and "Deny at", each from 80% to 99%, and what the last 30 days would have been at them. Shown at once
 * while dragging (with the replay asked again), saved when the finger lifts.
 */
@Composable
private fun CustomThresholds(profile: ProfileView, replay: ThresholdReplay?, viewModel: AutopilotViewModel) {
    val c = LocalColors.current
    var approve by remember(profile.id, profile.approveAt) { mutableFloatStateOf(profile.approveAt) }
    var deny by remember(profile.id, profile.denyAt) { mutableFloatStateOf(profile.denyAt) }
    LaunchedEffect(profile.id) { viewModel.replay(profile.id, approve, deny) }
    Column(
        Modifier
            .padding(top = 14.dp)
            .testTag("thresholds"),
    ) {
        ThresholdSlider("Approve at", approve, c.success, "approveAt", onChange = {
            approve = it
            viewModel.replay(profile.id, approve, deny)
        }) { viewModel.setThresholds(profile.id, approve, deny) }
        Spacer(Modifier.height(16.dp))
        ThresholdSlider("Deny at", deny, c.danger, "denyAt", onChange = {
            deny = it
            viewModel.replay(profile.id, approve, deny)
        }) { viewModel.setThresholds(profile.id, approve, deny) }
        if (replay != null) {
            val text = buildAnnotatedString {
                AutopilotText.replayParts(replay).forEach { (part, strong) ->
                    if (strong) withStyle(SpanStyle(color = c.text)) { append(part) } else append(part)
                }
            }
            androidx.compose.material3.Text(
                text,
                Modifier.padding(top = 14.dp).testTag("replayLine"),
                style = RType.sans(12.5f, lineHeight = 18f).copy(color = c.tertiary),
            )
        }
    }
}

/**
 * A threshold: its name and value, and a track from 0 to 100% with a tick at the 80% floor, filled from there to the
 * thumb. Dragging or tapping moves it in whole percent within 80–99%; a screen reader adjusts it the same way.
 */
@Composable
private fun ThresholdSlider(label: String, value: Float, tint: Color, tag: String, onChange: (Float) -> Unit, onDone: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val min = AutopilotText.MIN_THRESHOLD_PERCENT / 100f
    val max = AutopilotText.MAX_THRESHOLD_PERCENT / 100f
    val change by rememberUpdatedState(onChange)
    val done by rememberUpdatedState(onDone)
    val current by rememberUpdatedState(value)
    fun snap(x: Float): Float = (Math.round(x.coerceIn(min, max) * 100) / 100f)
    fun move(x: Float) {
        val v = snap(x)
        if (v != current) {
            change(v)
            feedback.play(Event.Detent, Math.round(v * 100) - AutopilotText.MIN_THRESHOLD_PERCENT)
        }
    }
    Column {
        Row(verticalAlignment = Alignment.CenterVertically) {
            RText(label, RType.sans(14.5f, FontWeight.Medium), c.secondary, Modifier.weight(1f), maxLines = 1)
            RText(AutopilotText.percent(value), RType.mono(14.5f, FontWeight.SemiBold), tint, Modifier.testTag("$tag:value"), maxLines = 1)
        }
        BoxWithConstraints(
            Modifier
                .padding(top = 6.dp)
                .fillMaxWidth()
                .height(28.dp)
                .pointerInput(Unit) {
                    detectTapGestures { o ->
                        move(o.x / size.width)
                        done()
                    }
                }
                .pointerInput(Unit) {
                    detectHorizontalDragGestures(onDragEnd = { done() }, onDragCancel = { done() }) { change, _ ->
                        change.consume()
                        move(change.position.x / size.width)
                    }
                }
                .semantics {
                    contentDescription = label
                    stateDescription = AutopilotText.percent(value)
                    progressBarRangeInfo = ProgressBarRangeInfo(value, min..max, steps = AutopilotText.MAX_THRESHOLD_PERCENT - AutopilotText.MIN_THRESHOLD_PERCENT - 1)
                    setProgress { target ->
                        move(target)
                        done()
                        true
                    }
                }
                .testTag(tag),
            contentAlignment = Alignment.CenterStart,
        ) {
            val track = if (c.dark) Color(0xFF26262B) else c.hairline
            Canvas(Modifier.fillMaxWidth().height(6.dp)) {
                val r = CornerRadius(size.height / 2)
                drawRoundRect(track, cornerRadius = r)
                val from = size.width * min
                drawRoundRect(tint, Offset(from, 0f), Size(size.width * (value - min), size.height), r)
                drawRect(c.tertiary, Offset(from - 1.dp.toPx(), -5.dp.toPx()), Size(2.dp.toPx(), size.height + 10.dp.toPx()))
            }
            Box(
                Modifier
                    .offset(x = maxWidth * value - 11.dp)
                    .size(22.dp)
                    .shadow(4.dp, CircleShape)
                    .clip(CircleShape)
                    .background(if (c.dark) Color(0xFFF2F2F4) else Color.White)
                    .border(if (c.dark) 0.dp else 1.dp, c.hairline, CircleShape),
            )
        }
    }
}

/**
 * One kind of request: a ring (full and green on Auto, filling in blue while it learns, a lock when it always asks),
 * where it stands, and, opened, the lock.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun KindRow(cls: ClassView, onLock: (Boolean?) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var open by remember { mutableStateOf(false) }
    val locked = cls.manual == false
    Column(Modifier.fillMaxWidth().testTag("class:${cls.classKey}")) {
        Row(
            Modifier
                .fillMaxWidth()
                .hairlineBelow(c.rule)
                .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp)) {
                    open = !open
                    feedback.play(Event.expand(open))
                }
                .padding(vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            when {
                locked -> Box(Modifier.size(34.dp).clip(CircleShape).background(c.paperSunken), contentAlignment = Alignment.Center) {
                    GlyphIcon(Glyph.Lock, c.ink2, size = 15.dp)
                }
                cls.autoApprove -> ProgressRing(1f, c.success, size = 34.dp, stroke = 3.5.dp)
                else -> ProgressRing(AutopilotText.unlockProgress(cls), c.search, size = 34.dp, stroke = 3.5.dp)
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText(cls.label, RType.row, c.ink, maxLines = 1)
                RText(AutopilotText.kindLine(cls), RType.meta, c.inkMeta, Modifier.padding(top = 2.dp).testTag("classStatus:${cls.classKey}"), maxLines = 1)
            }
            Spacer(Modifier.width(8.dp))
            if (cls.autoApprove) {
                Pill("Auto", c.violet)
            } else {
                GlyphIcon(if (open) Glyph.ChevronUp else Glyph.ChevronDown, c.inkMeta, size = 16.dp)
            }
        }
        AnimatedVisibility(
            open,
            enter = expandVertically(spring(dampingRatio = 0.9f, stiffness = 500f)) + fadeIn(),
            exit = shrinkVertically(spring(dampingRatio = 0.9f, stiffness = 500f)) + fadeOut(),
        ) {
            Column(Modifier.padding(start = 46.dp, top = 10.dp, bottom = 14.dp)) {
                RText(AutopilotText.classNumbers(cls), RType.meta, c.inkMeta, Modifier.padding(bottom = 8.dp), maxLines = 1)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    SignalChip("Learn by itself", cls.manual == null, Modifier.testTag("lock:auto:${cls.classKey}")) { onLock(null) }
                    SignalChip("Always ask", cls.manual == false, Modifier.testTag("lock:on:${cls.classKey}")) { onLock(true) }
                    SignalChip("Unlock now", cls.manual == true, Modifier.testTag("lock:off:${cls.classKey}")) { onLock(false) }
                }
            }
        }
    }
}
