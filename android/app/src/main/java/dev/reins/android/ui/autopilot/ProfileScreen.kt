package dev.reins.android.ui.autopilot

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.spring
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.CircleIconButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.ListRow
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SectionLabel
import dev.reins.android.design.SelectChip
import dev.reins.android.design.Tag
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ClassView
import dev.reins.core.Preset
import dev.reins.core.ProfileView

/**
 * One profile: how sure it must be, each kind of request on its way to running by itself (a ring fills toward
 * unlocking), and the profile's own settings.
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
    val connections by state.connections.collectAsStateWithLifecycle()
    val profile = ui.profiles.firstOrNull { it.id == profileId }
    var rename by remember { mutableStateOf(false) }
    var reset by remember { mutableStateOf(false) }
    var delete by remember { mutableStateOf(false) }
    var unlocking by remember { mutableStateOf<ClassView?>(null) }

    Screen(
        title = profile?.let { untrusted(it.name) } ?: "Profile",
        onBack = onBack,
        actions = {
            if (profile != null) CircleIconButton(Glyph.Pencil, Modifier.testTag("renameProfile"), label = "Rename") { rename = true }
        },
    ) {
        if (profile == null) {
            EmptyState(Glyph.People, "Not found", "This profile no longer exists.", tag = "profileGone")
            return@Screen
        }
        ProfileHeader(profile, connections.filter { it.id in profile.connections }.map { untrusted(it.label) })
        Column(Modifier.padding(horizontal = 16.dp)) {
            ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "profileError") }
            ui.notice?.let { Banner(it, Modifier.padding(top = 10.dp), tag = "profileNotice") }
        }

        SectionLabel("How sure it must be")
        Column(Modifier.padding(horizontal = 16.dp)) {
            Segmented(
                Preset.entries,
                profile.preset,
                AutopilotText::presetName,
                tag = { "preset:${it.name}" },
            ) { viewModel.setPreset(profile.id, it) }
            RText(AutopilotText.presetLine(profile.preset), RType.sans(13f), c.secondary, Modifier.padding(start = 16.dp, top = 8.dp).testTag("presetLine"))
        }

        Group(
            header = "Kinds of request",
            footer = "A kind runs by itself in Auto after 20 of your answers with 95% agreement, and never after Autopilot approved something you would have denied. Tap one to lock or unlock it yourself.",
        ) {
            if (profile.classes.isEmpty()) {
                EmptyState(
                    Glyph.Sparkle,
                    "Nothing learned yet",
                    "Answer requests in Assisted or Auto: every approve and deny teaches this profile.",
                    tag = "noClasses",
                )
            }
            profile.classes.forEachIndexed { i, cls ->
                if (i > 0) Hairline(inset = 84.dp)
                ClassRow(
                    cls,
                    onLock = { locked ->
                        if (locked == false) unlocking = cls else viewModel.setClassLock(profile.id, cls.classKey, locked)
                    },
                )
            }
        }

        Group(header = "Try it") {
            ListRow(
                "Try this profile",
                subtitle = "See what it would do with a request you type",
                glyph = Glyph.Flask,
                tint = c.accent,
                chevron = true,
                modifier = Modifier.testTag("tryProfile"),
            ) { onTryIt(profile.id) }
        }

        Group(header = "Profile") {
            if (!profile.isDefault) {
                ListRow(
                    "Use for every AI by default",
                    subtitle = "AIs without a profile of their own learn here",
                    glyph = Glyph.Star,
                    modifier = Modifier.testTag("makeDefault"),
                ) { viewModel.makeDefault(profile.id) }
                Hairline(inset = 51.dp)
            }
            ListRow(
                "Forget what it learned",
                subtitle = "${profile.memoryCount} remembered ${if (profile.memoryCount == 1u) "decision" else "decisions"}, and kinds you locked or unlocked",
                glyph = Glyph.Refresh,
                modifier = Modifier.testTag("resetProfile"),
            ) { reset = true }
            if (ui.profiles.size > 1) {
                Hairline(inset = 51.dp)
                ListRow("Delete profile", glyph = Glyph.Trash, destructive = true, modifier = Modifier.testTag("deleteProfile")) { delete = true }
            }
        }
        Spacer(Modifier.height(32.dp))
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
                text = "Its ${profile.memoryCount} remembered decisions go, every kind of request is locked again, and Autopilot starts learning from your next answer.",
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
                text = "What it learned is gone. AIs that used it move to the default profile.",
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
                text = "Autopilot will approve these on its own when it is sure enough, before it has learned enough to unlock them by itself. The riskiest requests still wait for you.",
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

@Composable
private fun ProfileHeader(profile: ProfileView, connectionLabels: List<String>) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().padding(top = 8.dp, bottom = 4.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        ProfileAvatar(profile, 80.dp)
        Row(Modifier.padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            RText(untrusted(profile.name), RType.sans(24f, FontWeight.SemiBold), c.text, maxLines = 1)
            if (profile.isDefault) {
                Spacer(Modifier.width(8.dp))
                Tag("Default", Modifier.testTag("defaultTag"), tint = c.accent)
            }
        }
        RText(
            when {
                connectionLabels.isNotEmpty() -> "Learns from " + connectionLabels.joinToString(", ")
                profile.isDefault -> "Learns from every AI without a profile of its own"
                else -> "No AI uses it yet: pick it on an AI's page"
            },
            RType.sans(13.5f),
            c.secondary,
            Modifier.padding(top = 4.dp, start = 32.dp, end = 32.dp),
            align = TextAlign.Center,
        )
    }
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Stat("${profile.memoryCount}", "remembered", Modifier.weight(1f).testTag("statMemory"))
        Stat("${profile.classes.count { it.autoApprove }} of ${profile.classes.size}", "on Auto", Modifier.weight(1f))
        Stat(profile.trainedAt?.let { relativeTime(it) } ?: "not yet", "trained", Modifier.weight(1f))
    }
}

@Composable
private fun Stat(value: String, label: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Column(
        modifier.clip(RoundedCornerShape(18.dp)).background(c.elevated).padding(vertical = 14.dp, horizontal = 10.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        RText(value, RType.sans(18f, FontWeight.SemiBold), c.text, maxLines = 1)
        RText(label, RType.sans(12.5f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 1)
    }
}

/**
 * One kind of request: a ring that fills with the user's answers toward unlocking (a tick once it runs by itself),
 * where it stands, and, opened, the lock.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ClassRow(cls: ClassView, onLock: (Boolean?) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var open by remember { mutableStateOf(false) }
    val locked = cls.manual == false
    val ringColor = when {
        locked -> c.tertiary
        cls.autoApprove -> c.success
        else -> c.accent
    }
    Column(Modifier.fillMaxWidth().testTag("class:${cls.classKey}")) {
        Row(
            Modifier
                .fillMaxWidth()
                .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp)) {
                    open = !open
                    feedback.play(Event.expand(open))
                }
                .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ProgressRing(if (locked) 0f else AutopilotText.unlockProgress(cls), ringColor, size = 52.dp, stroke = 5.dp) {
                when {
                    locked -> GlyphIcon(Glyph.Lock, c.tertiary, size = 18.dp)
                    cls.autoApprove -> GlyphIcon(Glyph.Check, c.success, size = 20.dp, weight = 2.4f)
                    else -> RText("${cls.decisions}", RType.mono(14f, FontWeight.SemiBold), c.text, maxLines = 1)
                }
            }
            Spacer(Modifier.width(16.dp))
            Column(Modifier.weight(1f)) {
                RText(cls.label, RType.sans(15.5f, FontWeight.SemiBold), c.text, maxLines = 1)
                RText(
                    AutopilotText.classStatus(cls),
                    RType.sans(13f, FontWeight.Medium),
                    when {
                        locked -> c.secondary
                        cls.autoApprove -> c.success
                        else -> c.accent
                    },
                    Modifier.padding(top = 2.dp).testTag("classStatus:${cls.classKey}"),
                    maxLines = 2,
                )
                RText(AutopilotText.classNumbers(cls), RType.sans(12.5f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 1)
            }
            if (cls.autoDeny) {
                Spacer(Modifier.width(8.dp))
                Tag("Denies", tint = c.danger)
            }
        }
        AnimatedVisibility(
            open,
            enter = expandVertically(spring(dampingRatio = 0.9f, stiffness = 500f)) + fadeIn(),
            exit = shrinkVertically(spring(dampingRatio = 0.9f, stiffness = 500f)) + fadeOut(),
        ) {
            FlowRow(
                Modifier.padding(start = 84.dp, end = 16.dp, bottom = 14.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                SelectChip("Learn by itself", cls.manual == null, Modifier.testTag("lock:auto:${cls.classKey}")) { onLock(null) }
                SelectChip("Always ask", cls.manual == false, Modifier.testTag("lock:on:${cls.classKey}")) { onLock(true) }
                SelectChip("Unlock now", cls.manual == true, Modifier.testTag("lock:off:${cls.classKey}")) { onLock(false) }
            }
        }
    }
}
