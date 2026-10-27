package dev.reins.android.ui.circles

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Group
import androidx.compose.material.icons.rounded.Handshake
import androidx.compose.material.icons.rounded.PersonAdd
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.AppContainer
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.NextPill
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SignalButton
import dev.reins.android.design.SignalDialog
import dev.reins.android.design.pressable
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.permissions.PrimaryPhoneSetupSection
import dev.reins.android.ui.permissions.PrimarySetupWords
import dev.reins.core.CircleLinkState
import dev.reins.core.CirclePerson
import dev.reins.core.CircleRole

/** A role's Material Symbol. */
fun roleIcon(role: CircleRole): ImageVector = when (role) {
    CircleRole.DELEGATE -> Icons.Rounded.Handshake
    CircleRole.COSIGN -> Icons.Rounded.Group
    CircleRole.GUARDIAN -> Icons.Rounded.Shield
    CircleRole.WATCHER -> Icons.Rounded.Visibility
}

/**
 * Settings › Circles (Decisions v2, 04 §6.1): the people who help this account, and those it helps, with Add someone
 * and Join on top. The owner pauses and resumes an arrangement; either side ends it (a helper leaves), behind a
 * question. On a phone that can't take part (not the account's primary phone) a line and the primary-phone setup.
 */
@Composable
fun CirclesScreen(
    viewModel: CirclesViewModel,
    container: AppContainer,
    authenticator: Authenticator,
    onBack: () -> Unit,
) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val add by viewModel.add.collectAsStateWithLifecycle()
    val join by viewModel.join.collectAsStateWithLifecycle()
    val revision by container.state.circleRevision.collectAsStateWithLifecycle()
    val connections by container.state.connections.collectAsStateWithLifecycle()
    var ending by remember { mutableStateOf<CirclePerson?>(null) }
    LaunchedEffect(viewModel) { viewModel.load(fromServer = true) }
    // The core said something changed (a person joined, left, was paused): read them again, quietly.
    LaunchedEffect(revision) { if (revision > 0) viewModel.load(fromServer = false) }

    Screen(title = "Circles", onBack = onBack) {
        when (ui.available) {
            null -> RText("Checking…", RType.sans(15f), c.ink2, Modifier.padding(horizontal = 20.dp, vertical = 12.dp))
            false -> Unavailable(viewModel, authenticator)
            true -> {
                Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    SignalButton(
                        "Add someone",
                        Modifier.weight(1f).testTag("circleAdd"),
                        size = ButtonSize.Small,
                        leading = { Icon(Icons.Rounded.PersonAdd, null, Modifier.size(18.dp), tint = c.onInverse) },
                        onClick = viewModel::openAdd,
                    )
                    SignalButton(
                        "Join",
                        Modifier.weight(1f).testTag("circleJoin"),
                        style = ButtonStyle.Soft,
                        size = ButtonSize.Small,
                        leading = { Icon(Icons.Rounded.Handshake, null, Modifier.size(18.dp), tint = c.ink) },
                        onClick = { viewModel.openJoin() },
                    )
                }
                ui.error?.let { Banner(it, Modifier.padding(horizontal = 16.dp, vertical = 6.dp), BannerKind.Error, tag = "circlesError") }
                People(ui, onPause = viewModel::pause, onEnd = { ending = it })
            }
        }
        Spacer(Modifier.padding(bottom = 32.dp))
    }

    ending?.let { person ->
        val leave = !person.mine
        SignalDialog(
            title = if (leave) "Leave ${shortName(untrusted(person.peer))}'s Circle?" else "End with ${shortName(untrusted(person.peer))}?",
            text = if (leave) "You stop deciding for them." else "They stop deciding with you.",
            confirmLabel = endLabel(person),
            onConfirm = {
                viewModel.end(person)
                ending = null
            },
            onDismiss = { ending = null },
            destructive = true,
            confirmTag = "circleEndConfirm",
            glyph = dev.reins.android.design.Glyph.People,
        )
    }
    add?.let { flow ->
        AddCircleSheet(
            flow,
            connections,
            onRole = viewModel::pickRole,
            onPreset = viewModel::pickPreset,
            onAgent = viewModel::pickAgent,
            onDeferred = viewModel::setDeferred,
            onCreate = viewModel::createInvite,
            onConfirm = { viewModel.confirmInvite(authenticator) },
            onBack = { if (!viewModel.addBack()) viewModel.closeAdd() },
            onClose = viewModel::closeAdd,
        )
    }
    join?.let { flow ->
        JoinCircleSheet(
            flow,
            onLink = viewModel::setJoinLink,
            onOpen = viewModel::openLink,
            onAnswer = viewModel::answerJoin,
            onClose = viewModel::closeJoin,
        )
    }
}

/** The people, by side; a short line when there is no one yet. */
@Composable
private fun People(ui: CirclesUi, onPause: (CirclePerson, Boolean) -> Unit, onEnd: (CirclePerson) -> Unit) {
    val c = LocalColors.current
    val people = ui.people
    if (people == null) {
        RText("Loading…", RType.sans(15f), c.ink2, Modifier.padding(horizontal = 20.dp, vertical = 12.dp))
        return
    }
    val groups = remember(people) { groupPeople(people) }
    if (groups.empty) {
        Group(footer = "People with their own Reins who approve with you, for you, or are told. Add someone, or join theirs.") {
            RText("No one yet.", RType.sans(15f), c.ink2, Modifier.padding(vertical = 14.dp).testTag("circlesEmpty"))
        }
        return
    }
    if (groups.helpingYou.isNotEmpty()) {
        Group(header = "Helping you", footer = "They decide with you, for you, or are told, within what each covers. Pause stops sending them anything.") {
            groups.helpingYou.forEachIndexed { i, person ->
                if (i > 0) Hairline(inset = 44.dp)
                PersonRow(person, busy = ui.busy == person.linkId, onPause = onPause, onEnd = onEnd)
            }
        }
    }
    if (groups.youHelp.isNotEmpty()) {
        Group(header = "You help", footer = "Their requests wait under For others in Activity. You can leave at any time.") {
            groups.youHelp.forEachIndexed { i, person ->
                if (i > 0) Hairline(inset = 44.dp)
                PersonRow(person, busy = ui.busy == person.linkId, onPause = onPause, onEnd = onEnd)
            }
        }
    }
}

/** One arrangement: who, their role and scope, its state, then Pause or Resume (owner) and End or Leave. */
@Composable
private fun PersonRow(person: CirclePerson, busy: Boolean, onPause: (CirclePerson, Boolean) -> Unit, onEnd: (CirclePerson) -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().padding(vertical = 10.dp).testTag("circlePerson:${person.linkId}"),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BlobAvatar(person.peer, size = 32.dp)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            RText(untrusted(person.peer), RType.row, c.ink, maxLines = 1)
            Row(horizontalArrangement = Arrangement.spacedBy(5.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(roleIcon(person.role), null, Modifier.size(14.dp), tint = c.inkMeta)
                RText(untrusted(personLine(person)), RType.meta, c.inkMeta, Modifier.testTag("circleLine:${person.linkId}"), maxLines = 2)
            }
            stateBadge(person.state)?.let { badge ->
                NextPill(
                    badge,
                    if (person.state == CircleLinkState.SUSPENDED) c.danger else c.warning,
                    Modifier.padding(top = 4.dp),
                    textTag = "circleState:${person.linkId}",
                )
            }
        }
        if (canPause(person)) {
            val paused = person.state == CircleLinkState.PAUSED
            TextAction(if (paused) "Resume" else "Pause", c.ink, "circlePause:${person.linkId}", enabled = !busy) { onPause(person, !paused) }
        }
        TextAction(endLabel(person), c.danger, "circleEnd:${person.linkId}", enabled = !busy) { onEnd(person) }
    }
}

/** A word that acts, in its colour (Next: "Sign out" on a row). */
@Composable
private fun TextAction(text: String, color: androidx.compose.ui.graphics.Color, tag: String, enabled: Boolean, onClick: () -> Unit) {
    RText(
        text,
        RType.sans(15f, FontWeight.SemiBold),
        if (enabled) color else color.copy(alpha = 0.4f),
        Modifier
            .clip(CircleShape)
            .pressable(enabled = enabled, shape = CircleShape, label = text, onClick = onClick)
            .padding(horizontal = 6.dp, vertical = 8.dp)
            .testTag(tag),
        maxLines = 1,
    )
}

/** This phone can't take part: a line, and the primary-phone setup. */
@Composable
private fun Unavailable(viewModel: CirclesViewModel, authenticator: Authenticator) {
    val c = LocalColors.current
    val setup = remember(viewModel) { viewModel.primaryPhoneSetup() }
    RText(
        "Circles run on your primary phone.",
        RType.sans(15f),
        c.ink2,
        Modifier.padding(horizontal = 20.dp, vertical = 10.dp).testTag("circlesUnavailable"),
    )
    PrimaryPhoneSetupSection(
        setup,
        authenticator,
        PrimarySetupWords(
            header = "Primary phone",
            needed = "Make this your primary phone to use Circles.",
            open = "Make this my primary phone",
            title = "Primary phone",
            reload = "Check again",
            already = "This phone is the primary.",
            elsewhere = "for Circles",
        ),
        onReload = { viewModel.load(fromServer = true) },
    )
}
