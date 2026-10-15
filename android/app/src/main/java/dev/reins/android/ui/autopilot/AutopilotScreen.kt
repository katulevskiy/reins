package dev.reins.android.ui.autopilot

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.AllInclusive
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.Memory
import androidx.compose.material.icons.rounded.Science
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.DownloadJob
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.Glyph
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.SwitchRow
import dev.reins.android.design.Tag
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowMillis
import dev.reins.android.design.rememberNowState
import dev.reins.android.feedback.DialogFeedback
import dev.reins.android.platform.Authenticator
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.BottomSheet
import dev.reins.android.ui.settings.IconPlate
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.android.ui.settings.SettingsPage
import dev.reins.android.ui.settings.SettingsRow
import dev.reins.android.ui.settings.settingsCard
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ModelState
import dev.reins.core.ModelStatus
import dev.reins.core.ProfileView
import kotlin.math.roundToInt

/**
 * The Autopilot tab: the mode (Manual or Auto, then Bypass and Lock down), this week in numbers, the model that runs
 * on this phone, and the profiles that learn from the user's answers.
 */
@Composable
fun AutopilotScreen(
    viewModel: AutopilotViewModel,
    /** Null at the root of the Autopilot tab. */
    onBack: (() -> Unit)?,
    onProfile: (String) -> Unit,
    onTryIt: () -> Unit,
    state: AppState? = null,
    /** A sheet is up: the tab bar steps aside so the sheet's buttons are not under it. */
    onCovered: (Boolean) -> Unit = {},
    /** The phone owner's check, before a bypass starts. */
    authenticator: Authenticator,
) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.showingAutopilot() }
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    val connections = state?.connections?.collectAsStateWithLifecycle()?.value.orEmpty()
    var bypassOpen by remember { mutableStateOf(false) }
    var overnightOpen by remember { mutableStateOf(false) }
    var modelOpen by remember { mutableStateOf(false) }
    var deleteAsk by remember { mutableStateOf(false) }
    var newProfile by remember { mutableStateOf(false) }
    var modeHelp by rememberSaveable { mutableStateOf(false) }
    val model = ui.model ?: settings?.model
    val modelReady = model?.state == ModelState.INSTALLED
    val covered = bypassOpen || overnightOpen || modelOpen
    LaunchedEffect(covered) { onCovered(covered) }
    DisposableEffect(Unit) { onDispose { onCovered(false) } }

    Box(Modifier.fillMaxSize()) {
        SettingsPage(title = "Autopilot", onBack = onBack) {
            val s = settings
            if (s == null) {
                Spacer(Modifier.height(40.dp))
                return@SettingsPage
            }
            ModeHeader(modeHelp, onHelp = { modeHelp = it })
            ModeHero(
                s,
                modelReady,
                onPick = { mode -> if (mode != s.mode) viewModel.setMode(mode) },
                onBypass = { bypassOpen = true },
                onOvernight = { overnightOpen = true },
                onLockdown = { viewModel.setMode(AutopilotMode.LOCKDOWN) },
                onEndLockdown = { viewModel.setMode(s.baseMode.takeIf { it != AutopilotMode.LOCKDOWN } ?: AutopilotMode.MANUAL) },
                onStop = { kind -> if (kind == TimedKind.Bypass) viewModel.stopBypass() else viewModel.stopOvernight() },
            )
            Column(Modifier.padding(horizontal = 16.dp)) {
                ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "autopilotError") }
                ui.notice?.let { Banner(it, Modifier.padding(top = 10.dp), tag = "autopilotNotice") }
            }

            if (model != null && !modelReady) {
                GetScoreCard(model, ui.job, s.wifiOnly, onDownload = viewModel::download, onPause = viewModel::cancelDownload)
            } else {
                SectionHeader("This week")
                StatTiles(
                    listOf(
                        Stat("${ui.stats?.scored ?: "—"}", "requests scored", tag = "statScored"),
                        Stat(AutopilotText.share(ui.stats?.matched), "matched you", c.success, tag = "statMatched"),
                        Stat("${ui.stats?.kindsOnAuto ?: "—"}", "kinds on Auto", tag = "statKinds"),
                    ),
                )
            }

            SettingsGroup(gap = 12.dp) {
                SettingsRow(
                    "On-device model",
                    Modifier.testTag("openModel"),
                    icon = Icons.Rounded.Memory,
                    tint = c.read,
                    valueContent = { ModelValue(model, ui.job) },
                ) { modelOpen = true }
                SettingsRow(
                    "Try it",
                    Modifier.testTag("openTryIt"),
                    icon = Icons.Rounded.Science,
                    tint = c.send,
                    value = if (modelReady) null else "after download",
                    onClick = onTryIt,
                )
            }

            SectionHeader("Profiles", trailing = {
                Row(
                    Modifier.clip(CircleShape).pressable(shape = CircleShape) { newProfile = true }.testTag("newProfile").padding(horizontal = 8.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Rounded.Add, null, Modifier.size(16.dp), tint = c.accent)
                    Spacer(Modifier.width(3.dp))
                    RText("New", RType.sans(14f, FontWeight.Medium), c.accent, maxLines = 1)
                }
            })
            Column(Modifier.padding(horizontal = 16.dp).settingsCard(c)) {
                val labels = connections.map { it.id to it.label }
                ui.profiles.forEach { profile ->
                    ProfileRow(profile, AutopilotText.profileLine(profile, AutopilotText.profileAis(profile, s, labels))) { onProfile(profile.id) }
                }
            }
            Spacer(Modifier.height(if (onBack == null) 24.dp else 32.dp))
        }

        BypassSheets(bypassOpen, who = null, onStart = { minutes -> viewModel.startBypass(minutes, authenticator = authenticator) }, onClose = { bypassOpen = false })
        OvernightSheets(
            overnightOpen,
            who = null,
            risky = settings?.overnightWhenRisky ?: dev.reins.core.OvernightRisky.WAIT,
            ratesRisk = model?.risk == true,
            onRisky = viewModel::setOvernightWhenRisky,
            onStart = { minutes -> viewModel.startOvernight(minutes, authenticator = authenticator) },
            onClose = { overnightOpen = false },
        )
        BottomSheet(modelOpen, "On-device model", onClose = { modelOpen = false }, tag = "modelSheet") {
            ModelDetails(
                model,
                job = ui.job,
                wifiOnly = settings?.wifiOnly ?: true,
                onDownload = viewModel::download,
                onCancel = viewModel::cancelDownload,
                onDelete = { deleteAsk = true },
                onWifiOnly = viewModel::setWifiOnly,
            )
        }
    }

    if (deleteAsk) {
        ConfirmDialog(
            title = "Delete the model?",
            text = "Manual asks without hints and Auto pauses until you download it again.",
            confirmLabel = "Delete",
            onConfirm = {
                deleteAsk = false
                modelOpen = false
                viewModel.deleteModel()
            },
            onDismiss = { deleteAsk = false },
        )
    }
    if (newProfile) {
        ProfileDialog(
            title = "New profile",
            initialName = "",
            initialIcon = AutopilotText.icons[2],
            confirmLabel = "Create",
            onConfirm = { name, icon ->
                newProfile = false
                viewModel.createProfile(name, icon, onProfile)
            },
            onDismiss = { newProfile = false },
        )
    }
}

/** "Mode" with its (?) right beside the word, and what the modes do under them when it is open. */
@Composable
private fun ModeHeader(open: Boolean, onHelp: (Boolean) -> Unit) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().padding(start = 22.dp, end = 22.dp, bottom = 4.dp).testTag("modeHeader")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            RText("Mode", RType.sans(14f, FontWeight.Medium), c.accent, Modifier.semantics { heading() }, maxLines = 1)
            Spacer(Modifier.width(2.dp))
            dev.reins.android.ui.settings.HelpIcon(open, "Mode", onHelp)
        }
        dev.reins.android.design.HelpText(AutopilotText.MODE_HELP, open, Modifier.padding(bottom = 8.dp).testTag("modeHelp"))
    }
}

/** A section's name in the accent colour, with something at its end ("+ New"). */
@Composable
internal fun SectionHeader(text: String, modifier: Modifier = Modifier, trailing: (@Composable () -> Unit)? = null) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().padding(start = 22.dp, end = 14.dp, top = 22.dp, bottom = 8.dp).heightIn(min = 22.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(text, RType.sans(14f, FontWeight.Medium), c.accent, Modifier.weight(1f).semantics { heading() }, maxLines = 1)
        trailing?.invoke()
    }
}

/** A bypass or an Overnight in force: which, its end ([AutopilotText.UNTIL_OFF] without one) and its start. */
internal data class Running(val kind: TimedKind, val until: Long, val since: Long?)

/** The global bypass in force, else the global Overnight; null when neither is. */
internal fun running(s: AutopilotSettings): Running? = when {
    s.mode == AutopilotMode.BYPASS && s.bypassUntil != null -> Running(TimedKind.Bypass, s.bypassUntil!!, s.bypassSince)
    s.mode == AutopilotMode.OVERNIGHT && s.overnightUntil != null -> Running(TimedKind.Overnight, s.overnightUntil!!, s.overnightSince)
    else -> null
}

/**
 * The mode, large. Manual and Auto side by side (the one in force tinted), then Overnight, then Bypass and Lock down.
 * During a bypass it turns red, during an Overnight night indigo: its countdown (or, without an end, how long it has
 * run) and Stop now. During a lockdown it turns amber and Lock down reads End lockdown.
 */
@Composable
private fun ModeHero(
    s: AutopilotSettings,
    modelReady: Boolean,
    onPick: (AutopilotMode) -> Unit,
    onBypass: () -> Unit,
    onOvernight: () -> Unit,
    onLockdown: () -> Unit,
    onEndLockdown: () -> Unit,
    onStop: (TimedKind) -> Unit,
) {
    val c = LocalColors.current
    val run = running(s)
    val lockdown = s.mode == AutopilotMode.LOCKDOWN
    val tint = when {
        run != null -> run.kind.tint(c)
        lockdown -> c.warning
        else -> null
    }
    val shape = RoundedCornerShape(28.dp)
    Box(
        Modifier
            .padding(horizontal = 12.dp)
            .fillMaxWidth()
            .clip(shape)
            .background(if (c.dark) Color(0xFF121215) else c.elevated)
            .then(if (tint != null) Modifier.background(Brush.verticalGradient(listOf(tint.copy(alpha = 0.20f), tint.copy(alpha = 0.05f)))) else Modifier)
            .border(1.dp, tint?.copy(alpha = 0.32f) ?: if (c.dark) Color(0xFF1C1C20) else c.hairline, shape)
            .testTag("modeHero"),
    ) {
        AnimatedContent(
            run?.kind,
            transitionSpec = { (fadeIn(tween(220)) + scaleIn(spring(dampingRatio = 0.8f, stiffness = 400f), initialScale = 0.97f)) togetherWith fadeOut(tween(120)) },
            label = "hero",
        ) { kind ->
            Column(Modifier.fillMaxWidth().padding(18.dp)) {
                if (kind != null) {
                    val r = running(s)?.takeIf { it.kind == kind } ?: return@Column
                    val color = kind.tint(c)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        TimedClock(kind, r.until, r.since)
                        Spacer(Modifier.width(16.dp))
                        Column(Modifier.weight(1f)) {
                            RText(AutopilotText.name(kind.mode), RType.sans(22f, FontWeight.SemiBold), color, Modifier.testTag("heroMode"), maxLines = 1)
                            RunningLine(kind, r.until, r.since)
                        }
                    }
                    PillButton(
                        if (kind == TimedKind.Bypass) "Stop bypass" else "Stop ${AutopilotText.OVERNIGHT}",
                        Modifier.padding(top = 14.dp).fillMaxWidth().testTag(kind.action("stop")),
                        background = color,
                        foreground = onTimed(kind, c),
                        height = 46.dp,
                        textSize = 15f,
                        onClick = { onStop(kind) },
                    )
                } else {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.height(IntrinsicSize.Min)) {
                        listOf(AutopilotMode.MANUAL, AutopilotMode.AUTO).forEach { mode ->
                            val needs = !modelReady && AutopilotText.needsModel(mode)
                            ModeCard(
                                mode,
                                selected = s.mode == mode,
                                dimmed = needs,
                                line = if (needs) "Needs the model" else AutopilotText.heroLine(mode),
                                modifier = Modifier.weight(1f).fillMaxHeight(),
                            ) { onPick(mode) }
                        }
                    }
                    PillButton(
                        AutopilotText.OVERNIGHT,
                        Modifier.padding(top = 10.dp).fillMaxWidth().testTag("mode:OVERNIGHT"),
                        icon = OvernightIcon,
                        background = c.night.copy(alpha = 0.13f),
                        foreground = c.night,
                        onClick = onOvernight,
                    )
                    Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        PillButton(
                            "Bypass",
                            Modifier.weight(1f).testTag("mode:BYPASS"),
                            icon = Icons.Rounded.Bolt,
                            background = c.danger.copy(alpha = 0.12f),
                            foreground = c.danger,
                            onClick = onBypass,
                        )
                        if (lockdown) {
                            PillButton(
                                "End lockdown",
                                Modifier.weight(1f).testTag("endLockdown"),
                                icon = Icons.Rounded.LockOpen,
                                background = c.warning.copy(alpha = 0.16f),
                                foreground = c.warning,
                                onClick = onEndLockdown,
                            )
                        } else {
                            PillButton(
                                "Lock down",
                                Modifier.weight(1f).testTag("mode:LOCKDOWN"),
                                icon = Icons.Rounded.Lock,
                                background = if (c.dark) Color.White.copy(alpha = 0.06f) else c.controlFill,
                                foreground = c.text,
                                onClick = onLockdown,
                            )
                        }
                    }
                }
            }
        }
    }
}

/** Manual or Auto: its tile, name and what it does; the one in force has a tinted edge and wash. */
@Composable
private fun ModeCard(mode: AutopilotMode, selected: Boolean, dimmed: Boolean, line: String, modifier: Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val tint = modeTint(mode, c)
    val shape = RoundedCornerShape(20.dp)
    val edge by animateColorAsState(if (selected) tint else if (c.dark) Color(0xFF26262B) else c.hairline, label = "modeEdge")
    val wash by animateColorAsState(if (selected) tint.copy(alpha = if (c.dark) 0.09f else 0.07f) else if (c.dark) Color(0xFF0D0D0F) else c.background, label = "modeWash")
    Column(
        modifier
            .graphicsLayer { alpha = if (dimmed) 0.5f else 1f }
            .clip(shape)
            .background(wash)
            .border(1.5.dp, edge, shape)
            .pressable(shape = shape, onClick = onClick)
            .semantics(mergeDescendants = true) {
                this.selected = selected
                role = Role.RadioButton
            }
            .testTag("mode:${mode.name}")
            .padding(14.dp),
    ) {
        Box(Modifier.size(40.dp).clip(RoundedCornerShape(13.dp)).background(tint.copy(alpha = 0.17f)), contentAlignment = Alignment.Center) {
            Icon(modeIcon(mode), null, Modifier.size(22.dp), tint = tint)
        }
        RText(AutopilotText.name(mode), RType.sans(17f, FontWeight.SemiBold), c.text, Modifier.padding(top = 10.dp), maxLines = 1)
        RText(line, RType.sans(12.5f, lineHeight = 17f), c.tertiary, Modifier.padding(top = 3.dp), maxLines = 3)
    }
}

/** What reads on a timed mode's solid colour: near black on the light dark-mode tints, white on the others. */
internal fun onTimed(kind: TimedKind, c: dev.reins.android.design.RColors): Color = when {
    !c.dark -> Color.White
    kind == TimedKind.Bypass -> Color(0xFF2A0606)
    else -> Color(0xFF0E1230)
}

/**
 * A timed mode's ring in its colour: the share left with m:ss in it, or a full ring around an infinity sign when it
 * has no end. Tags "bypassClock" / "overnightClock" and "bypassUntilOff" / "overnightUntilOff".
 */
@Composable
fun TimedClock(kind: TimedKind, until: Long, since: Long?, size: androidx.compose.ui.unit.Dp = 76.dp) {
    val c = LocalColors.current
    val color = kind.tint(c)
    if (AutopilotText.isUntilOff(until)) {
        Box(
            Modifier
                .size(size)
                .clip(CircleShape)
                .border(5.dp, color, CircleShape)
                .semantics { contentDescription = "${AutopilotText.name(kind.mode)} until you stop it" }
                .testTag(kind.tag("UntilOff")),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Rounded.AllInclusive, null, Modifier.size(size * 0.4f), tint = color) }
        return
    }
    val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
    ProgressRing(AutopilotText.bypassFraction(until, since, now), color, size = size, stroke = 5.dp) {
        RText(AutopilotText.clock(until, now), RType.mono(if (until - now < 3_600) 16f else 13f, FontWeight.SemiBold), color, Modifier.testTag(kind.tag("Clock")), maxLines = 1)
    }
}

/** "Approving without asking", or "On until you stop it · 2 days so far" (its own clock, so only it recomposes). */
@Composable
private fun RunningLine(kind: TimedKind, until: Long, since: Long?) {
    val c = LocalColors.current
    val now = rememberNowMillis(60_000) / 1000
    RText(AutopilotText.runningLine(kind.mode, until, since, now), RType.sans(13.5f), c.secondary, Modifier.padding(top = 3.dp).testTag(kind.tag("Line")), maxLines = 2)
}

/** One number of a strip, with what it counts. */
internal data class Stat(val value: String, val label: String, val color: Color? = null, val suffix: String? = null, val tag: String? = null)

/** Three numbers side by side. */
@Composable
internal fun StatTiles(stats: List<Stat>, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().padding(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        stats.forEach { stat ->
            Column(
                Modifier
                    .weight(1f)
                    .clip(RoundedCornerShape(20.dp))
                    .background(if (c.dark) Color(0xFF121215) else c.elevated)
                    .border(1.dp, if (c.dark) Color(0xFF1C1C20) else c.hairline, RoundedCornerShape(20.dp))
                    .semantics(mergeDescendants = true) {}
                    .then(if (stat.tag != null) Modifier.testTag(stat.tag) else Modifier)
                    .padding(12.dp),
            ) {
                Row(verticalAlignment = Alignment.Bottom) {
                    RText(stat.value, RType.sans(22f, FontWeight.SemiBold), stat.color ?: c.text, maxLines = 1)
                    stat.suffix?.let { RText(it, RType.sans(15f), c.tertiary, Modifier.padding(start = 3.dp, bottom = 2.dp), maxLines = 1) }
                }
                RText(stat.label, RType.sans(11.5f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 1)
            }
        }
    }
}

/** The model row's value: Ready, how far the download is, or Download. */
@Composable
private fun ModelValue(model: ModelStatus?, job: DownloadJob) {
    val c = LocalColors.current
    when {
        model == null -> Unit
        model.state == ModelState.INSTALLED -> Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("modelInstalled")) {
            Icon(Icons.Rounded.CheckCircle, null, Modifier.size(16.dp), tint = c.success)
            Spacer(Modifier.width(5.dp))
            RText("Ready", RType.sans(15f, FontWeight.Medium), c.success, maxLines = 1)
        }
        model.state == ModelState.DOWNLOADING || job == DownloadJob.Running ->
            RText(AutopilotText.fraction(model)?.let(AutopilotText::percent) ?: "…", RType.mono(14f), c.secondary, maxLines = 1)
        else -> RText("Download", RType.sans(15f, FontWeight.Medium), c.accent, maxLines = 1)
    }
}

/** Without the model: what it is for, its size, and getting it (with the progress while it comes). */
@Composable
private fun GetScoreCard(model: ModelStatus, job: DownloadJob, wifiOnly: Boolean, onDownload: () -> Unit, onPause: () -> Unit) {
    val c = LocalColors.current
    val downloading = model.state == ModelState.DOWNLOADING || job == DownloadJob.Running
    val waiting = job == DownloadJob.Waiting && !downloading
    val shape = RoundedCornerShape(24.dp)
    Column(
        Modifier
            .padding(start = 12.dp, end = 12.dp, top = 12.dp)
            .fillMaxWidth()
            .clip(shape)
            .background(Brush.verticalGradient(listOf(c.search.copy(alpha = 0.14f), c.search.copy(alpha = 0.03f))))
            .border(1.dp, c.search.copy(alpha = 0.26f), shape)
            .padding(16.dp)
            .testTag("getScore"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconPlate(Icons.Rounded.Memory, c.search)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText("Get Autopilot's score", RType.sans(16f, FontWeight.SemiBold), c.text, maxLines = 1)
                RText("${AutopilotText.modelSize(model)} · runs on this phone", RType.sans(13f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 1)
            }
        }
        if (downloading || waiting) {
            val fraction = AutopilotText.fraction(model)
            if (fraction != null) MeterBar(fraction, c.search, Modifier.padding(top = 12.dp).testTag("modelProgress")) else IndeterminateBar(Modifier.padding(top = 12.dp).testTag("modelProgress"))
            Row(Modifier.padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                RText(
                    if (waiting) AutopilotText.modelState(model, true, wifiOnly) else AutopilotText.progressText(model),
                    RType.sans(12.5f),
                    c.secondary,
                    Modifier.weight(1f).testTag("modelState"),
                    maxLines = 1,
                )
                if (waiting) {
                    RText("Pause", RType.sans(13f, FontWeight.Medium), c.accent, Modifier.clip(CircleShape).pressable(shape = CircleShape, onClick = onPause).testTag("pauseDownload").padding(horizontal = 6.dp, vertical = 2.dp), maxLines = 1)
                }
            }
        } else {
            if (model.state == ModelState.FAILED) {
                RText(AutopilotText.modelError(model.error), RType.sans(13f), c.danger, Modifier.padding(top = 10.dp).testTag("getModelError"))
            }
            PillButton(
                if (model.state == ModelState.FAILED) "Try again" else "Download",
                Modifier.padding(top = 12.dp).fillMaxWidth().testTag("getModel"),
                icon = Icons.Rounded.Download,
                background = c.search,
                foreground = if (c.dark) Color(0xFF04172F) else Color.White,
                onClick = onDownload,
            )
        }
    }
}

@Composable
private fun ProfileRow(profile: ProfileView, line: String, onClick: () -> Unit) {
    val c = LocalColors.current
    SettingsRow(
        untrusted(profile.name),
        Modifier.testTag("profile:${profile.id}"),
        subtitle = line,
        subtitleTag = "profileLine:${profile.id}",
        leading = { ProfileTile(profile, 40.dp) },
        onClick = onClick,
    )
}

/** A profile's emoji on a rounded tile. */
@Composable
fun ProfileTile(profile: ProfileView, size: androidx.compose.ui.unit.Dp) {
    val c = LocalColors.current
    Box(Modifier.size(size).clip(RoundedCornerShape(size * 0.3f)).background(if (c.dark) Color(0xFF232327) else c.controlFill), contentAlignment = Alignment.Center) {
        RText(AutopilotText.profileIcon(profile), RType.sans(size.value * 0.45f), c.text, maxLines = 1)
    }
}

/** The on-device model: what it is, whether it is here, getting it and removing it (set-up's card). */
@Composable
internal fun ModelCard(
    model: ModelStatus?,
    job: DownloadJob,
    wifiOnly: Boolean,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
    onDelete: () -> Unit,
    onWifiOnly: (Boolean) -> Unit,
) {
    Group(
        header = "On-device model",
        footer = "Runs on this phone, nothing leaves it. Without it, every request is left to you, without hints.",
    ) {
        ModelDetails(model, job, wifiOnly, onDownload, onCancel, onDelete, onWifiOnly)
    }
}

/** The model's details: version and size, its state, getting or deleting it, and Wi-Fi only. */
@Composable
internal fun ModelDetails(
    model: ModelStatus?,
    job: DownloadJob,
    wifiOnly: Boolean,
    onDownload: () -> Unit,
    onCancel: () -> Unit,
    onDelete: () -> Unit,
    onWifiOnly: (Boolean) -> Unit,
) {
    val c = LocalColors.current
    if (model == null) return
    val downloading = model.state == ModelState.DOWNLOADING || job == DownloadJob.Running
    val waiting = job == DownloadJob.Waiting && model.state != ModelState.DOWNLOADING
    Column(Modifier.padding(16.dp).testTag("modelCard")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconTile(Glyph.Chip, c.accent, size = 46.dp, filled = model.state == ModelState.INSTALLED)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                RText(model.label, RType.sans(16.5f, FontWeight.SemiBold), c.text, maxLines = 2)
                RText("Version ${model.version} · ${AutopilotText.modelSize(model)}", RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 2)
            }
            Spacer(Modifier.width(8.dp))
            when {
                downloading -> Unit
                waiting -> Tag("Waiting", tint = c.warning)
                model.state == ModelState.INSTALLED -> Tag("Installed", tint = c.success)
                model.state == ModelState.FAILED -> Tag("Failed", tint = c.danger)
                else -> Unit
            }
        }
        RText(
            AutopilotText.modelState(model, waiting, wifiOnly),
            RType.sans(13.5f, FontWeight.Medium),
            if (model.state == ModelState.FAILED) c.danger else c.secondary,
            Modifier.padding(top = 12.dp),
        )
        if (downloading) {
            val fraction = AutopilotText.fraction(model)
            if (fraction != null) {
                MeterBar(fraction, c.accent, Modifier.padding(top = 10.dp), height = 8.dp)
            } else {
                IndeterminateBar(Modifier.padding(top = 10.dp))
            }
        }
        if (model.state == ModelState.FAILED) {
            Banner(AutopilotText.modelError(model.error), Modifier.padding(top = 12.dp), BannerKind.Error)
        }
        Row(Modifier.padding(top = 14.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            when {
                downloading -> Unit
                waiting -> CapsuleButton("Cancel", Modifier.testTag("cancelDownload"), style = ButtonStyle.Secondary, compact = true, onClick = onCancel)
                model.state == ModelState.INSTALLED -> CapsuleButton(
                    "Delete model",
                    Modifier.testTag("deleteModel"),
                    style = ButtonStyle.Destructive,
                    compact = true,
                    glyph = Glyph.Trash,
                    onClick = onDelete,
                )
                else -> CapsuleButton(
                    if (model.state == ModelState.FAILED) "Try again" else "Download",
                    Modifier.fillMaxWidth().testTag("downloadModel"),
                    style = ButtonStyle.Accent,
                    glyph = if (model.state == ModelState.FAILED) Glyph.Refresh else Glyph.Download,
                    onClick = onDownload,
                )
            }
        }
    }
    Hairline(inset = 51.dp)
    SwitchRow(
        "Download on Wi-Fi only",
        wifiOnly,
        onWifiOnly,
        glyph = Glyph.Wifi,
        tag = "wifiOnly",
    )
}

/** A bar for a download whose size is not known: a short band sweeping across. Still when clocks are frozen. */
@Composable
private fun IndeterminateBar(modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val live = dev.reins.android.design.LocalLiveTimers.current
    // Read only where the band is placed: the sweep moves it every frame without recomposing or measuring anything.
    val phase = if (live) {
        androidx.compose.animation.core.rememberInfiniteTransition(label = "sweep").animateFloat(
            0f,
            1f,
            androidx.compose.animation.core.infiniteRepeatable(tween(1300)),
            label = "sweepPhase",
        )
    } else {
        null
    }
    androidx.compose.foundation.layout.BoxWithConstraints(
        modifier.fillMaxWidth().height(8.dp).clip(CircleShape).background(c.controlFill.copy(alpha = if (c.dark) 0.12f else 0.09f)),
    ) {
        val band = maxWidth * 0.35f
        Box(
            Modifier
                .offset { IntOffset((((maxWidth + band) * (phase?.value ?: 0.35f) - band).toPx()).roundToInt(), 0) }
                .width(band)
                .height(8.dp)
                .clip(CircleShape)
                .background(c.accent),
        )
    }
}

/** A profile's emoji on a soft disc. */
@Composable
fun ProfileAvatar(profile: ProfileView, size: androidx.compose.ui.unit.Dp) {
    val c = LocalColors.current
    Box(Modifier.size(size).clip(CircleShape).background(c.accentSoft), contentAlignment = Alignment.Center) {
        RText(AutopilotText.profileIcon(profile), RType.sans(size.value * 0.48f), c.text, maxLines = 1)
    }
}

/** Naming a profile and picking its icon (new and rename). */
@Composable
fun ProfileDialog(
    title: String,
    initialName: String,
    initialIcon: String?,
    confirmLabel: String,
    onConfirm: (String, String?) -> Unit,
    onDismiss: () -> Unit,
) {
    val c = LocalColors.current
    var name by remember { mutableStateOf(initialName) }
    var icon by remember { mutableStateOf(initialIcon) }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DialogFeedback()
        Column(
            Modifier.padding(horizontal = 24.dp).fillMaxWidth().glass(c, RoundedCornerShape(26.dp), 16.dp).padding(22.dp).testTag("profileDialog"),
        ) {
            RText(title, RType.sans(20f, FontWeight.SemiBold), c.text)
            RTextField(name, { name = it.take(40) }, "Name, e.g. Side project", Modifier.padding(top = 14.dp), tag = "profileName")
            RText("Icon", RType.sans(13f, FontWeight.Medium), c.tertiary, Modifier.padding(top = 16.dp, bottom = 8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                AutopilotText.icons.forEach { e ->
                    val on = e == icon
                    Box(
                        Modifier
                            .size(34.dp)
                            .clip(CircleShape)
                            .background(if (on) c.accentSoft else Color.Transparent)
                            .border(if (on) 1.5.dp else 0.dp, if (on) c.accent else Color.Transparent, CircleShape)
                            .pressable(shape = CircleShape) { icon = e }
                            .testTag("icon:$e"),
                        contentAlignment = Alignment.Center,
                    ) { RText(e, RType.sans(17f), c.text) }
                }
            }
            Spacer(Modifier.height(22.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                CapsuleButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Secondary, onClick = onDismiss)
                CapsuleButton(confirmLabel, Modifier.weight(1f).testTag("saveProfile"), enabled = name.isNotBlank()) { onConfirm(name, icon) }
            }
        }
    }
}
