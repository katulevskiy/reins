package dev.reins.android.ui.autopilot

import androidx.compose.animation.AnimatedContent
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
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
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
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.ListRow
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SectionLabel
import dev.reins.android.design.SelectChip
import dev.reins.android.design.SwitchRow
import dev.reins.android.design.Tag
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowMillis
import dev.reins.android.feedback.DialogFeedback
import dev.reins.android.ui.common.untrusted
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ModelState
import dev.reins.core.ModelStatus
import dev.reins.core.ProfileView
import kotlin.math.roundToInt

/**
 * Settings > Autopilot: the global mode, the model that runs on this phone, and the profiles that learn from the
 * user's answers.
 */
@Composable
fun AutopilotScreen(
    viewModel: AutopilotViewModel,
    onBack: () -> Unit,
    onProfile: (String) -> Unit,
    onTryIt: () -> Unit,
) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.showingProfiles() }
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    var bypassAsk by remember { mutableStateOf(false) }
    var lockdownAsk by remember { mutableStateOf(false) }
    var deleteAsk by remember { mutableStateOf(false) }
    var newProfile by remember { mutableStateOf(false) }
    val model = ui.model ?: settings?.model
    val modelReady = model?.state == ModelState.INSTALLED

    Screen(title = "Autopilot", onBack = onBack) {
        val s = settings
        if (s == null) {
            Spacer(Modifier.height(40.dp))
            return@Screen
        }
        ModeHero(s, modelReady, onStopBypass = { viewModel.stopBypass() }, onEndLockdown = { viewModel.setMode(s.baseMode.takeIf { it != AutopilotMode.LOCKDOWN } ?: AutopilotMode.MANUAL) })
        Column(Modifier.padding(horizontal = 16.dp)) {
            ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "autopilotError") }
            ui.notice?.let { Banner(it, Modifier.padding(top = 10.dp), tag = "autopilotNotice") }
        }

        SectionLabel("Mode")
        ModePicker(
            selected = s.mode,
            modelReady = modelReady,
            modifier = Modifier.padding(horizontal = 16.dp),
        ) { mode ->
            when {
                mode == AutopilotMode.BYPASS -> bypassAsk = true
                mode == AutopilotMode.LOCKDOWN && s.mode != AutopilotMode.LOCKDOWN -> lockdownAsk = true
                mode != s.mode -> viewModel.setMode(mode)
            }
        }
        RText(
            "For every AI, unless one has a mode of its own (Settings, then the AI). Riskier requests, such as passwords, deletions and new connections, always wait for you.",
            RType.sans(12.5f),
            c.tertiary,
            Modifier.padding(start = 32.dp, end = 32.dp, top = 8.dp),
        )

        ModelCard(
            model,
            job = ui.job,
            wifiOnly = s.wifiOnly,
            onDownload = viewModel::download,
            onCancel = viewModel::cancelDownload,
            onDelete = { deleteAsk = true },
            onWifiOnly = viewModel::setWifiOnly,
        )

        Group(
            header = "Profiles",
            footer = "Each AI's answers teach its profile. Every AI uses the default one unless you pick another on its page.",
        ) {
            ui.profiles.forEachIndexed { i, profile ->
                if (i > 0) Hairline(inset = 70.dp)
                ProfileRow(profile) { onProfile(profile.id) }
            }
            if (ui.profiles.isNotEmpty()) Hairline(inset = 70.dp)
            ListRow("New profile", glyph = Glyph.Plus, tint = c.accent, modifier = Modifier.testTag("newProfile")) { newProfile = true }
        }

        Group(header = "Try it", footer = "Type a request and see what Autopilot would do with it. Nothing is kept.") {
            ListRow(
                "See how Autopilot judges",
                subtitle = if (modelReady) "A push, an email, a command, an injection attempt" else "Download the model first",
                glyph = Glyph.Flask,
                tint = c.accent,
                chevron = true,
                modifier = Modifier.testTag("openTryIt"),
                onClick = onTryIt,
            )
        }
        Spacer(Modifier.height(32.dp))
    }

    if (bypassAsk) {
        BypassDialog(
            who = null,
            onConfirm = { minutes ->
                bypassAsk = false
                viewModel.setMode(AutopilotMode.BYPASS, minutes)
            },
            onDismiss = { bypassAsk = false },
        )
    }
    if (lockdownAsk) {
        ConfirmDialog(
            title = "Lock down?",
            text = "Every request is denied at once, the ones waiting now included. New connections still reach you. Switch back any time.",
            confirmLabel = "Lock down",
            onConfirm = {
                lockdownAsk = false
                viewModel.setMode(AutopilotMode.LOCKDOWN)
            },
            onDismiss = { lockdownAsk = false },
        )
    }
    if (deleteAsk) {
        ConfirmDialog(
            title = "Delete the model?",
            text = "Assisted and Auto stop until you download it again; requests wait for you. What your profiles learned stays.",
            confirmLabel = "Delete",
            onConfirm = {
                deleteAsk = false
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

/** The mode in force, large: its tile and colour, what it means right now, and the way out of Bypass or Lockdown. */
@Composable
private fun ModeHero(s: AutopilotSettings, modelReady: Boolean, onStopBypass: () -> Unit, onEndLockdown: () -> Unit) {
    val c = LocalColors.current
    val mode = s.mode
    val tint = modeTint(mode, c)
    Box(
        Modifier
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(26.dp))
            .background(c.elevated)
            .background(Brush.verticalGradient(listOf(tint.copy(alpha = if (c.dark) 0.20f else 0.12f), Color.Transparent)))
            .border(0.75.dp, tint.copy(alpha = 0.22f), RoundedCornerShape(26.dp))
            .testTag("modeHero"),
    ) {
        AnimatedContent(
            mode,
            transitionSpec = { (fadeIn(tween(220)) + scaleIn(spring(dampingRatio = 0.8f, stiffness = 400f), initialScale = 0.96f)) togetherWith fadeOut(tween(120)) },
            label = "hero",
        ) { shown ->
            Column(Modifier.fillMaxWidth().padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                if (shown == AutopilotMode.BYPASS && s.bypassUntil != null) {
                    BypassClock(s.bypassUntil!!)
                } else {
                    IconTile(modeGlyph(shown), modeTint(shown, c), size = 64.dp, filled = true)
                }
                RText(AutopilotText.name(shown), RType.sans(28f, FontWeight.SemiBold), c.text, Modifier.padding(top = 14.dp).testTag("heroMode"))
                RText(heroLine(shown, modelReady), RType.sans(14.5f, lineHeight = 20f), c.secondary, Modifier.padding(top = 4.dp), align = TextAlign.Center)
                when (shown) {
                    AutopilotMode.BYPASS -> CapsuleButton(
                        "Stop bypass",
                        Modifier.padding(top = 16.dp).testTag("stopBypass"),
                        style = ButtonStyle.Destructive,
                        glyph = Glyph.Stop,
                        onClick = onStopBypass,
                    )
                    AutopilotMode.LOCKDOWN -> CapsuleButton(
                        "End lockdown",
                        Modifier.padding(top = 16.dp).testTag("endLockdown"),
                        style = ButtonStyle.Secondary,
                        glyph = Glyph.Unlock,
                        onClick = onEndLockdown,
                    )
                    else -> Unit
                }
            }
        }
    }
}

private fun heroLine(mode: AutopilotMode, modelReady: Boolean): String = when (mode) {
    AutopilotMode.MANUAL -> "Every request waits for you. Autopilot stays out of the way."
    AutopilotMode.ASSISTED -> if (modelReady) "Requests wait for you, with what Autopilot would do. Every answer teaches it." else "Requests wait for you. Download the model to see Autopilot's suggestions."
    AutopilotMode.AUTO -> if (modelReady) "Autopilot answers what it is sure of, in the kinds of request it has learned. The rest waits for you." else "Download the model: until then every request waits for you."
    AutopilotMode.BYPASS -> "Everything is approved without asking, except the riskiest requests."
    AutopilotMode.LOCKDOWN -> "Every request is denied at once. New connections still reach you."
}

/** The bypass countdown: a ring that empties as the time runs out, and the minutes and seconds left. */
@Composable
fun BypassClock(until: Long, size: androidx.compose.ui.unit.Dp = 112.dp) {
    val c = LocalColors.current
    val now = rememberNowMillis(1_000) / 1000
    // The core keeps only the end; the ring measures against the length the bypass was most likely given.
    val total = remember(until) { AutopilotText.bypassLength(until - now) }
    val left = (until - now).coerceAtLeast(0)
    ProgressRing(left.toFloat() / total, c.danger, size = size, stroke = 7.dp) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            RText(AutopilotText.clock(until, now), RType.mono(24f, FontWeight.SemiBold), c.text, Modifier.testTag("bypassClock"))
            RText("left", RType.sans(12f), c.secondary)
        }
    }
}

/** The on-device model: what it is, whether it is here, getting it and removing it. */
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
    val c = LocalColors.current
    Group(
        header = "On-device model",
        footer = "Runs on this phone, nothing leaves it. Without it, Assisted and Auto simply leave every request to you.",
    ) {
        if (model == null) return@Group
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
                    model.state == ModelState.INSTALLED -> Tag("Installed", Modifier.testTag("modelInstalled"), tint = c.success)
                    model.state == ModelState.FAILED -> Tag("Failed", tint = c.danger)
                    else -> Unit
                }
            }
            RText(
                AutopilotText.modelState(model, waiting, wifiOnly),
                RType.sans(13.5f, FontWeight.Medium),
                if (model.state == ModelState.FAILED) c.danger else c.secondary,
                Modifier.padding(top = 12.dp).testTag("modelState"),
            )
            if (downloading) {
                val fraction = AutopilotText.fraction(model)
                if (fraction != null) {
                    MeterBar(fraction, c.accent, Modifier.padding(top = 10.dp).testTag("modelProgress"), height = 8.dp)
                } else {
                    IndeterminateBar(Modifier.padding(top = 10.dp).testTag("modelProgress"))
                }
            }
            if (model.state == ModelState.FAILED) {
                Banner(AutopilotText.modelError(model.error), Modifier.padding(top = 12.dp), BannerKind.Error, tag = "modelError")
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
            subtitle = "The model is a few hundred megabytes",
            glyph = Glyph.Wifi,
            tag = "wifiOnly",
        )
    }
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

@Composable
private fun ProfileRow(profile: ProfileView, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp)
            .testTag("profile:${profile.id}"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ProfileAvatar(profile, 40.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(untrusted(profile.name), RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1)
            RText(AutopilotText.profileSummary(profile), RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 1)
        }
        val auto = profile.classes.count { it.autoApprove }
        if (profile.classes.isNotEmpty()) {
            ProgressRing(auto.toFloat() / profile.classes.size, c.accent, size = 26.dp, stroke = 3.dp)
            Spacer(Modifier.width(10.dp))
        }
        GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp)
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

/**
 * Turning a bypass on: how long (15, 30 or 60 minutes) and what it means. [who] names the AI for a connection's own
 * bypass; null is every AI.
 */
@Composable
fun BypassDialog(who: String?, onConfirm: (UInt) -> Unit, onDismiss: () -> Unit) {
    val c = LocalColors.current
    var minutes by remember { mutableStateOf(AutopilotText.bypassMinutes.first()) }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DialogFeedback()
        Column(
            Modifier.padding(horizontal = 24.dp).fillMaxWidth().glass(c, RoundedCornerShape(26.dp), 16.dp).padding(22.dp).testTag("bypassDialog"),
        ) {
            IconTile(Glyph.Bolt, c.danger, size = 48.dp, filled = true)
            RText(if (who == null) "Bypass for every AI?" else "Bypass ${untrusted(who)}?", RType.sans(20f, FontWeight.SemiBold), c.text, Modifier.padding(top = 14.dp))
            RText(
                "Requests are approved without asking until the time runs out. Approved means done: an email sent cannot be unsent.",
                RType.sans(15f, lineHeight = 21f),
                c.secondary,
                Modifier.padding(top = 8.dp),
            )
            Row(
                Modifier.padding(top = 14.dp).fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.controlFill).padding(12.dp),
                verticalAlignment = Alignment.Top,
            ) {
                GlyphIcon(Glyph.Shield, c.secondary, size = 18.dp)
                Spacer(Modifier.width(10.dp))
                RText(
                    "Still asked every time: new connections, permissions, passwords and secrets, deletions and other one-off changes, SSH and flagged files.",
                    RType.sans(13.5f, lineHeight = 18f),
                    c.secondary,
                )
            }
            RText("For", RType.sans(13f, FontWeight.Medium), c.tertiary, Modifier.padding(top = 16.dp, bottom = 8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AutopilotText.bypassMinutes.forEach { m ->
                    SelectChip(if (m == 60u) "1 hour" else "$m min", minutes == m, Modifier.testTag("bypass:$m")) { minutes = m }
                }
            }
            Spacer(Modifier.height(22.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                CapsuleButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Secondary, onClick = onDismiss)
                CapsuleButton(
                    "Turn on",
                    Modifier.weight(1f).testTag("confirmBypass"),
                    style = ButtonStyle.Destructive,
                    glyph = Glyph.Bolt,
                ) { onConfirm(minutes) }
            }
        }
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
